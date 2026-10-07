//! Port of the session's brush-side tool state: `CloneStamp.swift` and the
//! fills & palette extension (`ColorPalette.swift`).

use super::*;

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

    pub fn swap_palette_colors(&mut self) {
        if self.is_mask_selected {
            self.mask_paint_white = !self.mask_paint_white;
        } else {
            let old = self.palette_color(false);
            let background = self.background_color;
            self.brush_settings.red = background.red;
            self.brush_settings.green = background.green;
            self.brush_settings.blue = background.blue;
            self.background_color = old;
        }
    }

    pub fn reset_palette_colors(&mut self) {
        if self.is_mask_selected {
            self.mask_paint_white = false;
        } else {
            self.brush_settings.red = 0.0;
            self.brush_settings.green = 0.0;
            self.brush_settings.blue = 0.0;
            self.background_color = PaletteColor::WHITE;
        }
    }
}
