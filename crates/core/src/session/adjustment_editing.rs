//! Port of `Document/AdjustmentEditing.swift` — the adjustment-layer editing
//! session: the editors bind to their adjustment layer, previews push the
//! editors' settings into it live, and OK/Cancel folds into one undo step.

use uuid::Uuid;

use super::EditorSession;

impl EditorSession {
    /// `beginAdjustmentEditing(_:)` — opens the editors on the adjustment
    /// layer `id` (the menu passes the active layer). One undo step named
    /// "Edit X Adjustment" folds every preview until
    /// `finish_adjustment_editing` closes it. Refuses while an editor is
    /// already open or the layer carries no adjustment.
    pub fn begin_adjustment_editing(&mut self, id: Uuid) -> bool {
        if self.adjustment_editing_id.is_some()
            || self.adjustment_original.is_some()
            || self.levels.is_some()
            || self.hue_saturation.is_some()
            || self.filter_edit.is_some()
        {
            return false;
        }
        let Some(adjustment) = self
            .document
            .as_ref()
            .and_then(|document| document.layer(id))
            .and_then(|layer| layer.adjustment.clone())
        else {
            return false;
        };
        self.adjustment_editing_id = Some(id);
        self.adjustment_original = Some(adjustment.clone());
        self.begin_edit(&format!("Edit {} Adjustment", adjustment.kind.label()));
        true
    }

    /// `editedAdjustment` — the opening value overlaid with the open editors'
    /// settings. None while no edit is open, or when the kind's editor is
    /// missing (Invert has none: nothing to carry back).
    pub fn edited_adjustment(&self) -> Option<crate::adjustment::LayerAdjustment> {
        use crate::adjustment::AdjustmentKind;
        let mut value = self.adjustment_original.clone()?;
        match value.kind {
            // Nothing to carry back: Invert has no settings.
            AdjustmentKind::Invert => {}
            AdjustmentKind::Levels => value.levels = self.levels?,
            AdjustmentKind::Hsv => value.hsv_settings = Some(self.hue_saturation.clone()?),
            _ => {
                let settings = self.filter_edit.as_ref()?;
                match value.kind {
                    AdjustmentKind::Exposure => value.set_exposure(settings.exposure),
                    AdjustmentKind::GradientMap => value.set_gradient_map(settings.gradient_map),
                    AdjustmentKind::Grain => value.set_grain(settings.grain),
                    AdjustmentKind::BlackWhite => value.set_black_white(settings.black_white),
                    AdjustmentKind::ColorBalance => value.set_color_balance(settings.color_balance),
                    AdjustmentKind::GaussianBlur => value.set_gaussian_radius(settings.radius),
                    AdjustmentKind::MotionBlur => {
                        value.set_motion_angle(settings.angle);
                        value.set_motion_distance(settings.distance);
                    }
                    AdjustmentKind::AddNoise => {
                        value.set_noise_amount(settings.amount);
                        value.noise_gaussian = Some(settings.gaussian);
                        value.noise_monochromatic = Some(settings.monochromatic);
                    }
                    // Curves (and any filter kind without a slot of its own).
                    _ => value.curves = settings.curves.clone(),
                }
            }
        }
        Some(value)
    }

    /// `previewAdjustmentEditing(preview:)` — pushes the editors' settings
    /// into the edited layer (or restores the opening value), which the
    /// canvas renders live. Stays bound to the layer the edit opened on,
    /// whatever the active layer is by now.
    pub fn preview_adjustment_editing(&mut self, preview: bool) -> bool {
        let Some(id) = self.adjustment_editing_id else {
            return false;
        };
        let Some(original) = self.adjustment_original.clone() else {
            return false;
        };
        let Some(value) = self.edited_adjustment() else {
            return false;
        };
        self.update_adjustment(id, if preview { value } else { original });
        true
    }

    /// `finishAdjustmentEditing(commit:)` — OK keeps the editors' settings;
    /// Cancel (including the window close button) restores the opening
    /// value. Either way the editors close and the folded undo step ends.
    pub fn finish_adjustment_editing(&mut self, commit: bool) -> bool {
        let Some(id) = self.adjustment_editing_id else {
            return false;
        };
        let Some(original) = self.adjustment_original.clone() else {
            return false;
        };
        self.update_adjustment(
            id,
            if commit {
                self.edited_adjustment().unwrap_or_else(|| original.clone())
            } else {
                original.clone()
            },
        );
        self.levels = None;
        self.hue_saturation = None;
        self.filter_edit = None;
        self.end_edit();
        self.adjustment_original = None;
        self.adjustment_editing_id = None;
        self.canvas_focus_request += 1;
        true
    }
}
