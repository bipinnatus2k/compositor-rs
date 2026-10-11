//! Port of the session's brush-side tool state: `CloneStamp.swift` and the
//! fills & palette extension (`ColorPalette.swift`).

use super::*;

/// `lastBrushPoint` (EditorSession+Brush.swift:61) — where the last stroke
/// ended and on which layer/mask target it painted.
pub struct LastBrushPoint {
    pub point: CGPoint,
    pub layer_id: Uuid,
    pub mask: bool,
}

impl EditorSession {
    // MARK: Clone Stamp (CloneStamp.swift)

    /// `cloneStrokeOffset(at:)` — the whole-pixel offset a stroke starting at
    /// `point` would copy with: aligned strokes keep the first stroke's
    /// (`cloneOffset`); otherwise it runs from the brush to the source.
    pub fn clone_stroke_offset(&self, at: CGPoint) -> Option<CGSize> {
        let source = self.clone_source?;
        Some(match self.clone_settings.aligned {
            true => self.clone_offset.unwrap_or_else(|| {
                CGSize::new((source.x - at.x).round(), (source.y - at.y).round())
            }),
            false => CGSize::new((source.x - at.x).round(), (source.y - at.y).round()),
        })
    }

    /// A stroke starting at `point`: with aligned on, the first stroke fixes
    /// `cloneOffset` and later strokes reuse it.
    pub fn begin_clone_stroke(&mut self, at: CGPoint) -> Option<CGSize> {
        let offset = self.clone_stroke_offset(at)?;
        if self.clone_settings.aligned && self.clone_offset.is_none() {
            self.clone_offset = Some(offset);
        }
        Some(offset)
    }

    // MARK: Fills & palette

    pub fn palette_color(&self, background: bool) -> PaletteColor {
        if self.is_mask_selected {
            let white = if background {
                !self.mask_paint_white
            } else {
                self.mask_paint_white
            };
            if white {
                PaletteColor::WHITE
            } else {
                PaletteColor::BLACK
            }
        } else if background {
            self.background_color
        } else {
            PaletteColor::new(
                self.brush_settings.red,
                self.brush_settings.green,
                self.brush_settings.blue,
            )
        }
    }

    /// `swapPaletteColors` (ColorPalette.swift:45).
    pub fn swap_palette_colors(&mut self) {
        if !self.can_edit_palette() {
            return;
        }
        if self.is_mask_selected {
            self.mask_paint_white = !self.mask_paint_white;
        } else {
            let old = self.palette_color(false);
            let background = self.background_color;
            // Swift routes through `setPaletteColor`, so the Type tool's open
            // draft follows the swatch (:49-51).
            self.set_palette_color(background, false);
            self.background_color = old;
        }
    }

    /// `resetPaletteColors` (ColorPalette.swift:54).
    pub fn reset_palette_colors(&mut self) {
        if !self.can_edit_palette() {
            return;
        }
        if self.is_mask_selected {
            self.mask_paint_white = false;
        } else {
            self.set_palette_color(PaletteColor::BLACK, false);
            self.background_color = PaletteColor::WHITE;
        }
    }

    /// `shiftLineStart` (EditorSession+Brush.swift:89) — where a Shift-click
    /// paints a line from: the end of the last stroke, while the same layer
    /// (or mask) is the target.
    pub fn shift_line_start(&self) -> Option<CGPoint> {
        let last = self.last_brush_point.as_ref()?;
        if last.layer_id == self.active_layer_id? && last.mask == self.is_mask_selected {
            Some(last.point)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> EditorSession {
        let mut s = EditorSession::new();
        s.create_document(100, 100, true);
        s
    }

    // canEditPalette guards (ColorPalette.swift:25)

    #[test]
    fn swap_palette_colors_refuses_while_a_stroke_is_in_flight() {
        let mut s = session();
        s.set_palette_color(PaletteColor::new(1.0, 0.0, 0.0), false);
        s.has_brush_stroke = true;
        s.swap_palette_colors();
        assert_eq!(s.brush_settings.red, 1.0);
        assert_eq!(s.background_color, PaletteColor::WHITE);
    }

    #[test]
    fn reset_palette_colors_refuses_while_a_project_is_busy() {
        let mut s = session();
        s.set_palette_color(PaletteColor::new(1.0, 0.0, 0.0), false);
        s.is_project_busy = true;
        s.reset_palette_colors();
        assert_eq!(s.brush_settings.red, 1.0);
        assert_eq!(s.background_color, PaletteColor::WHITE);
    }

    #[test]
    fn swap_and_reset_follow_the_open_text_draft() {
        // Swift routes through `setPaletteColor`, so a Type draft follows the
        // swatch (ColorPalette.swift:49-51, :57).
        let mut s = session();
        s.begin_text(CGPoint::new(10.0, 10.0), true);
        s.set_palette_color(PaletteColor::new(0.0, 0.0, 1.0), false);
        s.swap_palette_colors();
        // The draft followed the swatch to the old background (white) and the
        // foreground became the old background too.
        assert_eq!(s.text_draft.as_ref().unwrap().style.red, 1.0);
        assert_eq!(s.brush_settings.red, 1.0);
        assert_eq!(s.background_color, PaletteColor::new(0.0, 0.0, 1.0));
    }

    // shiftLineStart (EditorSession+Brush.swift:89)

    #[test]
    fn shift_line_start_follows_the_last_stroke_on_the_same_target() {
        let mut s = session();
        let layer_id = s.active_layer_id.unwrap();
        s.last_brush_point = Some(LastBrushPoint {
            point: CGPoint::new(12.0, 34.0),
            layer_id,
            mask: false,
        });
        assert_eq!(s.shift_line_start(), Some(CGPoint::new(12.0, 34.0)));
        // A different target (the mask) is not the same place to draw from.
        s.last_brush_point.as_mut().unwrap().mask = true;
        assert_eq!(s.shift_line_start(), None);
    }

    #[test]
    fn shift_line_start_needs_the_strokes_layer_still_active() {
        let mut s = session();
        s.last_brush_point = Some(LastBrushPoint {
            point: CGPoint::new(1.0, 1.0),
            layer_id: Uuid::new_v4(),
            mask: false,
        });
        assert_eq!(s.shift_line_start(), None);
    }
}
