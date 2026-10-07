//! Port of the session's Gradient tool extension (`Gradient.swift`).

use super::*;

impl EditorSession {
    // MARK: Gradient (Gradient.swift)

    /// `beginGradient(at:)` — dragging a new line replaces the pending one on
    /// the same target. Swift's first guard is `tool == .gradient`
    /// (Gradient.swift:37).
    pub fn begin_gradient(&mut self, at: CGPoint) {
        if self.tool != NavigationTool::Gradient || !self.can_edit_pixels() {
            return;
        }
        let Some(id) = self.active_layer_id else {
            return;
        };
        if self.document.as_ref().and_then(|d| d.layer(id)).is_none() {
            return;
        }
        let is_mask = self.is_mask_selected;
        if let Some(draft) = self.gradient_edit.as_mut()
            && draft.layer_id == id
            && draft.is_mask == is_mask
        {
            draft.start = at;
            draft.end = at;
            return;
        }
        self.finish_opacity_edit();
        self.gradient_edit = Some(GradientDraft {
            layer_id: id,
            is_mask,
            start: at,
            end: at,
        });
    }

    pub fn move_gradient(&mut self, start: Option<CGPoint>, end: Option<CGPoint>) {
        if let Some(draft) = self.gradient_edit.as_mut() {
            if let Some(start) = start {
                draft.start = start;
            }
            if let Some(end) = end {
                draft.end = end;
            }
        }
    }

    /// `refreshGradient` — re-renders the pending gradient from the current
    /// endpoints, settings, and palette. `hasLine` is the plain euclidean
    /// length (Gradient.swift:32) — no extra square root.
    pub fn refresh_gradient(&self) -> Option<Bitmap> {
        let draft = self.gradient_edit.as_ref()?;
        let has_line = (draft.end.x - draft.start.x).hypot(draft.end.y - draft.start.y) >= 0.5;
        if !has_line {
            return None;
        }
        let Some(document) = self.document.as_ref() else {
            return None;
        };
        let edit = self.raster_edit_for_layer(document, draft.layer_id, draft.is_mask)?;
        let (fg, bg, transparent) = self.gradient_colors(draft.is_mask);
        let mut colors = if self.gradient_settings.foreground_to_background {
            [fg, bg]
        } else {
            [fg, transparent]
        };
        if self.gradient_settings.reversed {
            colors = [colors[1], colors[0]];
        }
        Some(edit.fill_gradient(
            self.gradient_settings.shape,
            draft.start,
            draft.end,
            colors,
            self.gradient_settings.opacity,
        ))
    }

    /// `gradientColors` — the two stop colors; on a mask the palette is gray.
    fn gradient_colors(&self, is_mask: bool) -> ([u8; 4], [u8; 4], [u8; 4]) {
        let value = |c: f64| (c * 255.0).round() as u8;
        let fg = self.palette_color(false);
        let bg = self.palette_color(true);
        if is_mask {
            let fg_gray = [value(fg.red), value(fg.red), value(fg.red), 255];
            let bg_gray = [value(bg.red), value(bg.red), value(bg.red), 255];
            (fg_gray, bg_gray, [0, 0, 0, 0])
        } else {
            (
                [value(fg.red), value(fg.green), value(fg.blue), 255],
                [value(bg.red), value(bg.green), value(bg.blue), 255],
                [value(fg.red), value(fg.green), value(fg.blue), 0],
            )
        }
    }

    /// `commitGradient` — the pending gradient rasterizes into the layer (or
    /// mask) its draft captured (Gradient.swift:88-100 commits `edit.raster`,
    /// which is bound to the layer from begin time), as one undo step. A click
    /// without a line just cancels. `hasLine` is the plain euclidean length.
    pub fn apply_gradient(&mut self) -> bool {
        let Some(draft) = self.gradient_edit.clone() else {
            return false;
        };
        self.gradient_edit = None;
        if !self.can_edit_pixels() {
            self.gradient_edit = Some(draft);
            return false;
        }
        let has_line = (draft.end.x - draft.start.x).hypot(draft.end.y - draft.start.y) >= 0.5;
        if !has_line {
            return true;
        }
        let Some(document) = self.document.clone() else {
            return false;
        };
        let Some(edit) = self.raster_edit_for_layer(&document, draft.layer_id, draft.is_mask)
        else {
            return false;
        };
        let (fg, bg, transparent) = self.gradient_colors(edit.is_mask);
        let mut colors = if self.gradient_settings.foreground_to_background {
            [fg, bg]
        } else {
            [fg, transparent]
        };
        if self.gradient_settings.reversed {
            colors = [colors[1], colors[0]];
        }
        let composed = edit.fill_gradient(
            self.gradient_settings.shape,
            draft.start,
            draft.end,
            colors,
            self.gradient_settings.opacity,
        );
        self.commit_pixel_edit_for_layer(
            draft.layer_id,
            &edit,
            &composed,
            if edit.is_mask {
                "Gradient Mask"
            } else {
                "Gradient"
            },
        );
        true
    }

    /// `cancelGradient`.
    pub fn cancel_gradient(&mut self) {
        self.gradient_edit = None;
    }

    /// `resolveGradient` — switching tools, layers, or targets applies the
    /// pending gradient.
    pub fn resolve_gradient(&mut self) {
        if self.gradient_edit.is_some() {
            self.apply_gradient();
        }
    }
}
