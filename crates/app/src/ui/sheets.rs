//! Modal sheet plumbing: opening sheets, slider wiring and the live preview
//! that stands in for the active layer's pixels. The sheet UI itself lives in
//! `sheets_ui.rs`. The floating panels' interactions moved onto their view
//! entities (ticket 41); what stays here are the shell-side preview
//! recomputes those entities reach through their seam, plus the modal
//! sheets' own widgets.

use gpui_kit::component::input::InputState;
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::{AppContext, Context, Entity, Window};

use compositor_core::adjust::{LevelsSettings, hue_saturation_apply, levels_apply_settings};
use compositor_core::effects::LayerEffectKind;
use compositor_core::geometry::CGAffineTransform;
use compositor_core::raster::Bitmap;
use compositor_core::transform::LayerTransform;

use crate::state::{ProjectWorkspaceView, Sheet, SheetState};

impl ProjectWorkspaceView {
    // MARK: Opening

    pub fn open_sheet(&mut self, kind: Sheet, window: &mut Window, cx: &mut Context<Self>) {
        let sheet = self.build_sheet(kind, window, cx);
        self.sheet = Some(sheet);
        if kind == Sheet::ExportJpeg {
            self.refresh_jpeg_preview(cx);
        }
        self.touch(cx);
    }

    fn make_input(
        &self,
        default: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        cx.new(|cx| InputState::new(window, cx).default_value(default))
    }

    /// Creates the JPEG export sheet's quality slider. Its changes refresh
    /// the live encoded preview — `apply_slider`'s JpegQuality branch, which
    /// stayed behind when the panel sliders moved onto their view entities
    /// (ticket 41), tail included: the epoch and the repaint followed every
    /// event in the old dispatcher, guards or not.
    fn make_slider(
        &mut self,
        min: f32,
        max: f32,
        value: f32,
        cx: &mut Context<Self>,
    ) -> Entity<SliderState> {
        let state = cx.new(|_| {
            SliderState::new()
                .min(min)
                .max(max)
                .step(if max - min > 12.0 { 1.0 } else { 0.1 })
                .default_value(value)
        });
        let subscription = cx.subscribe(&state, |this, _slider, event: &SliderEvent, cx| {
            let _value = match event {
                SliderEvent::Change(value) | SliderEvent::Release(value) => value.start(),
            };
            this.refresh_jpeg_preview(cx);
            this.touch(cx);
        });
        self.subscriptions.push(subscription);
        state
    }

    /// Builds the modal sheet's widget bundle. The tool dialogs that became
    /// floating panels (`ui/floating_panel.rs`) no longer build sheets here; what
    /// remains are the modal dialogs of the two-phase flows.
    fn build_sheet(
        &mut self,
        kind: Sheet,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> SheetState {
        let (doc_w, doc_h) = {
            let size = self.doc_size(cx);
            (size.0 as usize, size.1 as usize)
        };
        let base = SheetState {
            kind,
            width: self.make_input(&doc_w.to_string(), window, cx),
            height: self.make_input(&doc_h.to_string(), window, cx),
            path: self.make_input("export.png", window, cx),
            resolution: self.make_input("72", window, cx),
            tolerance: self.make_input("2", window, cx),
            percent: false,
            link: true,
            relative: false,
            anchor: 4,
            preset: 0,
            sliders: Vec::new(),
            based_on: 0,
            sides: [true, true, true, true],
            jpeg_preview: None,
            jpeg_ready: None,
            jpeg_data: None,
            jpeg_size_label: String::new(),
            message: String::new(),
            confirm_step: None,
        };
        match kind {
            Sheet::ImageSize => {
                let resolution = self
                    .current_tab(cx)
                    .session
                    .read(cx)
                    .document
                    .as_ref()
                    .map(|d| d.resolution as usize)
                    .unwrap_or(72)
                    .to_string();
                SheetState {
                    resolution: self.make_input(&resolution, window, cx),
                    link: true,
                    ..base
                }
            }
            Sheet::CanvasSize => base,
            Sheet::ExportPng => SheetState {
                path: self.make_input("export.png", window, cx),
                ..base
            },
            Sheet::ExportJpeg => SheetState {
                path: self.make_input("export.jpg", window, cx),
                // The quality of the last export, which the next one starts
                // from (JPEGExportSheet.init's UserDefaults read).
                sliders: vec![self.make_slider(
                    0.0,
                    1.0,
                    crate::settings::lock_global().number("jpegExportQuality", 0.85) as f32,
                    cx,
                )],
                // 0 = white matte, 1 = black.
                preset: 0,
                ..base
            },
            Sheet::SaveComp => SheetState {
                path: self.make_input("./project.comp", window, cx),
                ..base
            },
            Sheet::Trim | Sheet::About | Sheet::Confirm | Sheet::Alert => base,
        }
    }

    /// Captures the active layer's pixels so adjustment panels can preview.
    pub(crate) fn begin_preview(&mut self, cx: &gpui_kit::App) {
        let layer = self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .active_layer()
            .cloned();
        self.preview_base = layer
            .as_ref()
            .and_then(|layer| layer.asset.as_ref().map(|asset| asset.image.clone()));
        self.preview_base_transform = layer.as_ref().map(|layer| layer.transform);
        self.preview = self.preview_base.clone();
    }

    /// Reads one of the current sheet's slider values.
    pub(crate) fn sheet_value(&self, cx: &gpui_kit::App, index: usize) -> f32 {
        self.sheet
            .as_ref()
            .and_then(|sheet| sheet.sliders.get(index))
            // Single-value sliders keep the live value in `end`; `start` is always 0.
            .map(|slider| slider.read(cx).value().end())
            .unwrap_or(0.0)
    }

    // MARK: The panels' preview recomputes (reached through the seam)

    /// Writes the Levels settings into the live preview — `apply_slider`'s
    /// Levels branch, hosted here so the panel entity reaches it through its
    /// seam (ticket 41). An adjustment-layer editing session previews
    /// through the session and — exactly as the slider dispatcher's early
    /// return did — leaves before the repaint epoch; a pixel session
    /// recomputes the preview bitmap, then the epoch moves.
    pub(crate) fn apply_levels_preview(
        &mut self,
        settings: LevelsSettings,
        preview: bool,
        cx: &mut Context<Self>,
    ) {
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .adjustment_editing_id
            .is_some()
        {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut().session.update(cx, |session, _| {
                    session.levels = Some(settings);
                    session.preview_adjustment_editing(preview);
                })
            });
            return;
        }
        if let Some(base) = self.preview_base.clone() {
            if preview {
                let mut adjusted = base.as_ref().clone();
                levels_apply_settings(&mut adjusted, &settings);
                adjusted = self.blend_adjusted(adjusted, cx);
                self.preview = Some(std::sync::Arc::new(adjusted));
            } else {
                self.preview = Some(base);
            }
        }
        self.touch(cx);
    }

    /// Rebuilds the Levels preview after the Preview checkbox flips: unchecked
    /// shows the untouched pixels, exactly as Swift's `edit.preview` does.
    /// Unlike a slider change ([`Self::apply_levels_preview`]), the
    /// adjustment path repaints too — the old `refresh_levels_preview`
    /// bumped the epoch on both branches.
    pub(crate) fn refresh_levels_preview(
        &mut self,
        settings: LevelsSettings,
        preview: bool,
        cx: &mut Context<Self>,
    ) {
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .adjustment_editing_id
            .is_some()
        {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut().session.update(cx, |session, _| {
                    session.levels = Some(settings);
                    session.preview_adjustment_editing(preview);
                })
            });
            self.touch(cx);
            return;
        }
        let Some(base) = self.preview_base.clone() else {
            return;
        };
        if preview {
            let mut adjusted = base.as_ref().clone();
            levels_apply_settings(&mut adjusted, &settings);
            adjusted = self.blend_adjusted(adjusted, cx);
            self.preview = Some(std::sync::Arc::new(adjusted));
        } else {
            self.preview = Some(base);
        }
        self.touch(cx);
    }

    /// Writes the Hue/Saturation settings into the live preview —
    /// `apply_slider`'s Hue branch, hosted for the panel entity (ticket 41).
    /// The adjustment path returns before the epoch, as the dispatcher did.
    pub(crate) fn apply_hue_saturation_preview(
        &mut self,
        settings: compositor_core::adjust::HueSaturationSettings,
        cx: &mut Context<Self>,
    ) {
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .adjustment_editing_id
            .is_some()
        {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut().session.update(cx, |session, _| {
                    session.hue_saturation = Some(settings);
                    session.preview_adjustment_editing(true);
                })
            });
            return;
        }
        if let Some(base) = self.preview_base.clone() {
            let mut adjusted = base.as_ref().clone();
            hue_saturation_apply(&mut adjusted, &settings);
            adjusted = self.blend_adjusted(adjusted, cx);
            self.preview = Some(std::sync::Arc::new(adjusted));
        }
        self.touch(cx);
    }

    // MARK: Effects panel

    /// The layers-panel effects menu: adds the effect (which selects and
    /// starts editing it) and opens the Effects panel for it. No panel when
    /// the add was refused (no editable pixel layer active).
    pub fn add_effect(
        &mut self,
        kind: LayerEffectKind,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.add_effect(kind))
        });
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .effects_editing
            .map(|editing| editing.kind)
            == Some(kind)
        {
            self.open_effects_panel(cx);
        }
        self.touch(cx);
    }

    /// Blends an adjustment result back over the untouched base through the
    /// selection, as `LevelsFilter.run`/`HueSaturationFilter.run` do before
    /// the pixels reach the layer (Levels.swift:80-82, HueSaturation.swift:
    /// 244-250). Without a selection the adjustment stands alone.
    fn blend_adjusted(&self, adjusted: Bitmap, cx: &gpui_kit::App) -> Bitmap {
        let Some(base) = self.preview_base.as_deref() else {
            return adjusted;
        };
        blend_through_selection(
            base,
            adjusted,
            self.preview_base_transform,
            self.current_tab(cx).session.read(cx).selection().cloned(),
        )
    }
}

/// Reads one of a panel's slider values (the panel entities read their own
/// widgets with this since ticket 41).
pub(crate) fn panel_value(
    sliders: &[Entity<SliderState>],
    cx: &gpui_kit::App,
    index: usize,
) -> f32 {
    sliders
        .get(index)
        // Single-value sliders keep the live value in `end`; `start` is always 0.
        .map(|slider| slider.read(cx).value().end())
        .unwrap_or(0.0)
}

/// The placement transform `adjust::pixel::coverage` samples by: every pixel
/// center is mapped through it into the document before reading the
/// selection (`PixelAdjust.coverage`'s `concatenate(pixelToDocument.inverted())`,
/// Levels.swift:90-96).
fn coverage_mapping(
    transform: Option<LayerTransform>,
    width: usize,
    height: usize,
) -> CGAffineTransform {
    transform
        .map(|t| compositor_core::raster::pixel_to_document(&t, width as f64, height as f64))
        .unwrap_or(CGAffineTransform::IDENTITY)
}

/// `PixelAdjust.blend(over:through:)`: coverage × adjusted + (1 − coverage) ×
/// original, with the selection rasterized on the layer's pixel grid through
/// its placement. No selection keeps the adjustment whole.
fn blend_through_selection(
    base: &Bitmap,
    adjusted: Bitmap,
    transform: Option<LayerTransform>,
    selection: Option<compositor_core::selection::DocumentSelection>,
) -> Bitmap {
    let Some(selection) = selection else {
        return adjusted;
    };
    let mapping = coverage_mapping(transform, base.width, base.height);
    compositor_core::adjust::pixel::blend_path(&adjusted, base, &selection, &mapping)
}

/// The Levels histogram's per-pixel weights: the selection's coverage on the
/// layer's own pixel grid (`PixelAdjust.coverage(...)` in
/// `LevelsFilter.histogram`, Levels.swift:90-96).
pub(crate) fn histogram_coverage(
    transform: Option<LayerTransform>,
    selection: Option<compositor_core::selection::DocumentSelection>,
    width: usize,
    height: usize,
) -> Option<compositor_core::raster::GrayMask> {
    let selection = selection?;
    let mapping = coverage_mapping(transform, width, height);
    Some(compositor_core::adjust::pixel::coverage_path(
        &selection, width, height, &mapping,
    ))
}

#[cfg(test)]
mod tests {
    use super::{blend_through_selection, histogram_coverage};
    use compositor_core::filters::FilterSettings;
    use compositor_core::geometry::{CGPoint, CGRect, CGSize};
    use compositor_core::raster::Bitmap;
    use compositor_core::selection::{DocumentSelection, SelectionPath};
    use compositor_core::transform::LayerTransform;

    /// A selection covering document x 2…6 in a 10×1 canvas.
    fn strip_selection() -> compositor_core::selection::DocumentSelection {
        DocumentSelection::new(SelectionPath::rect(CGRect::new(2.0, 0.0, 4.0, 1.0)))
    }

    fn solid(width: usize, value: [u8; 4]) -> Bitmap {
        let mut bitmap = Bitmap::new(width, 1);
        for x in 0..width {
            bitmap.set_pixel(x, 0, value);
        }
        bitmap
    }

    #[test]
    fn blend_through_selection_limits_the_adjustment() {
        let base = solid(10, [0, 0, 0, 255]);
        let adjusted = solid(10, [255, 255, 255, 255]);
        let blended = blend_through_selection(&base, adjusted, None, Some(strip_selection()));
        // Fully inside the selection the adjustment wins; outside — right up
        // against the boundary — the base.
        for x in 2..6 {
            assert_eq!(blended.pixel(x, 0), [255, 255, 255, 255], "column {x}");
        }
        for x in [0, 1, 6, 8] {
            assert_eq!(blended.pixel(x, 0), [0, 0, 0, 255], "column {x}");
        }
    }

    #[test]
    fn blend_through_selection_follows_the_layer_placement() {
        let base = solid(10, [0, 0, 0, 255]);
        let adjusted = solid(10, [255, 255, 255, 255]);
        // Document x 2.5…6.5 over a layer shifted two pixels right: the
        // layer's pixels sitting under the selection adjust, and each rim
        // pixel blends at exactly half coverage.
        let transform = LayerTransform::new(CGPoint::new(2.0, 0.0), CGSize::new(10.0, 1.0));
        let selection =
            DocumentSelection::new(SelectionPath::rect(CGRect::new(2.5, 0.0, 4.0, 1.0)));
        let blended = blend_through_selection(&base, adjusted, Some(transform), Some(selection));
        assert_eq!(blended.pixel(0, 0), [128, 128, 128, 255]);
        assert_eq!(blended.pixel(2, 0), [255, 255, 255, 255]);
        assert_eq!(blended.pixel(3, 0), [255, 255, 255, 255]);
        assert_eq!(blended.pixel(4, 0), [128, 128, 128, 255]);
        assert_eq!(blended.pixel(5, 0), [0, 0, 0, 255]);
        assert_eq!(blended.pixel(8, 0), [0, 0, 0, 255]);
    }

    #[test]
    fn blend_without_selection_keeps_the_adjustment_whole() {
        let base = solid(4, [0, 0, 0, 255]);
        let adjusted = solid(4, [255, 255, 255, 255]);
        let blended = blend_through_selection(&base, adjusted.clone(), None, None);
        for x in 0..4 {
            assert_eq!(blended.pixel(x, 0), [255, 255, 255, 255]);
        }
    }

    #[test]
    fn histogram_coverage_places_the_selection_on_the_layer_grid() {
        assert!(histogram_coverage(None, None, 10, 1).is_none());
        let coverage = histogram_coverage(None, Some(strip_selection()), 10, 1).unwrap();
        assert_eq!(coverage.value(3, 0), 255);
        assert_eq!(coverage.value(8, 0), 0);
        // A layer shifted two pixels right reads the selection from the grid
        // positions that actually sit under it.
        let transform = LayerTransform::new(CGPoint::new(2.0, 0.0), CGSize::new(10.0, 1.0));
        let shifted = histogram_coverage(Some(transform), Some(strip_selection()), 10, 1).unwrap();
        assert_eq!(shifted.value(1, 0), 255);
        assert_eq!(shifted.value(4, 0), 0);
        assert_eq!(shifted.value(6, 0), 0);
    }

    #[test]
    fn filter_slider_defaults_match_filters_swift() {
        let defaults = FilterSettings::default();
        assert_eq!(defaults.radius, 1.0);
        assert_eq!(defaults.distance, 10.0);
        assert_eq!(defaults.amount, 10.0);
    }
}
