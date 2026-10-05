//! Port of the color-picker session API (`Document/ColorPalette.swift`
//! `extension EditorSession`) — open/commit/cancel and live preview.
//! `ColorPickerState`/`ColorPickerTarget` live on `session`.

use crate::document::{NavigationTool, PaletteColor};
use crate::effects::LayerEffectKind;
use crate::session::{ColorPickerState, ColorPickerTarget, EditorSession, TextDraft};

/// Swift's `TextDraft` is `Identifiable` with a fresh UUID per draft; the Rust
/// `TextDraft` carries no id, so a draft is identified by the layer it edits —
/// or by its document when the draft will still create a new layer. At most
/// one draft exists at a time, so the same-draft check in `closeColorPicker`
/// stays exact except across a cancel-and-reopen of the Type tool on the same
/// layer while the picker stays open.
fn text_draft_identity(draft: &TextDraft) -> Option<uuid::Uuid> {
    draft.layer_id.or(Some(draft.document_id))
}

impl EditorSession {
    /// `canEditPalette` — `!isProjectBusy && brushStroke == nil` (the `_ =
    /// showsBusy` prefix only subscribes the view to the busy flag). The core
    /// session models neither a busy project nor an in-flight brush stroke —
    /// both live at the app layer, as with `can_edit_layers` — so this is
    /// always true here and the app conjuncts its own state on top.
    pub fn can_edit_palette(&self) -> bool {
        true
    }

    /// `setPaletteColor(_:background:)`.
    pub fn set_palette_color(&mut self, color: PaletteColor, background: bool) {
        if !self.can_edit_palette() {
            return;
        }
        if self.is_mask_selected {
            let white = color == PaletteColor::WHITE;
            self.mask_paint_white = if background { !white } else { white };
        } else if background {
            self.background_color = color;
        } else {
            self.brush_settings.red = color.red;
            self.brush_settings.green = color.green;
            self.brush_settings.blue = color.blue;
            // Type paints in the foreground color, so text being edited
            // follows the swatch. A text layer merely selected keeps its
            // color: it changes only while its text is open for editing.
            if self.tool == NavigationTool::Type && self.text_draft.is_some() {
                self.change_text_style(|style| {
                    style.red = color.red;
                    style.green = color.green;
                    style.blue = color.blue;
                });
            }
        }
    }

    /// `var typeColor` — what the Type bar's swatch shows and edits: the text
    /// being edited, otherwise the foreground color the next text will use. A
    /// text layer that is only selected is not touched.
    pub fn type_color(&self) -> PaletteColor {
        match self.text_draft.as_ref() {
            Some(draft) => PaletteColor::new(draft.style.red, draft.style.green, draft.style.blue),
            // `foregroundColor`
            None => PaletteColor::new(
                self.brush_settings.red,
                self.brush_settings.green,
                self.brush_settings.blue,
            ),
        }
    }

    /// `openColorPicker(background:)`.
    pub fn open_color_picker(&mut self, background: bool) {
        if !self.can_edit_palette() || self.is_mask_selected {
            return;
        }
        let original = self.palette_color(background);
        self.color_picker = Some(ColorPickerState::new(
            ColorPickerTarget::Palette { background },
            original,
        ));
    }

    /// `openTextColorPicker()`.
    pub fn open_text_color_picker(&mut self) {
        if !self.can_edit_palette()
            || self.color_picker.is_some()
            || self.tool != NavigationTool::Type
        {
            return;
        }
        let draft_id = self.text_draft.as_ref().and_then(text_draft_identity);
        let original = self.type_color();
        self.color_picker = Some(ColorPickerState::new(
            ColorPickerTarget::Text { draft_id },
            original,
        ));
    }

    /// `openEffectColorPicker(_:)` — opens the picker on a layer effect's
    /// own color.
    pub fn open_effect_color_picker(&mut self, kind: LayerEffectKind) {
        if !self.can_edit_palette() || self.color_picker.is_some() || self.effects_editing.is_none()
        {
            return;
        }
        // `editingEffects` (LayerEffects.swift:224): the layer the panel is
        // bound to, or an empty set when that layer is gone.
        let effects = self
            .document
            .as_ref()
            .and_then(|document| {
                self.effects_editing
                    .as_ref()
                    .and_then(|editing| document.layer(editing.layer_id))
            })
            .and_then(|layer| layer.effects)
            .unwrap_or_default();
        let original = effects.color(kind).unwrap_or(PaletteColor::BLACK);
        self.color_picker = Some(ColorPickerState::new(
            ColorPickerTarget::Effect { kind },
            original,
        ));
    }

    /// `closeColorPicker(commit:)`.
    pub fn close_color_picker(&mut self, commit: bool) {
        if let Some(picker) = self.color_picker.clone() {
            match picker.target {
                ColorPickerTarget::Palette { background } => {
                    if commit && !self.is_mask_selected {
                        self.set_palette_color(picker.color(), background);
                    }
                }
                ColorPickerTarget::Text { draft_id } => {
                    if commit
                        && self.tool == NavigationTool::Type
                        && self.text_draft.as_ref().and_then(text_draft_identity) == draft_id
                    {
                        let color = picker.color();
                        if draft_id.is_some() {
                            self.change_text_style(|style| {
                                style.red = color.red;
                                style.green = color.green;
                                style.blue = color.blue;
                            });
                        } else {
                            let defaults = &mut self.text_defaults;
                            defaults.red = color.red;
                            defaults.green = color.green;
                            defaults.blue = color.blue;
                        }
                        // The text color is the foreground color: picking one
                        // in the Type bar moves the swatch too.
                        if !self.is_mask_selected {
                            self.brush_settings.red = color.red;
                            self.brush_settings.green = color.green;
                            self.brush_settings.blue = color.blue;
                        }
                    }
                }
                ColorPickerTarget::Effect { kind } => {
                    // The end has been previewing the working color; Cancel
                    // puts the original back.
                    let color = if commit {
                        picker.color()
                    } else {
                        picker.original
                    };
                    self.change_effect_color_for_preview(kind, color);
                }
                ColorPickerTarget::GradientMap { .. } => {
                    // `setGradientMapColor(commit ? colorPicker.color :
                    // colorPicker.original, highlights:)` — not portable yet:
                    // the core has no filter-edit session (`filterEdit` /
                    // `updateFilter`) for a Gradient Map to write into, so
                    // nothing can open this target either.
                }
            }
        }
        self.color_picker = None;
    }

    /// `previewEffectColor()` — while the picker is open on an effect's
    /// color, the canvas follows its working color.
    pub fn preview_effect_color(&mut self) {
        let Some(picker) = self.color_picker.as_ref() else {
            return;
        };
        let ColorPickerTarget::Effect { kind } = picker.target else {
            return;
        };
        let color = picker.color();
        self.change_effect_color_for_preview(kind, color);
    }

    /// `changeEffects { $0.setColor(color, for: kind) }` inlined — the shared
    /// `changeEffects` is owned by effects_session.rs, so the color write and
    /// its commit live here instead, with the same semantics as
    /// LayerEffects.swift:313-320 and `setEffects` (:305-311).
    fn change_effect_color_for_preview(&mut self, kind: LayerEffectKind, color: PaletteColor) {
        // Panel edits stay bound to the layer that opened the panel, and that
        // layer must already carry the edited effect.
        let Some(editing) = self.effects_editing else {
            return;
        };
        let Some(layer) = self
            .document
            .as_ref()
            .and_then(|document| document.layer(editing.layer_id))
        else {
            return;
        };
        if !layer
            .effects
            .map(|effects| effects.contains(editing.kind))
            .unwrap_or(false)
        {
            return;
        }
        let layer_id = layer.id;
        let current = layer.effects;
        let is_group = layer.is_group;
        let has_asset = layer.asset.is_some();
        let mut effects = current.unwrap_or_default();
        effects.set_color(color, kind);
        // `setEffects` guards: the layer must take effects, and an unchanged
        // set writes nothing. The set stays non-empty here — the edited
        // effect exists — so it stores `Some`.
        if !self.can_edit_layers() || !effects.is_valid() || is_group || !has_asset {
            return;
        }
        if current == Some(effects) {
            return;
        }
        self.finish_opacity_edit();
        let name = format!("Edit {}", editing.kind.label());
        self.history
            .begin(&name, &self.document, self.active_layer_id);
        if let Some(document) = self.document.as_mut()
            && let Some(layer) = document.layer_mut(layer_id)
        {
            layer.effects = Some(effects);
        }
        self.history.end(&self.document, self.active_layer_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::PickerHSB;
    use crate::effects::{LayerEffectSelection, LayerEffects, ShadowEffect};
    use crate::geometry::CGPoint;
    use crate::raster::{Bitmap, ImportedImage};

    fn session() -> EditorSession {
        let mut s = EditorSession::new();
        s.create_document(100, 100, true);
        s
    }

    fn opaque_bitmap(size: usize) -> Bitmap {
        let mut image = Bitmap::new(size, size);
        for i in (0..image.data.len()).step_by(4) {
            image.data[i] = 255;
            image.data[i + 3] = 255;
        }
        image
    }

    /// A document whose one pixel layer carries a black Drop Shadow and whose
    /// Effects panel is editing that layer — what `addEffect`/`selectEffect`
    /// would leave behind.
    fn session_editing_shadow() -> (EditorSession, uuid::Uuid) {
        let mut s = session();
        let layer_id = s.active_layer_id.unwrap();
        {
            let layer = s.document.as_mut().unwrap().layer_mut(layer_id).unwrap();
            layer.asset = Some(ImportedImage::new(opaque_bitmap(8), "Layer 1"));
            layer.effects = Some(LayerEffects {
                shadow: Some(ShadowEffect::default()),
                ..Default::default()
            });
        }
        s.effects_editing = Some(LayerEffectSelection {
            layer_id,
            kind: LayerEffectKind::Shadow,
        });
        (s, layer_id)
    }

    // (a) openColorPicker guards

    #[test]
    fn open_color_picker_refuses_while_a_mask_is_selected() {
        let mut s = session();
        s.add_layer_mask(false);
        assert!(s.is_mask_selected);
        s.open_color_picker(false);
        assert!(s.color_picker.is_none());
    }

    #[test]
    fn open_color_picker_loads_the_swatch_as_the_original() {
        let mut s = session();
        // The foreground swatch starts black, the background white.
        s.open_color_picker(false);
        let picker = s.color_picker.clone().unwrap();
        assert_eq!(
            picker.target,
            ColorPickerTarget::Palette { background: false }
        );
        assert_eq!(picker.original, PaletteColor::BLACK);
        assert!(!picker.background());
        s.close_color_picker(false);
        s.open_color_picker(true);
        let picker = s.color_picker.clone().unwrap();
        assert_eq!(
            picker.target,
            ColorPickerTarget::Palette { background: true }
        );
        assert_eq!(picker.original, PaletteColor::WHITE);
        assert!(picker.background());
    }

    // (b) closeColorPicker(commit: true) on a palette target

    #[test]
    fn commit_writes_the_quantized_working_color_to_the_foreground_swatch() {
        let mut s = session();
        s.open_color_picker(false);
        s.color_picker.as_mut().unwrap().hsb = PickerHSB::new(0.0, 1.0, 0.5);
        let working = s.color_picker.as_ref().unwrap().color();
        s.close_color_picker(true);
        assert_eq!(working.red, 128.0 / 255.0);
        assert_eq!(s.brush_settings.red, working.red);
        assert_eq!(s.brush_settings.green, 0.0);
        assert_eq!(s.brush_settings.blue, 0.0);
        assert!(s.color_picker.is_none());
    }

    #[test]
    fn commit_writes_the_working_color_to_the_background_swatch() {
        let mut s = session();
        s.open_color_picker(true);
        s.color_picker.as_mut().unwrap().hsb = PickerHSB::new(120.0, 1.0, 1.0);
        s.close_color_picker(true);
        assert_eq!(s.background_color, PaletteColor::new(0.0, 1.0, 0.0));
        // The foreground swatch is untouched.
        assert_eq!(s.brush_settings.red, 0.0);
    }

    // (c) closeColorPicker(commit: false) on a palette target

    #[test]
    fn cancel_leaves_the_palette_at_the_original() {
        let mut s = session();
        s.open_color_picker(false);
        s.color_picker.as_mut().unwrap().hsb = PickerHSB::new(120.0, 1.0, 1.0);
        s.close_color_picker(false);
        assert_eq!(s.brush_settings.red, 0.0);
        assert_eq!(s.brush_settings.green, 0.0);
        assert_eq!(s.brush_settings.blue, 0.0);
        assert_eq!(s.background_color, PaletteColor::WHITE);
        assert!(s.color_picker.is_none());
    }

    // setPaletteColor

    #[test]
    fn set_palette_color_on_a_mask_drives_mask_paint_white() {
        let mut s = session();
        s.add_layer_mask(false);
        s.set_palette_color(PaletteColor::WHITE, false);
        assert!(s.mask_paint_white);
        s.set_palette_color(PaletteColor::BLACK, false);
        assert!(!s.mask_paint_white);
        // The background swatch sets the opposite paint color on a mask.
        s.set_palette_color(PaletteColor::WHITE, true);
        assert!(!s.mask_paint_white);
    }

    #[test]
    fn set_palette_color_follows_the_open_text_draft() {
        let mut s = session();
        s.begin_text(CGPoint::new(10.0, 10.0), true);
        s.set_palette_color(PaletteColor::new(1.0, 0.0, 0.0), false);
        assert_eq!(s.text_draft.as_ref().unwrap().style.red, 1.0);
        assert_eq!(s.brush_settings.red, 1.0);
    }

    // (d) the text target

    #[test]
    fn text_picker_refuses_when_the_type_tool_is_not_active() {
        let mut s = session();
        s.open_text_color_picker();
        assert!(s.color_picker.is_none());
    }

    #[test]
    fn text_picker_commit_edits_the_pending_draft() {
        let mut s = session();
        s.begin_text(CGPoint::new(10.0, 10.0), true);
        s.open_text_color_picker();
        // A new layer's draft is identified by its document.
        let document_id = s.document.as_ref().unwrap().id;
        let picker = s.color_picker.clone().unwrap();
        assert_eq!(
            picker.target,
            ColorPickerTarget::Text {
                draft_id: Some(document_id)
            }
        );
        assert_eq!(picker.original, s.type_color());
        s.color_picker.as_mut().unwrap().hsb = PickerHSB::new(0.0, 1.0, 1.0);
        s.close_color_picker(true);
        let draft = s.text_draft.as_ref().unwrap();
        assert_eq!(draft.style.red, 1.0);
        assert_eq!(draft.style.green, 0.0);
        // The text color is the foreground color: the swatch follows.
        assert_eq!(s.brush_settings.red, 1.0);
    }

    #[test]
    fn text_picker_without_a_draft_edits_the_defaults() {
        let mut s = session();
        s.tool = NavigationTool::Type;
        s.open_text_color_picker();
        let picker = s.color_picker.clone().unwrap();
        assert_eq!(picker.target, ColorPickerTarget::Text { draft_id: None });
        s.color_picker.as_mut().unwrap().hsb = PickerHSB::new(0.0, 1.0, 1.0);
        s.close_color_picker(true);
        assert_eq!(s.text_defaults.red, 1.0);
        assert!(s.text_draft.is_none());
        assert_eq!(s.brush_settings.red, 1.0);
    }

    #[test]
    fn text_picker_commit_needs_the_same_draft_still_open() {
        let mut s = session();
        s.tool = NavigationTool::Type;
        s.open_text_color_picker();
        // A draft appearing after the picker opened is not the picker's draft.
        s.begin_text(CGPoint::new(10.0, 10.0), true);
        s.color_picker.as_mut().unwrap().hsb = PickerHSB::new(0.0, 1.0, 1.0);
        s.close_color_picker(true);
        assert_eq!(s.text_defaults.red, 0.0);
    }

    // (e) the effect target

    #[test]
    fn effect_picker_refuses_without_effects_editing() {
        let mut s = session();
        s.open_effect_color_picker(LayerEffectKind::Shadow);
        assert!(s.color_picker.is_none());
    }

    #[test]
    fn effect_picker_commit_writes_the_layer_effect_with_undo() {
        let (mut s, layer_id) = session_editing_shadow();
        s.open_effect_color_picker(LayerEffectKind::Shadow);
        let picker = s.color_picker.clone().unwrap();
        assert_eq!(
            picker.target,
            ColorPickerTarget::Effect {
                kind: LayerEffectKind::Shadow
            }
        );
        assert_eq!(picker.original, PaletteColor::BLACK);
        s.color_picker.as_mut().unwrap().hsb = PickerHSB::new(240.0, 1.0, 1.0);
        s.close_color_picker(true);
        let shadow = s
            .document
            .as_ref()
            .unwrap()
            .layer(layer_id)
            .unwrap()
            .effects
            .unwrap()
            .shadow
            .unwrap();
        assert_eq!(shadow.color(), PaletteColor::new(0.0, 0.0, 1.0));
        assert_eq!(s.history.undo_name(), "Edit Drop Shadow");
        assert!(s.color_picker.is_none());
        // One undo step restores the original color.
        s.undo();
        let shadow = s
            .document
            .as_ref()
            .unwrap()
            .layer(layer_id)
            .unwrap()
            .effects
            .unwrap()
            .shadow
            .unwrap();
        assert_eq!(shadow.color(), PaletteColor::BLACK);
    }

    #[test]
    fn effect_picker_cancel_restores_the_original_color() {
        let (mut s, layer_id) = session_editing_shadow();
        s.open_effect_color_picker(LayerEffectKind::Shadow);
        s.color_picker.as_mut().unwrap().hsb = PickerHSB::new(240.0, 1.0, 1.0);
        s.preview_effect_color();
        s.close_color_picker(false);
        let shadow = s
            .document
            .as_ref()
            .unwrap()
            .layer(layer_id)
            .unwrap()
            .effects
            .unwrap()
            .shadow
            .unwrap();
        assert_eq!(shadow.color(), PaletteColor::BLACK);
    }

    // (f) previewEffectColor

    #[test]
    fn preview_effect_color_updates_the_layer_live() {
        let (mut s, layer_id) = session_editing_shadow();
        s.open_effect_color_picker(LayerEffectKind::Shadow);
        s.color_picker.as_mut().unwrap().hsb = PickerHSB::new(240.0, 1.0, 1.0);
        s.preview_effect_color();
        let shadow = s
            .document
            .as_ref()
            .unwrap()
            .layer(layer_id)
            .unwrap()
            .effects
            .unwrap()
            .shadow
            .unwrap();
        assert_eq!(shadow.color(), PaletteColor::new(0.0, 0.0, 1.0));
        // The picker stays open while previewing.
        assert!(s.color_picker.is_some());
        assert_eq!(s.history.undo_name(), "Edit Drop Shadow");
    }

    #[test]
    fn preview_does_nothing_without_an_effect_picker() {
        let (mut s, layer_id) = session_editing_shadow();
        // A palette picker open: not an effect target, so no preview write.
        s.open_color_picker(false);
        s.preview_effect_color();
        let shadow = s
            .document
            .as_ref()
            .unwrap()
            .layer(layer_id)
            .unwrap()
            .effects
            .unwrap()
            .shadow
            .unwrap();
        assert_eq!(shadow.color(), PaletteColor::BLACK);
        // No picker at all: also a quiet no-op.
        s.close_color_picker(false);
        s.preview_effect_color();
        // The only undo entry is the document creation: nothing was written.
        assert_eq!(s.history.undo_name(), "New Canvas");
    }

    // The Gradient Map target cannot be opened yet (no filter-edit session),
    // and close_color_picker's GradientMap arm is a no-op; the match arm is
    // exercised through close on the other targets' tests above.
}
