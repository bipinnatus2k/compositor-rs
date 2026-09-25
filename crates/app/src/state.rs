//! Application state: the `EditorSession` plus the viewport, the active
//! canvas drag, the modal sheet and the rendered-frame cache. Owned by the
//! window's root view (see `workspace.rs`).

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::{
    px, App, AppContext, Bounds, Context, Entity, FocusHandle, KeyDownEvent,
    KeyUpEvent, Pixels, Point, RenderImage, Subscription, Window,
};
use gpui_kit::component::input::InputState;
use gpui_kit::component::WindowExt;
use gpui_kit::component::slider::SliderState;
use gpui_kit::component::slider::SliderEvent;
use image::Frame;

use compositor_core::document::CanvasDocument;
use compositor_core::document::ImageLayer;
use compositor_core::filters::{FilterJob, FilterKind, FilterSettings};
use compositor_core::geometry::{CGAffineTransform, CGPoint, CGRect};
use compositor_core::tools::warp::WarpStroke;
use compositor_core::raster::{ImportedImage, Bitmap};
use compositor_core::render;
use compositor_core::session::{EditorSession, SelectionAmountOperation};
use compositor_core::transform::LayerTransform;
use compositor_core::tools::brush::RasterEdit;

pub const RULER_SIZE: f32 = 20.0;
pub const MIN_ZOOM: f64 = 0.008;
pub const MAX_ZOOM: f64 = 32.0;

/// Document coordinates to viewport pixels under the given zoom and pan.
/// Pure so tests can cover it without a window.
pub fn doc_to_screen_at(zoom: f64, pan: (f64, f64), x: f64, y: f64) -> (f32, f32) {
    ((x * zoom + pan.0) as f32, (y * zoom + pan.1) as f32)
}

/// Viewport pixels back to document coordinates; the inverse of
/// `doc_to_screen_at`.
pub fn screen_to_doc_at(zoom: f64, pan: (f64, f64), x: f32, y: f32) -> CGPoint {
    CGPoint::new(
        (x as f64 - pan.0) / zoom,
        (y as f64 - pan.1) / zoom,
    )
}

/// Clamps a zoom factor to the editor's supported range.
pub fn clamp_zoom(zoom: f64) -> f64 {
    zoom.clamp(MIN_ZOOM, MAX_ZOOM)
}

/// What the pointer is currently doing on the canvas.
pub enum CanvasDrag {
    /// Hand tool or middle-mouse panning.
    Pan { start: Point<Pixels>, start_pan: (f64, f64) },
    /// A brush-family stroke in progress.
    Stroke(RasterEdit),
    /// Gradient drag: the edit plus the document-space start point.
    Gradient(RasterEdit, CGPoint),
    /// Marquee / freehand lasso drag; the draft lives in the session.
    MarqueeLasso,
    /// A Smudge or Liquify stroke working on a document-size copy.
    Warp(WarpStroke),
    /// The Shape tool dragging a shape out; the draft rect grows live.
    Shape { start: CGPoint },
    /// Move tool drag.
    MoveLayer {
        start_origin: compositor_core::transform::LayerTransform,
        start: CGPoint,
    },
    /// Crop drag; the drag lives in the session.
    Crop,
}

/// Which modal sheet is open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SheetKind {
    NewCanvas,
    ImageSize,
    CanvasSize,
    Levels,
    HueSaturation,
    Effects,
    Filter(FilterKind),
    Amount(SelectionAmountOperation),
    ExportPng,
    ExportJpeg,
    Import,
    SaveComp,
    Open,
    Trim,
    About,
    Shortcuts,
}

/// The modal sheet: which one plus all of its widget states. Fields a given
/// sheet doesn't use are simply never read.
pub struct SheetState {
    pub kind: SheetKind,
    pub name: Entity<InputState>,
    pub width: Entity<InputState>,
    pub height: Entity<InputState>,
    pub path: Entity<InputState>,
    pub resolution: Entity<InputState>,
    pub tolerance: Entity<InputState>,
    pub percent: bool,
    pub link: bool,
    pub relative: bool,
    pub anchor: usize,
    pub background: usize,
    pub preset: usize,
    pub channel: usize,
    pub sliders: Vec<Entity<SliderState>>,
    pub checkbox_a: bool,
    pub checkbox_b: bool,
    pub colorize: bool,
    pub based_on: usize,
    pub sides: [bool; 4],
    /// Histogram of the adjusted pixels (Levels).
    pub histogram: [f64; 1024],
    /// The full Levels settings behind the sliders: Auto replaces every
    /// channel's range at once, while the sliders only ever edit the visible
    /// channel's range on top of this.
    pub levels: compositor_core::adjust::LevelsSettings,
}

/// Everything the window shows, owned as a single GPUI entity.
pub struct AppState {
    pub session: EditorSession,
    pub focus: FocusHandle,
    /// Screen pixels per document pixel.
    pub zoom: f64,
    /// Document (0, 0) in viewport pixels.
    pub pan: (f64, f64),
    /// Bumped on every session/viewport mutation; invalidates the frame image.
    pub revision: u64,
    frame_image: Option<Arc<RenderImage>>,
    frame_image_rev: u64,
    /// The canvas paint area, updated by every frame.
    pub viewport: Bounds<Pixels>,
    pub drag: Option<CanvasDrag>,
    pub sheet: Option<SheetState>,
    pub status: Option<String>,
    pub rename: Option<uuid::Uuid>,
    pub rename_input: Option<Entity<InputState>>,
    thumbnails: HashMap<(uuid::Uuid, bool), (u64, Arc<RenderImage>)>,
    /// Document dims the viewport was last fit for.
    pub fit_for: (usize, usize),
    /// The active layer's pixels when an adjustment sheet opened, and the
    /// live preview result shown on canvas.
    pub preview_base: Option<Arc<Bitmap>>,
    /// The Camera Raw defringe eyedropper is armed: the next canvas click
    /// centers a defringe range on the clicked fringe color.
    pub camera_raw_samples_defringe: bool,
    /// RGB under the pointer while Camera Raw is open; cleared outside.
    pub camera_raw_readout: Option<[u8; 3]>,
    pub preview: Option<Arc<Bitmap>>,
    pub preview_base_transform: Option<LayerTransform>,
    /// The Shape tool's drag: the shape being placed and its current rect.
    pub shape_shape: Option<compositor_core::document::LayerShape>,
    pub shape_rect: Option<CGRect>,
    /// The gradient tool's drag end, in document coordinates.
    pub gradient_cursor: Option<CGPoint>,
    /// Tool option sliders (brush size/hardness/opacity/smoothing,
    /// wand tolerance, gradient opacity).
    pub brush_sliders: [Entity<SliderState>; 4],
    pub wand_slider: Entity<SliderState>,
    pub gradient_slider: Entity<SliderState>,
    /// The active layer's opacity (Layers panel header).
    pub opacity_slider: Entity<SliderState>,
    pub subscriptions: Vec<Subscription>,
    /// The tool saved while Space temporarily turns the canvas into the Hand
    /// tool (`spaceHeld` in EditorCanvas.swift).
    pub space_saved_tool: Option<compositor_core::document::NavigationTool>,
    /// The open color picker's widgets (`color_picker_ui`).
    pub picker_ui: Option<crate::color_picker_ui::PickerUi>,
    /// Monotonic epoch for the opacity-digit pair window (0.6 s).
    key_epoch: std::time::Instant,
}

/// Creates a slider with the given range and initial value.
fn slider(min: f32, max: f32, step: f32, value: f32) -> SliderState {
    SliderState::new()
        .min(min)
        .max(max)
        .step(step)
        .default_value(value)
}

impl AppState {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut session = EditorSession::new();
        session.create_document(1280, 800, true);
        let brush = &session.brush_settings;

        let brush_sliders = [
            cx.new(|_| slider(1.0, 300.0, 1.0, brush.diameter as f32)),
            cx.new(|_| slider(0.0, 100.0, 1.0, (brush.hardness * 100.0) as f32)),
            cx.new(|_| slider(1.0, 100.0, 1.0, (brush.opacity * 100.0) as f32)),
            cx.new(|_| slider(0.0, 100.0, 1.0, brush.smoothing as f32)),
        ];
        let wand_slider = cx.new(|_| {
            slider(0.0, 255.0, 1.0, session.wand_settings.tolerance as f32)
        });
        let gradient_slider = cx.new(|_| {
            slider(1.0, 100.0, 1.0, (session.gradient_settings.opacity * 100.0) as f32)
        });
        let opacity_slider = cx.new(|_| slider(0.0, 100.0, 1.0, 100.0));

        let mut state = AppState {
            session,
            focus: cx.focus_handle(),
            zoom: 1.0,
            pan: (0.0, 0.0),
            revision: 1,
            frame_image: None,
            frame_image_rev: 0,
            viewport: Bounds::new(Point::new(px(0.), px(0.)), gpui_kit::size(px(0.), px(0.))),
            drag: None,
            sheet: None,
            status: None,
            rename: None,
            rename_input: None,
            thumbnails: HashMap::new(),
            fit_for: (0, 0),
            preview_base: None,
            camera_raw_samples_defringe: false,
            camera_raw_readout: None,
            preview: None,
            preview_base_transform: None,
            gradient_cursor: None,
            shape_shape: None,
            shape_rect: None,
            brush_sliders,
            wand_slider,
            gradient_slider,
            opacity_slider,
            subscriptions: Vec::new(),
            space_saved_tool: None,
            picker_ui: None,
            key_epoch: std::time::Instant::now(),
        };
        state.subscribe_sliders(cx);
        state
    }

    /// Monotonic seconds since the app started (`systemUptime`).
    pub fn uptime(&self) -> f64 {
        self.key_epoch.elapsed().as_secs_f64()
    }

    /// Space keyDown: the canvas pans until the key comes back up
    /// (EditorCanvas.swift:1825).
    pub fn space_down(&mut self, cx: &mut Context<Self>) {
        if self.space_saved_tool.is_some()
            || self.session.document.is_none()
            || self.session.text_draft.is_some()
        {
            return;
        }
        self.space_saved_tool = Some(self.session.tool);
        self.session.tool = compositor_core::document::NavigationTool::Hand;
        self.touch(cx);
    }

    /// Space keyUp: the saved tool comes back (EditorCanvas.swift:1866).
    pub fn space_up(&mut self, cx: &mut Context<Self>) {
        if let Some(tool) = self.space_saved_tool.take() {
            self.session.tool = tool;
            self.touch(cx);
        }
    }

    /// Number keys set the active tool's opacity (Photoshop-style).
    pub fn opacity_digit(&mut self, digit: i32, window: &mut Window, cx: &mut Context<Self>) {
        self.session.type_opacity_digit(digit, self.uptime());
        // Keep the panel sliders truthful (Swift's bindings follow the same
        // settings).
        use compositor_core::document::NavigationTool;
        let sync: Option<(Entity<SliderState>, f32)> = match self.session.tool {
            NavigationTool::Brush
            | NavigationTool::SpotHealing
            | NavigationTool::CloneStamp
            | NavigationTool::Blur => Some((
                self.brush_sliders[2].clone(),
                (self.session.brush_settings.opacity * 100.0) as f32,
            )),
            NavigationTool::Gradient => Some((
                self.gradient_slider.clone(),
                (self.session.gradient_settings.opacity * 100.0) as f32,
            )),
            NavigationTool::Move => Some((
                self.opacity_slider.clone(),
                (self.session.active_layer().map(|l| l.opacity).unwrap_or(1.0) * 100.0) as f32,
            )),
            _ => None,
        };
        if let Some((slider, value)) = sync {
            slider.update(cx, |state, cx| {
                state.set_value(
                    gpui_kit::component::slider::SliderValue::Single(value),
                    window,
                    cx,
                )
            });
        }
        self.touch(cx);
    }

    /// Option-arrows step the edited text's tracking (InlineTextEditor.swift:18-19).
    pub fn adjust_text_tracking(&mut self, delta: f64, cx: &mut Context<Self>) {
        if self.session.text_draft.is_none() {
            return;
        }
        self.session.change_text_style(|style| style.tracking += delta);
        self.touch(cx);
    }

    /// Option-arrows step the edited text's leading, counting from whatever
    /// Auto works out to; up closes the lines up (InlineTextEditor.swift:20-22).
    pub fn adjust_text_leading(&mut self, down: bool, step: f64, cx: &mut Context<Self>) {
        if self.session.text_draft.is_none() {
            return;
        }
        self.session.change_text_style(|style| {
            style.leading = if down {
                style.line_height() + step
            } else {
                (style.line_height() - step).max(1.0)
            };
        });
        self.touch(cx);
    }

    /// ⌘Return finishes the text being edited (InlineTextEditor.swift:25-27).
    pub fn finish_text_edit(&mut self, cx: &mut Context<Self>) {
        if self.session.text_draft.is_some() {
            self.session.finish_text();
            self.touch(cx);
        }
    }

    /// The raw canvas keys: Space hold-to-pan, opacity digits, Option-arrow
    /// text spacing and ⌘Return (`EditorCanvas.keyDown`). Runs on the bubble
    /// phase; keys that mean something here don't reach the action system.
    pub fn canvas_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.canvas_keys_active(window, cx) {
            return;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = &event.keystroke.modifiers;
        let bare = !(modifiers.platform || modifiers.control || modifiers.alt || modifiers.shift);
        // ⌘Return finishes the text draft (⌘ maps to Ctrl on Windows).
        if modifiers.control && !modifiers.alt && key == "enter" {
            self.finish_text_edit(cx);
            cx.stop_propagation();
            return;
        }
        // Space holds the Hand tool; a text draft keeps typing into the box.
        if key == "space" && bare && self.session.text_draft.is_none() {
            self.space_down(cx);
            cx.stop_propagation();
            return;
        }
        // Option with the arrows sets the text's spacing: left and right the
        // tracking, up and down the leading; Shift makes each step ten.
        if modifiers.alt && !modifiers.control && !modifiers.platform {
            let step = if modifiers.shift { 10.0 } else { 1.0 };
            match key {
                "left" => {
                    self.adjust_text_tracking(-step, cx);
                    cx.stop_propagation();
                    return;
                }
                "right" => {
                    self.adjust_text_tracking(step, cx);
                    cx.stop_propagation();
                    return;
                }
                "up" => {
                    self.adjust_text_leading(false, step, cx);
                    cx.stop_propagation();
                    return;
                }
                "down" => {
                    self.adjust_text_leading(true, step, cx);
                    cx.stop_propagation();
                    return;
                }
                _ => {}
            }
        }
        // Number keys set opacity when the tool uses it (usesOpacityKeys).
        if bare {
            if let Ok(digit) = key.parse::<i32>() {
                if (0..=9).contains(&digit) {
                    self.opacity_digit(digit, window, cx);
                    cx.stop_propagation();
                }
            }
        }
    }

    /// The raw canvas key releases: Space gives the saved tool back.
    pub fn canvas_key_up(
        &mut self,
        event: &KeyUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "space" {
            self.space_up(cx);
        }
    }

    /// Keeps the session's tool settings in sync with the sliders.
    fn subscribe_sliders(&mut self, cx: &mut Context<Self>) {
        let brush_indices = [0usize, 1, 2, 3];
        for index in brush_indices {
            let slider = self.brush_sliders[index].clone();
            let subscription = cx.subscribe(&slider, move |state, _slider, event: &SliderEvent, cx| {
                let value = match event {
                    SliderEvent::Change(value) | SliderEvent::Release(value) => value.start(),
                };
                let brush = &mut state.session.brush_settings;
                match index {
                    0 => brush.diameter = value as f64,
                    1 => brush.hardness = (value / 100.0) as f64,
                    2 => brush.opacity = (value / 100.0) as f64,
                    _ => brush.smoothing = value as f64,
                }
                state.revision += 1;
                cx.notify();
            });
            self.subscriptions.push(subscription);
        }
        let wand_slider = self.wand_slider.clone();
        let subscription = cx.subscribe(&wand_slider, |state, _slider, event: &SliderEvent, cx| {
            let value = match event {
                SliderEvent::Change(value) | SliderEvent::Release(value) => value.start(),
            };
            {
                state.session.wand_settings.tolerance = value as i32;
                state.revision += 1;
                cx.notify();
            }
        });
        self.subscriptions.push(subscription);
        let gradient_slider = self.gradient_slider.clone();
        let subscription = cx.subscribe(&gradient_slider, |state, _slider, event: &SliderEvent, cx| {
            let value = match event {
                SliderEvent::Change(value) | SliderEvent::Release(value) => value.start(),
            };
            state.session.gradient_settings.opacity = (value / 100.0) as f64;
            state.revision += 1;
            cx.notify();
        });
        self.subscriptions.push(subscription);
        let opacity_slider = self.opacity_slider.clone();
        let subscription = cx.subscribe(&opacity_slider, |state, _slider, event: &SliderEvent, cx| {
            match event {
                SliderEvent::Change(value) => {
                    state.session.begin_opacity_edit();
                    state.session.set_layer_opacity((value.start() / 100.0) as f64);
                }
                SliderEvent::Release(value) => {
                    state.session.set_layer_opacity((value.start() / 100.0) as f64);
                    state.session.finish_opacity_edit();
                }
            }
            state.revision += 1;
            cx.notify();
        });
        self.subscriptions.push(subscription);
    }

    /// `[` / `]` shrink and grow the brush, keeping the slider in sync.
    pub fn resize_brush(&mut self, factor: f64, window: &mut Window, cx: &mut Context<Self>) {
        let next = (self.session.brush_settings.diameter * factor).clamp(1.0, 300.0);
        self.session.brush_settings.diameter = next;
        let slider = self.brush_sliders[0].clone();
        slider.update(cx, |state, cx| {
            state.set_value(
                gpui_kit::component::slider::SliderValue::Single(next as f32),
                window,
                cx,
            )
        });
        self.set_status(format!("Brush size {:.0} px", next), cx);
    }

    /// Shift-[ / Shift-]: hardness in Photoshop's 25% steps (0, 25, 50, 75, 100%).
    pub fn change_brush_hardness(&mut self, increase: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.session.change_brush_hardness(increase);
        let next = self.session.brush_settings.hardness;
        let slider = self.brush_sliders[1].clone();
        slider.update(cx, |state, cx| {
            state.set_value(
                gpui_kit::component::slider::SliderValue::Single((next * 100.0) as f32),
                window,
                cx,
            )
        });
        self.set_status(format!("Brush hardness {:.0}%", next * 100.0), cx);
    }

    /// Content-Aware Fill (Shift-Delete): heals the selection from surrounding
    /// pixels and commits it as one undo step, without opening a sheet.
    pub fn run_content_aware_fill(&mut self, cx: &mut Context<Self>) {
        let Some(base) = self
            .session
            .active_layer()
            .and_then(|layer| layer.asset.as_ref().map(|asset| asset.image.clone()))
        else {
            return;
        };
        let selection = self.selection_clip();
        if self.session.selection().is_none() {
            return;
        }
        let job = FilterJob {
            kind: FilterKind::ContentAwareFill,
            image: &base,
            settings: FilterSettings::default(),
            scale: 1.0,
            selection,
            mapping: CGAffineTransform::IDENTITY,
            seed: 7,
            noise_origin: CGPoint::ZERO,
            camera_raw_clipping: None,
            visualize_point_color: -1,
            sharpen_mask: false,
        };
        let result = compositor_core::filters::run_filter(&job);
        if let Some(id) = self.session.active_layer_id {
            self.session.history.begin(
                "Content-Aware Fill",
                &self.session.document,
                self.session.active_layer_id,
            );
            if let Some(doc) = self.session.document.as_mut() {
                if let Some(layer) = doc.layer_mut(id) {
                    layer.asset = Some(ImportedImage::new(result, layer.name.clone()));
                }
            }
            self.session
                .history
                .end(&self.session.document, self.session.active_layer_id);
        }
        self.touch(cx);
    }

    /// Option-P with the Levels sheet open: flips its Preview checkbox.
    pub fn toggle_levels_preview(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.sheet.as_mut() else {
            return;
        };
        if sheet.kind != SheetKind::Levels {
            return;
        }
        sheet.checkbox_a = !sheet.checkbox_a;
        self.refresh_levels_preview(cx);
    }

    /// Every mutation funnels through here so the frame image rebuilds.
    pub fn touch(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        cx.notify();
    }

    pub fn notify_only(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    // MARK: View transform

    pub fn doc_size(&self) -> (f64, f64) {
        self.session
            .document
            .as_ref()
            .map(|d| (d.width as f64, d.height as f64))
            .unwrap_or((0.0, 0.0))
    }

    pub fn doc_to_screen(&self, x: f64, y: f64) -> (f32, f32) {
        doc_to_screen_at(self.zoom, self.pan, x, y)
    }

    pub fn screen_to_doc(&self, x: f32, y: f32) -> CGPoint {
        screen_to_doc_at(self.zoom, self.pan, x, y)
    }

    /// The document rect in viewport pixels.
    pub fn image_rect(&self) -> Bounds<Pixels> {
        let (w, h) = self.doc_size();
        let (x, y) = self.doc_to_screen(0.0, 0.0);
        Bounds::<Pixels>::new(
            Point::new(px(x), px(y)),
            gpui_kit::size(px((w * self.zoom) as f32), px((h * self.zoom) as f32)),
        )
    }

    pub fn zoom_at(&mut self, screen: (f32, f32), factor: f64) {
        let doc = self.screen_to_doc(screen.0, screen.1);
        self.zoom = clamp_zoom(self.zoom * factor);
        self.pan = (
            screen.0 as f64 - doc.x * self.zoom,
            screen.1 as f64 - doc.y * self.zoom,
        );
        self.revision += 1;
    }

    pub fn zoom_fit(&mut self, viewport: (f32, f32)) {
        let (w, h) = self.doc_size();
        if w <= 0.0 || h <= 0.0 || viewport.0 <= 0.0 || viewport.1 <= 0.0 {
            return;
        }
        let margin = 48.0f64;
        let zoom = ((viewport.0 as f64 - margin) / w)
            .min((viewport.1 as f64 - margin) / h)
            .clamp(MIN_ZOOM, 1.0);
        self.zoom = zoom;
        self.pan = (
            (viewport.0 as f64 - w * zoom) / 2.0,
            (viewport.1 as f64 - h * zoom) / 2.0,
        );
        self.fit_for = (w as usize, h as usize);
        self.revision += 1;
    }

    // MARK: Frame image

    fn set_preview_layer(layer: &mut ImageLayer, pixels: &Bitmap, transform: LayerTransform) {
        layer.asset = Some(ImportedImage::with_thumbnail(
            pixels.clone(),
            compositor_core::raster::thumbnail_of(pixels),
            layer.name.clone(),
        ));
        layer.transform = transform;
    }

    /// The composited canvas, with any in-progress stroke or adjustment
    /// preview standing in for the active layer's pixels.
    pub fn composite(&self) -> Option<Bitmap> {
        let document = self.session.document.as_ref()?;
        let active_id = self.session.active_layer_id;
        let override_pixels = |doc: &mut CanvasDocument, pixels: &Bitmap, transform: LayerTransform| {
            if let Some(id) = active_id {
                if let Some(layer) = doc.layer_mut(id) {
                    Self::set_preview_layer(layer, pixels, transform);
                }
            }
        };

        // Preview pixels override the active layer while a sheet is open.
        if let (Some(preview), Some(base_transform)) = (&self.preview, &self.preview_base_transform)
        {
            let mut doc = document.clone();
            override_pixels(&mut doc, preview, *base_transform);
            return Some(render::draw_live_composite(&doc));
        }
        // A stroke in progress overrides the active layer too.
        if let Some(CanvasDrag::Stroke(edit)) = &self.drag {
            let result = edit.result();
            let mut doc = document.clone();
            override_pixels(&mut doc, &result, edit.paint_transform);
            return Some(render::draw_live_composite(&doc));
        }
        // A Smudge / Liquify stroke shows its document-size working copy in
        // place of the layer, exactly as the Swift canvas does.
        if let Some(CanvasDrag::Warp(stroke)) = &self.drag {
            let mut doc = document.clone();
            override_pixels(
                &mut doc,
                stroke.image(),
                compositor_core::transform::LayerTransform::new(
                    compositor_core::geometry::CGPoint::ZERO,
                    document.size(),
                ),
            );
            return Some(render::draw_live_composite(&doc));
        }
        // The Shape tool previews its layer live while dragging.
        if let Some(CanvasDrag::Shape { .. }) = &self.drag {
            if let (Some(shape), Some(rect)) = (&self.shape_shape, &self.shape_rect) {
                let mut doc = document.clone();
                let raster = render::render_shape_image(shape, rect.width().round() as usize, rect.height().round() as usize);
                let mut layer = ImageLayer::with_asset(
                    compositor_core::raster::ImportedImage::new(raster, String::new()),
                    rect.origin,
                );
                layer.transform = compositor_core::transform::LayerTransform::new(rect.origin, rect.size);
                doc.layers.push(layer);
                return Some(render::draw_live_composite(&doc));
            }
        }
        // A gradient drag previews the fill.
        if let Some(CanvasDrag::Gradient(edit, start)) = &self.drag {
            let result = self.gradient_result(edit, *start);
            let mut doc = document.clone();
            override_pixels(&mut doc, &result, edit.paint_transform);
            return Some(render::draw_live_composite(&doc));
        }
        Some(render::draw_live_composite(document))
    }

    pub(crate) fn gradient_result(&self, edit: &RasterEdit, start: CGPoint) -> Bitmap {
        let settings = self.session.gradient_settings;
        let fg = straight_of(self.session.palette_color(false));
        let bg = straight_of(self.session.palette_color(true));
        // "Foreground to Transparent" runs fg at full alpha down to fg at zero
        // alpha (Gradient.swift:78-80) — repeating the fg color fills solid.
        let mut colors = if settings.foreground_to_background {
            [fg, bg]
        } else {
            let mut transparent = fg;
            transparent[3] = 0;
            [fg, transparent]
        };
        if settings.reversed {
            colors.reverse();
        }
        let end = self.gradient_cursor.unwrap_or(start);
        edit.fill_gradient(settings.shape, start, end, colors, settings.opacity)
    }

    /// The cached BGRA texture for the composite, rebuilt when the revision
    /// changes.
    pub fn frame_image(&mut self, composite: &Bitmap) -> Arc<RenderImage> {
        if self.frame_image.is_none() || self.frame_image_rev != self.revision {
            self.frame_image = Some(bgra_render_image(composite));
            self.frame_image_rev = self.revision;
        }
        self.frame_image.clone().unwrap()
    }

    /// The cached thumbnail texture for one layer row, keyed by layer and by
    /// whether the mask is the row's subject (the mask thumbnail swaps in).
    pub fn thumbnail_image(
        &mut self,
        id: uuid::Uuid,
        layer: &ImageLayer,
    ) -> Option<Arc<RenderImage>> {
        let mask_selected = self.session.is_mask_selected && layer.mask.is_some();
        if let Some((rev, image)) = self.thumbnails.get(&(id, mask_selected)) {
            if *rev == self.revision {
                return Some(image.clone());
            }
        }
        let document = self.session.document.as_ref()?;
        let canvas = compositor_core::geometry::CGSize::new(
            document.width as f64,
            document.height as f64,
        );
        // CanvasThumbnail: the pixels (or the mask) sit on a canvas-shaped
        // frame, whatever the layer's own bounds.
        let thumbnail = if mask_selected {
            render::mask_canvas_thumbnail(
                layer.mask.as_ref()?,
                &layer.mask_transform(),
                canvas,
            )?
        } else {
            render::layer_canvas_thumbnail(layer, canvas)?
        };
        let image = bgra_render_image(&thumbnail);
        self.thumbnails
            .insert((id, mask_selected), (self.revision, image.clone()));
        Some(image)
    }

    // MARK: Status

    pub fn set_status(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.status = Some(message.into());
        self.touch(cx);
    }

    // MARK: Rename

    pub fn begin_rename(&mut self, id: uuid::Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let current = self
            .session
            .document
            .as_ref()
            .and_then(|d| d.layer(id))
            .map(|l| l.name.clone())
            .unwrap_or_default();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
        let subscription = cx.subscribe(
            &input,
            |this: &mut Self,
             _input,
             event: &gpui_kit::component::input::InputEvent,
             cx: &mut Context<Self>| {
                match event {
                    gpui_kit::component::input::InputEvent::PressEnter { .. }
                    | gpui_kit::component::input::InputEvent::Blur => this.commit_rename(cx),
                    _ => {}
                }
            },
        );
        self.subscriptions.push(subscription);
        self.rename = Some(id);
        self.rename_input = Some(input);
        self.notify_only(cx);
    }

    pub fn commit_rename(&mut self, cx: &mut Context<Self>) {
        if let (Some(id), Some(input)) = (self.rename, self.rename_input.clone()) {
            let text = input.read(cx).value().to_string();
            self.session.rename_layer(id, &text);
        }
        self.rename = None;
        self.rename_input = None;
        self.touch(cx);
    }

    #[allow(dead_code)]
    pub fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        self.rename = None;
        self.rename_input = None;
        self.notify_only(cx);
    }

    // MARK: Sheets

    pub fn close_sheet(&mut self, cx: &mut Context<Self>) {
        self.sheet = None;
        self.preview = None;
        self.preview_base = None;
        self.preview_base_transform = None;
        self.gradient_cursor = None;
        self.touch(cx);
    }

    /// Commits the preview into the active layer's pixels as one undo step.
    pub fn commit_preview(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Some(preview) = self.preview.clone() {
            let transform = self.preview_base_transform.unwrap_or_default();
            self.session.history.begin(
                name,
                &self.session.document,
                self.session.active_layer_id,
            );
            if let Some(id) = self.session.active_layer_id {
                if let Some(doc) = self.session.document.as_mut() {
                    if let Some(layer) = doc.layer_mut(id) {
                        layer.asset = Some(ImportedImage::new(
                            preview.as_ref().clone(),
                            layer.name.clone(),
                        ));
                        layer.transform = transform;
                    }
                }
            }
            self.session
                .history
                .end(&self.session.document, self.session.active_layer_id);
        }
        self.close_sheet(cx);
    }

    /// The active selection clipped to the canvas (`selection?.clip(canvas:)`).
    pub(crate) fn selection_clip(&self) -> Option<compositor_core::selection::SelectionClip> {
        self.session.selection().and_then(|selection| {
            self.session
                .document
                .as_ref()
                .map(|document| selection.clip(document.size()))
        })
    }

    /// Runs `filter` over the preview base and shows it live on canvas.
    pub fn apply_filter_preview(&mut self, kind: FilterKind, settings: FilterSettings) {
        let Some(base) = self.preview_base.clone() else { return };
        let selection = self.selection_clip();
        let job = FilterJob {
            kind,
            image: &base,
            settings,
            scale: 1.0,
            selection,
            mapping: CGAffineTransform::IDENTITY,
            seed: 7,
            noise_origin: CGPoint::ZERO,
            camera_raw_clipping: None,
            // Swift's default is -1: no visualization (CameraRaw.swift:221).
            visualize_point_color: -1,
            sharpen_mask: false,
        };
        let result = compositor_core::filters::run_filter(&job);
        self.preview = Some(Arc::new(result));
        self.revision += 1;
    }

    /// `sampleCameraRawDefringe` — centers the purple or green defringe range
    /// on the clicked fringe color and refreshes the preview.
    pub fn sample_camera_raw_defringe(&mut self, at: CGPoint, cx: &mut Context<Self>) {
        let Some(base) = self.preview_base.clone() else {
            return;
        };
        let px = at.x.floor();
        let py = at.y.floor();
        if px < 0.0 || py < 0.0 || px >= base.width as f64 || py >= base.height as f64 {
            return;
        }
        let Some(hue) =
            compositor_core::adjust::camera_raw::straight_pixel_hue(base.pixel(px as usize, py as usize))
        else {
            return;
        };
        let mut settings = self.filter_settings_from_sheet(cx);
        compositor_core::adjust::camera_raw::apply_defringe_sample(
            &mut settings.camera_raw.optics,
            hue,
        );
        self.camera_raw_samples_defringe = false;
        self.apply_filter_preview(FilterKind::CameraRaw, settings);
    }

    /// `updateCameraRawReadout` — the RGB under the pointer; outside the
    /// layer the readout clears.
    pub fn update_camera_raw_readout(&mut self, at: CGPoint) {
        let Some(base) = self.preview_base.clone() else {
            self.camera_raw_readout = None;
            return;
        };
        let px = at.x.floor();
        let py = at.y.floor();
        if px < 0.0 || py < 0.0 || px >= base.width as f64 || py >= base.height as f64 {
            self.camera_raw_readout = None;
            return;
        }
        let pixel = base.pixel(px as usize, py as usize);
        let alpha = pixel[3] as f64;
        if alpha <= 0.0 {
            self.camera_raw_readout = None;
            return;
        }
        self.camera_raw_readout = Some([
            ((pixel[0] as f64 / alpha * 255.0).round() as u8).min(255),
            ((pixel[1] as f64 / alpha * 255.0).round() as u8).min(255),
            ((pixel[2] as f64 / alpha * 255.0).round() as u8).min(255),
        ]);
    }

    /// `applyCameraRawAutoWhiteBalance` — selects Auto and solves the
    /// temperature/tint from the preview base.
    pub fn apply_camera_raw_auto_white_balance(&mut self, cx: &mut Context<Self>) {
        use compositor_core::adjust::camera_raw::CameraRawWhiteBalance;
        let Some(base) = self.preview_base.clone() else {
            return;
        };
        let mut settings = self.filter_settings_from_sheet(cx);
        settings.camera_raw.white_balance = CameraRawWhiteBalance::Auto;
        if let Some((temperature, tint)) =
            compositor_core::adjust::camera_raw::CameraRawSettings::auto_balance(&base)
        {
            settings.camera_raw.temperature = temperature;
            settings.camera_raw.tint = tint;
        }
        self.apply_filter_preview(FilterKind::CameraRaw, settings);
    }

    // MARK: Keys

    /// Single-letter shortcuts and nudges are ignored while a text input or
    /// an open sheet owns the keyboard.
    pub fn canvas_keys_active(&self, window: &mut Window, cx: &mut App) -> bool {
        if self.sheet.is_some() {
            return false;
        }
        !window.has_focused_input(cx)
    }
}

/// Straight [r, g, b, a] bytes for a palette color (fully opaque).
fn straight_of(color: compositor_core::document::PaletteColor) -> [u8; 4] {
    [
        (color.red * 255.0).round() as u8,
        (color.green * 255.0).round() as u8,
        (color.blue * 255.0).round() as u8,
        255,
    ]
}

/// Converts a premultiplied-RGBA bitmap into the BGRA texture GPUI expects.
pub fn bgra_render_image(bitmap: &Bitmap) -> Arc<RenderImage> {
    let mut data = bitmap.data.clone();
    for px in data.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let buffer = image::ImageBuffer::from_raw(bitmap.width as u32, bitmap.height as u32, data)
        .expect("bitmap size matches data");
    Arc::new(RenderImage::new(vec![Frame::new(buffer)]))
}

#[cfg(test)]
mod tests {
    use super::{clamp_zoom, doc_to_screen_at, screen_to_doc_at, MAX_ZOOM, MIN_ZOOM};

    #[test]
    fn doc_to_screen_applies_zoom_and_pan() {
        // A 100x100 document at 2x zoom, panned so its origin sits at (40, 30).
        let (sx, sy) = doc_to_screen_at(2.0, (40.0, 30.0), 10.0, 5.0);
        assert_eq!((sx, sy), (60.0, 40.0));
        // Identity zoom and zero pan map document coordinates unchanged.
        assert_eq!(doc_to_screen_at(1.0, (0.0, 0.0), 12.5, 3.25), (12.5, 3.25));
    }

    #[test]
    fn screen_doc_roundtrip_recovers_document_point() {
        let zoom = 0.35;
        let pan = (-123.0, 456.5);
        let (sx, sy) = doc_to_screen_at(zoom, pan, 88.0, -21.0);
        let doc = screen_to_doc_at(zoom, pan, sx, sy);
        // Screen coordinates are f32, so the roundtrip loses some precision.
        assert!((doc.x - 88.0).abs() < 1e-3, "x = {}", doc.x);
        assert!((doc.y - -21.0).abs() < 1e-3, "y = {}", doc.y);
    }

    #[test]
    fn clamp_zoom_keeps_zoom_in_range() {
        assert_eq!(clamp_zoom(1.0), 1.0);
        assert_eq!(clamp_zoom(0.000_001), MIN_ZOOM);
        assert_eq!(clamp_zoom(1_000.0), MAX_ZOOM);
        assert_eq!(clamp_zoom(2.5), 2.5);
    }

    #[test]
    fn screen_doc_roundtrip_holds_at_zoom_extremes() {
        for zoom in [MIN_ZOOM, 0.1, 1.0, 8.0, MAX_ZOOM] {
            let pan = (37.5, -18.25);
            let (sx, sy) = doc_to_screen_at(zoom, pan, 250.0, 90.0);
            let doc = screen_to_doc_at(zoom, pan, sx, sy);
            assert!((doc.x - 250.0).abs() < 1e-2, "zoom {zoom}: x = {}", doc.x);
            assert!((doc.y - 90.0).abs() < 1e-2, "zoom {zoom}: y = {}", doc.y);
        }
    }
}

