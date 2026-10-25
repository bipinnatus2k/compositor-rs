//! Port of the effects-editing session API (`Document/LayerEffects.swift`
//! `extension EditorSession`, lines 221-348) — the Effects panel's backing
//! state machine. The panel edits stay bound to the layer that opened them
//! (`effects_editing`), every change commits a real undo step (Swift's
//! `changeEffects` → `setEffects`), and Cancel restores only the edited kind.

#![allow(dead_code)]

use uuid::Uuid;

use crate::effects::{
    ColorOverlayEffect, InnerGlowEffect, InnerShadowEffect, LayerEffectKind, LayerEffectSelection,
    LayerEffects, OuterGlowEffect, ShadowEffect, StrokeEffect,
};
use crate::session::EditorSession;

impl EditorSession {
    /// `canEditEffects` — a pixel layer with pixels (asset) must be active.
    pub fn can_edit_effects(&self) -> bool {
        self.can_edit_layers()
            && self
                .active_layer()
                .map(|layer| !layer.is_group && layer.asset.is_some())
                .unwrap_or(false)
    }

    /// `activeEffects`.
    pub fn active_effects(&self) -> LayerEffects {
        self.active_layer()
            .and_then(|layer| layer.effects)
            .unwrap_or_default()
    }

    /// `editingEffects` — the effects of the layer the panel is bound to,
    /// even after the selection moved elsewhere.
    pub fn editing_effects(&self) -> LayerEffects {
        self.effects_editing
            .and_then(|editing| {
                self.document
                    .as_ref()?
                    .layer(editing.layer_id)
                    .and_then(|layer| layer.effects)
            })
            .unwrap_or_default()
    }

    /// `selectedEffect` — the stored selection, while its layer is active and
    /// still carries the effect.
    pub fn selected_effect(&self) -> Option<LayerEffectSelection> {
        let selection = self.effect_selection?;
        if Some(selection.layer_id) != self.active_layer_id
            || !self.active_effects().contains(selection.kind)
        {
            return None;
        }
        Some(selection)
    }

    /// `addEffect(_:)` — adds the kind with its default look (a new stroke or
    /// color overlay takes the background color: the foreground is usually
    /// what the layer is painted in), selects it and starts editing.
    pub fn add_effect(&mut self, kind: LayerEffectKind) {
        if !self.can_edit_effects() {
            return;
        }
        let Some(id) = self.active_layer_id else {
            return;
        };
        if self.effects_editing == Some(LayerEffectSelection { layer_id: id, kind }) {
            return;
        }
        self.finish_effects_editing(false);
        let original = self.active_effects();
        let mut effects = original;
        let background = self.background_color;
        match kind {
            LayerEffectKind::Stroke if effects.stroke.is_none() => {
                effects.stroke = Some(StrokeEffect {
                    red: background.red,
                    green: background.green,
                    blue: background.blue,
                    ..StrokeEffect::default()
                });
            }
            LayerEffectKind::Shadow if effects.shadow.is_none() => {
                effects.shadow = Some(ShadowEffect::default());
            }
            LayerEffectKind::ColorOverlay if effects.color_overlay.is_none() => {
                effects.color_overlay = Some(ColorOverlayEffect {
                    red: background.red,
                    green: background.green,
                    blue: background.blue,
                    ..ColorOverlayEffect::default()
                });
            }
            LayerEffectKind::InnerShadow if effects.inner_shadow.is_none() => {
                effects.inner_shadow = Some(InnerShadowEffect::default());
            }
            LayerEffectKind::OuterGlow if effects.outer_glow.is_none() => {
                effects.outer_glow = Some(OuterGlowEffect::default());
            }
            LayerEffectKind::InnerGlow if effects.inner_glow.is_none() => {
                effects.inner_glow = Some(InnerGlowEffect::default());
            }
            _ => {}
        }
        self.set_effects(effects, Some(id), &format!("Add {}", kind.label()));
        self.select_effect(kind, id, true);
        // `selectEffect` snapshots the just-added effects; the panel's Cancel
        // must restore the state from *before* the add instead.
        self.effects_editing_original = Some(original);
    }

    /// `selectEffect(_:on:editing:)` — forces single selection, deselects the
    /// mask, and with `editing` binds the panel to this effect.
    pub fn select_effect(&mut self, kind: LayerEffectKind, id: Uuid, editing: bool) {
        if !self.can_edit_layers() {
            return;
        }
        let carries = self
            .document
            .as_ref()
            .and_then(|document| document.layer(id))
            .and_then(|layer| layer.effects)
            .map(|effects| effects.contains(kind))
            .unwrap_or(false);
        if !carries {
            return;
        }
        let selection = LayerEffectSelection { layer_id: id, kind };
        if editing && self.effects_editing != Some(selection) {
            self.close_color_picker(false);
        }
        self.select_layer(Some(id));
        self.selected_layer_ids = vec![id];
        self.is_mask_selected = false;
        self.effect_selection = Some(selection);
        if editing && self.effects_editing != Some(selection) {
            self.effects_editing_original = self
                .document
                .as_ref()
                .and_then(|document| document.layer(id))
                .and_then(|layer| layer.effects)
                .unwrap_or_default()
                .into();
            self.effects_editing = Some(selection);
        }
    }

    /// `finishEffectsEditing(commit:)` — Cancel restores only this panel's
    /// effect, preserving edits to other effects or layers. For a newly added
    /// effect the original value is absent, so Cancel removes it again.
    pub fn finish_effects_editing(&mut self, commit: bool) {
        let Some(editing) = self.effects_editing else {
            return;
        };
        self.close_color_picker(commit);
        if !commit && let Some(original) = self.effects_editing_original {
            // Swift only restores when the layer still *has* effects.
            if let Some(mut effects) = self
                .document
                .as_ref()
                .and_then(|document| document.layer(editing.layer_id))
                .and_then(|layer| layer.effects)
            {
                match editing.kind {
                    LayerEffectKind::Stroke => effects.stroke = original.stroke,
                    LayerEffectKind::Shadow => effects.shadow = original.shadow,
                    LayerEffectKind::ColorOverlay => effects.color_overlay = original.color_overlay,
                    LayerEffectKind::InnerShadow => effects.inner_shadow = original.inner_shadow,
                    LayerEffectKind::OuterGlow => effects.outer_glow = original.outer_glow,
                    LayerEffectKind::InnerGlow => effects.inner_glow = original.inner_glow,
                }
                self.set_effects(
                    effects,
                    Some(editing.layer_id),
                    &format!("Cancel {}", editing.kind.label()),
                );
            }
        }
        self.effects_editing = None;
        self.effects_editing_original = None;
        if self.selected_effect().is_none() {
            self.effect_selection = None;
        }
    }

    /// `removeSelectedEffect` (LayerEffects.swift:357-368): drop the selected
    /// kind from its layer's effects in one "Remove <kind>" step, releasing
    /// the panel binding when it pointed at the removed effect.
    pub fn remove_selected_effect(&mut self) {
        let Some(selection) = self.selected_effect() else {
            return;
        };
        if !self.can_edit_layers() {
            return;
        }
        if self.effects_editing == Some(selection) {
            self.close_color_picker(false);
            self.effects_editing = None;
            self.effects_editing_original = None;
        }
        let Some(mut effects) = self
            .document
            .as_ref()
            .and_then(|document| document.layer(selection.layer_id))
            .and_then(|layer| layer.effects)
        else {
            return;
        };
        effects.remove(selection.kind);
        self.set_effects(
            effects,
            Some(selection.layer_id),
            &format!("Remove {}", selection.kind.label()),
        );
        self.effect_selection = None;
    }

    /// `setEffects(_:on:name:)` — one undo step; rejects folders, layers
    /// without pixels and invalid values, and normalizes empty effects to nil.
    pub fn set_effects(&mut self, effects: LayerEffects, on: Option<Uuid>, name: &str) {
        if !self.can_edit_layers() || !effects.is_valid() {
            return;
        }
        let id = on.or(self.active_layer_id);
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let Some(index) = id.and_then(|id| document.layer_index(id)) else {
            return;
        };
        let layer = &document.layers[index];
        if layer.is_group || layer.asset.is_none() {
            return;
        }
        let normalized = if effects.is_empty() {
            None
        } else {
            Some(effects)
        };
        if layer.effects == normalized {
            return;
        }
        self.finish_opacity_edit();
        self.history
            .begin(name, &self.document, self.active_layer_id);
        if let Some(document) = self.document.as_mut() {
            document.layers[index].effects = normalized;
        }
        self.history.end(&self.document, self.active_layer_id);
    }

    /// `changeEffects(_:)` — panel edits stay bound to the layer that opened
    /// the panel, even if selection changes.
    pub fn change_effects(&mut self, change: impl FnOnce(&mut LayerEffects)) {
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
        let id = layer.id;
        let mut effects = layer.effects.unwrap_or_default();
        change(&mut effects);
        self.set_effects(effects, Some(id), &format!("Edit {}", editing.kind.label()));
    }

    /// `canCopyEffect(_:from:to:)`.
    pub fn can_copy_effect(&self, kind: LayerEffectKind, source: Uuid, target: Uuid) -> bool {
        if !self.can_edit_layers() || source == target {
            return false;
        }
        let carries = self
            .document
            .as_ref()
            .and_then(|document| document.layer(source))
            .and_then(|layer| layer.effects)
            .map(|effects| effects.contains(kind))
            .unwrap_or(false);
        let takes = self
            .document
            .as_ref()
            .and_then(|document| document.layer(target))
            .map(|layer| !layer.is_group && layer.asset.is_some())
            .unwrap_or(false);
        carries && takes
    }

    /// `copyEffect(_:from:to:)` — closes the destination's editor before
    /// replacing its effect so a later Cancel cannot undo the copy.
    pub fn copy_effect(&mut self, kind: LayerEffectKind, source: Uuid, target: Uuid) {
        if !self.can_copy_effect(kind, source, target) {
            return;
        }
        let Some(original) = self
            .document
            .as_ref()
            .and_then(|document| document.layer(source))
            .and_then(|layer| layer.effects)
        else {
            return;
        };
        if self.effects_editing
            == Some(LayerEffectSelection {
                layer_id: target,
                kind,
            })
        {
            self.finish_effects_editing(true);
        }
        let mut effects = self
            .document
            .as_ref()
            .and_then(|document| document.layer(target))
            .and_then(|layer| layer.effects)
            .unwrap_or_default();
        match kind {
            LayerEffectKind::Stroke => effects.stroke = original.stroke,
            LayerEffectKind::Shadow => effects.shadow = original.shadow,
            LayerEffectKind::ColorOverlay => effects.color_overlay = original.color_overlay,
            LayerEffectKind::InnerShadow => effects.inner_shadow = original.inner_shadow,
            LayerEffectKind::OuterGlow => effects.outer_glow = original.outer_glow,
            LayerEffectKind::InnerGlow => effects.inner_glow = original.inner_glow,
        }
        self.set_effects(effects, Some(target), &format!("Copy {}", kind.label()));
        self.select_effect(kind, target, false);
    }

    /// `toggleEffect(_:on:)` (LayerEffects.swift:350-355) — the effect row's
    /// eye: flips the kind's enabled flag in one "Hide/Show <kind>" undo
    /// step. A layer without effects is left alone.
    pub fn toggle_effect(&mut self, kind: LayerEffectKind, id: Uuid) {
        let Some(mut effects) = self
            .document
            .as_ref()
            .and_then(|document| document.layer(id))
            .and_then(|layer| layer.effects)
        else {
            return;
        };
        let enabled = effects.is_enabled(kind);
        effects.set_enabled(!enabled, kind);
        self.set_effects(
            effects,
            Some(id),
            &format!("{} {}", if enabled { "Hide" } else { "Show" }, kind.label()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ImageLayer, PaletteColor};
    use crate::geometry::CGSize;
    use crate::raster::{Bitmap, ImportedImage};
    use crate::session::EditorSession;

    /// A session with a document whose first layer carries pixels, so it can
    /// take effects (a local copy of session.rs's private `session()`).
    fn session() -> EditorSession {
        let mut s = EditorSession::new();
        s.create_document(100, 100, true);
        let doc = s.document.as_mut().unwrap();
        doc.layers[0].asset = Some(ImportedImage::new(Bitmap::new(10, 10), "Layer 1"));
        s
    }

    /// A second pixel layer, inserted above the active one.
    fn pixel_layer(s: &mut EditorSession, name: &str) -> Uuid {
        let layer = ImageLayer::with_asset(
            ImportedImage::new(Bitmap::new(10, 10), name),
            crate::geometry::CGPoint::ZERO,
        );
        let id = layer.id;
        s.document.as_mut().unwrap().layers.push(layer);
        id
    }

    fn effects_of(s: &EditorSession, id: Uuid) -> LayerEffects {
        s.document
            .as_ref()
            .unwrap()
            .layer(id)
            .and_then(|layer| layer.effects)
            .unwrap_or_default()
    }

    /// `removeSelectedEffect` (LayerEffects.swift:357-368): the selected
    /// kind leaves in one "Remove <kind>" step and the selection clears;
    /// removing the effect the panel edits releases the binding too.
    #[test]
    fn remove_selected_effect_drops_the_kind_in_one_step() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();

        s.add_effect(LayerEffectKind::Stroke);
        s.finish_effects_editing(true);
        s.add_effect(LayerEffectKind::Shadow);
        s.finish_effects_editing(true);
        s.select_effect(LayerEffectKind::Stroke, id, false);
        assert_eq!(
            s.selected_effect(),
            Some(LayerEffectSelection {
                layer_id: id,
                kind: LayerEffectKind::Stroke
            })
        );

        s.remove_selected_effect();
        let effects = effects_of(&s, id);
        assert!(!effects.contains(LayerEffectKind::Stroke));
        assert!(effects.contains(LayerEffectKind::Shadow));
        assert_eq!(s.history.undo_name(), "Remove Stroke");
        assert_eq!(s.selected_effect(), None);

        // Removing the effect the panel is editing releases the binding.
        s.select_effect(LayerEffectKind::Shadow, id, true);
        s.remove_selected_effect();
        assert_eq!(s.effects_editing, None);
        assert!(effects_of(&s, id).is_empty());
    }

    #[test]
    fn add_effect_defaults_and_undo_name() {
        let mut s = session();
        // The background color feeds a new stroke and color overlay.
        s.background_color = PaletteColor::new(0.25, 0.5, 0.75);
        let id = s.active_layer_id.unwrap();

        s.add_effect(LayerEffectKind::Stroke);
        let effects = effects_of(&s, id);
        assert_eq!(
            effects.color(LayerEffectKind::Stroke),
            Some(PaletteColor::new(0.25, 0.5, 0.75)),
            "a new stroke takes the background color"
        );
        assert_eq!(effects.stroke.unwrap().size, 4.0);
        assert_eq!(s.history.undo_name(), "Add Stroke");
        assert_eq!(
            s.effects_editing,
            Some(LayerEffectSelection {
                layer_id: id,
                kind: LayerEffectKind::Stroke
            })
        );

        // The other kinds keep their struct defaults. Each add cancels the
        // previous *uncommitted* edit (Swift's finishEffectsEditing(commit:
        // false) inside addEffect), so commit between adds like a user
        // pressing OK.
        s.finish_effects_editing(true);
        s.add_effect(LayerEffectKind::Shadow);
        s.finish_effects_editing(true);
        s.add_effect(LayerEffectKind::OuterGlow);
        let effects = effects_of(&s, id);
        assert_eq!(effects.shadow, Some(ShadowEffect::default()));
        assert_eq!(effects.outer_glow, Some(OuterGlowEffect::default()));
        assert_eq!(s.history.undo_name(), "Add Outer Glow");
        assert_eq!(
            s.effects_editing,
            Some(LayerEffectSelection {
                layer_id: id,
                kind: LayerEffectKind::OuterGlow
            })
        );
    }

    #[test]
    fn change_effects_stays_bound_to_the_editing_layer() {
        let mut s = session();
        let first = s.active_layer_id.unwrap();
        s.add_effect(LayerEffectKind::Stroke);
        // The selection moves to another layer; the panel stays bound.
        let second = pixel_layer(&mut s, "Second");
        s.select_layer(Some(second));
        assert_ne!(s.active_layer_id, Some(first));

        s.change_effects(|effects| {
            if let Some(stroke) = effects.stroke.as_mut() {
                stroke.size = 9.0;
            }
        });
        assert_eq!(effects_of(&s, first).stroke.unwrap().size, 9.0);
        assert!(
            effects_of(&s, second).is_empty(),
            "the other layer is untouched"
        );
        assert_eq!(s.history.undo_name(), "Edit Stroke");
    }

    #[test]
    fn cancel_restores_only_the_edited_kind() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        // A committed shadow from before the panel opened.
        s.add_effect(LayerEffectKind::Shadow);
        s.finish_effects_editing(true);
        let shadow_before = effects_of(&s, id).shadow;

        // Then a stroke is added and edited in the panel.
        s.add_effect(LayerEffectKind::Stroke);
        s.change_effects(|effects| {
            if let Some(stroke) = effects.stroke.as_mut() {
                stroke.size = 12.0;
            }
        });
        s.finish_effects_editing(false);

        let after = effects_of(&s, id);
        assert_eq!(
            after.stroke, None,
            "the stroke had no original: Cancel removes it"
        );
        assert_eq!(after.shadow, shadow_before, "the earlier shadow survives");
        assert_eq!(s.history.undo_name(), "Cancel Stroke");
        assert_eq!(s.effects_editing, None);
        assert_eq!(s.effects_editing_original, None);
    }

    #[test]
    fn canceling_a_newly_added_effect_removes_it() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        s.add_effect(LayerEffectKind::InnerGlow);
        assert!(effects_of(&s, id).contains(LayerEffectKind::InnerGlow));
        s.finish_effects_editing(false);
        assert!(effects_of(&s, id).is_empty(), "the added effect is gone");
        // Nothing stored: an empty effects set normalizes to nil.
        assert!(
            s.document
                .as_ref()
                .unwrap()
                .layer(id)
                .unwrap()
                .effects
                .is_none()
        );
        assert_eq!(s.effects_editing, None);
        assert_eq!(
            s.effect_selection, None,
            "the selection leaves with the panel"
        );
    }

    #[test]
    fn commit_keeps_the_effect_and_selection() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        s.add_effect(LayerEffectKind::Stroke);
        s.finish_effects_editing(true);
        assert!(effects_of(&s, id).contains(LayerEffectKind::Stroke));
        assert_eq!(s.effects_editing, None);
        // The effect selection stays while the layer is active and carries it.
        assert_eq!(
            s.selected_effect(),
            Some(LayerEffectSelection {
                layer_id: id,
                kind: LayerEffectKind::Stroke
            })
        );
    }

    #[test]
    fn set_effects_rejects_folders_assetless_layers_and_invalid_values() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();

        // An invalid effect (size over the 500 px cap) is refused.
        s.set_effects(
            LayerEffects {
                stroke: Some(StrokeEffect {
                    size: 600.0,
                    ..Default::default()
                }),
                ..Default::default()
            },
            Some(id),
            "Layer Effects",
        );
        assert!(effects_of(&s, id).is_empty());

        // A layer without pixels cannot take effects.
        let mut flat = ImageLayer::blank("Flat", CGSize::new(10.0, 10.0));
        flat.id = Uuid::new_v4();
        let flat_id = flat.id;
        s.document.as_mut().unwrap().layers.push(flat);
        s.set_effects(
            LayerEffects {
                shadow: Some(ShadowEffect::default()),
                ..Default::default()
            },
            Some(flat_id),
            "Layer Effects",
        );
        assert!(effects_of(&s, flat_id).is_empty());

        // Neither can a folder.
        let mut group = ImageLayer::blank("Group", CGSize::new(10.0, 10.0));
        group.id = Uuid::new_v4();
        group.is_group = true;
        let group_id = group.id;
        s.document.as_mut().unwrap().layers.push(group);
        s.set_effects(
            LayerEffects {
                shadow: Some(ShadowEffect::default()),
                ..Default::default()
            },
            Some(group_id),
            "Layer Effects",
        );
        assert!(effects_of(&s, group_id).is_empty());

        // A valid set on a pixel layer goes through.
        s.set_effects(
            LayerEffects {
                shadow: Some(ShadowEffect::default()),
                ..Default::default()
            },
            Some(id),
            "Layer Effects",
        );
        assert!(effects_of(&s, id).contains(LayerEffectKind::Shadow));
    }

    #[test]
    fn copy_effect_moves_one_kind_between_layers() {
        let mut s = session();
        let source = s.active_layer_id.unwrap();
        s.add_effect(LayerEffectKind::Stroke);
        s.change_effects(|effects| {
            if let Some(stroke) = effects.stroke.as_mut() {
                stroke.size = 7.0;
            }
        });
        let target = pixel_layer(&mut s, "Target");

        assert!(s.can_copy_effect(LayerEffectKind::Stroke, source, target));
        assert!(
            !s.can_copy_effect(LayerEffectKind::Stroke, source, source),
            "same layer"
        );
        assert!(
            !s.can_copy_effect(LayerEffectKind::Shadow, source, target),
            "the source has no shadow"
        );
        s.copy_effect(LayerEffectKind::Stroke, source, target);

        let copied = effects_of(&s, target).stroke.unwrap();
        assert_eq!(copied.size, 7.0);
        assert_eq!(s.history.undo_name(), "Copy Stroke");
        // The copy selects the effect on the target (without editing); the
        // panel stays bound to the source's stroke, which it was editing.
        assert_eq!(
            s.effect_selection,
            Some(LayerEffectSelection {
                layer_id: target,
                kind: LayerEffectKind::Stroke
            })
        );
        assert_eq!(
            s.effects_editing,
            Some(LayerEffectSelection {
                layer_id: source,
                kind: LayerEffectKind::Stroke
            })
        );
    }

    /// The effect row's eye (`toggleEffect`, LayerEffects.swift:350-355):
    /// hides and shows the kind, each in its own one-step edit, and Undo
    /// puts the previous state back.
    #[test]
    fn toggle_effect_hides_and_shows_in_one_undo_step() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        s.add_effect(LayerEffectKind::Stroke);
        s.finish_effects_editing(true);
        assert!(effects_of(&s, id).is_enabled(LayerEffectKind::Stroke));

        s.toggle_effect(LayerEffectKind::Stroke, id);
        let effects = effects_of(&s, id);
        assert!(effects.contains(LayerEffectKind::Stroke), "still carried");
        assert!(!effects.is_enabled(LayerEffectKind::Stroke));
        assert_eq!(s.history.undo_name(), "Hide Stroke");

        s.toggle_effect(LayerEffectKind::Stroke, id);
        assert!(effects_of(&s, id).is_enabled(LayerEffectKind::Stroke));
        assert_eq!(s.history.undo_name(), "Show Stroke");

        // Undo steps back over exactly one toggle: the show, then the hide.
        s.undo();
        assert!(!effects_of(&s, id).is_enabled(LayerEffectKind::Stroke));
        s.undo();
        assert!(effects_of(&s, id).is_enabled(LayerEffectKind::Stroke));

        // A layer without effects has nothing to toggle; no edit is made.
        let bare = ImageLayer::blank("Bare", CGSize::new(10.0, 10.0));
        let bare_id = bare.id;
        s.document.as_mut().unwrap().layers.push(bare);
        let name_before = s.history.undo_name().to_string();
        s.toggle_effect(LayerEffectKind::Stroke, bare_id);
        assert_eq!(
            s.history.undo_name(),
            name_before,
            "the bare layer left no step"
        );
    }
}
