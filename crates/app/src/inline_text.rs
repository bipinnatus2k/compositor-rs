//! The inline canvas text editor (`InlineTextEditor.swift`): a text field
//! anchored over the text draft's position on the canvas, editing the
//! session's pending `TextDraft` live.
//!
//! Swift draws its own NSTextView overlay with per-pixel handles, mirror and
//! a measured point-text box. This port anchors a multi-line gpui input at
//! the draft's document position (through the same `doc_to_screen` transform
//! the canvas paints with), keeps its text two-way bound to the draft, and
//! routes Enter/Escape like the Swift editor: plain Return inserts a line
//! break (the text system's own behavior), ⌘Return commits, Escape cancels.

use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::{px, Focusable, InteractiveElement, IntoElement, ParentElement, Styled};
use gpui_kit::MouseButton;
use gpui_kit::{Context, Entity, Window};

use compositor_core::document::LayerTextStyle;
use compositor_core::geometry::{CGPoint, CGSize};
use compositor_core::transform::LayerTransform;

use crate::state::AppState;

/// The overlay input's cap, mirroring Swift's
/// `shouldChangeTextIn` guard (100,000 characters).
pub const INLINE_TEXT_MAX_CHARS: usize = 100_000;

/// The screen-space box the inline editor occupies: origin and size in
/// viewport pixels, plus the on-canvas typeface (size and color follow the
/// draft's style at the current zoom).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct InlineTextOverlay {
    pub origin: (f32, f32),
    pub size: (f32, f32),
    pub font_px: f32,
    pub color: u32,
}

/// `InlineTextEditor.synchronize`'s geometry, as a pure function. Point text
/// has no box: it measures as whatever `measured` says (the Swift editor
/// grows the layer as it types; the overlay only follows the draft's box or
/// measurement). Boxed text sits where the draft's transform puts it.
pub fn inline_text_box(
    origin: CGPoint,
    transform: Option<LayerTransform>,
    box_size: Option<CGSize>,
    measured: CGSize,
    zoom: f64,
    pan: (f64, f64),
) -> ((f32, f32), (f32, f32)) {
    let logical = box_size.unwrap_or(measured);
    let transform = transform.unwrap_or_else(|| LayerTransform::new(origin, logical));
    let (sx, sy) = crate::state::doc_to_screen_at(zoom, pan, transform.origin.x, transform.origin.y);
    (
        (sx, sy),
        (
            (transform.size.width * zoom) as f32,
            (transform.size.height * zoom) as f32,
        ),
    )
}

/// The measured logical size of point text (`EditorSession.textBoxSize`),
/// at least big enough to hold a caret.
pub fn inline_text_measured_size(style: &LayerTextStyle) -> CGSize {
    let size = compositor_core::tools::text::text_box_size(style);
    CGSize::new(size.width.max(16.0), size.height.max(16.0))
}

/// A key press while the inline editor owns the keyboard, and the action it
/// routes to (`CanvasTextView.keyDown`). Plain characters and plain Return
/// are the text system's business; ⌘Return finishes the draft and Escape
/// cancels it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum InlineTextKey {
    /// ⌘Return — `finishText`.
    Finish,
    /// Escape — `cancelText`.
    Cancel,
    /// Option-left/right — tracking step (negative to tighten).
    Tracking(f64),
    /// Option-up/down — leading step; `true` opens the lines out.
    Leading(bool, f64),
}

pub fn inline_text_key(
    key: &str,
    control: bool,
    alt: bool,
    platform: bool,
    shift: bool,
) -> Option<InlineTextKey> {
    let step = if shift { 10.0 } else { 1.0 };
    match key {
        "enter" if control && !alt => Some(InlineTextKey::Finish),
        "escape" => Some(InlineTextKey::Cancel),
        "left" if alt && !control && !platform => Some(InlineTextKey::Tracking(-step)),
        "right" if alt && !control && !platform => Some(InlineTextKey::Tracking(step)),
        "up" if alt && !control && !platform => Some(InlineTextKey::Leading(false, step)),
        "down" if alt && !control && !platform => Some(InlineTextKey::Leading(true, step)),
        _ => None,
    }
}

impl AppState {
    /// Whether the inline editor's input currently owns the keyboard.
    pub fn inline_text_focused(&self, window: &mut Window, cx: &gpui_kit::App) -> bool {
        self.current_tab().session.text_draft.is_some()
            && self
                .inline_text_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
    }

    /// Two-way binding, session → input: when a draft opens its content
    /// fills the input and the input takes focus; while it stays open an
    /// externally changed style (the Type header, color picker) refreshes
    /// the text (`synchronize`'s `shownStyle != style` half). `set_value`
    /// emits no events, so this never loops into `inline_text_changed`.
    pub fn sync_inline_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.tab().session.text_draft.clone() else {
            self.inline_text_open = false;
            self.inline_text_pushed = None;
            return;
        };
        let content = draft.style.content.clone();
        if !self.inline_text_open {
            self.inline_text_open = true;
            self.inline_text_pushed = Some(content.clone());
            self.inline_text_input.update(cx, |state, cx| {
                state.set_value(content.clone(), window, cx);
                state.focus(window, cx);
            });
            return;
        }
        if self.inline_text_pushed.as_deref() != Some(content.as_str()) {
            let differs = self.inline_text_input.read(cx).value() != content;
            if differs {
                self.inline_text_pushed = Some(content.clone());
                self.inline_text_input
                    .update(cx, |state, cx| state.set_value(content, window, cx));
            } else {
                // The input already shows the draft's text (the user typed it);
                // just remember it so the binding stays settled.
                self.inline_text_pushed = Some(content);
            }
        }
    }

    /// Two-way binding, input → session: the text as typed becomes the
    /// draft's content and the canvas re-renders (`textDidChange`).
    pub fn inline_text_changed(&mut self, text: &str, cx: &mut Context<Self>) {
        self.inline_text_pushed = Some(text.to_string());
        let Some(draft) = self.tab().session.text_draft.as_mut() else {
            return;
        };
        if text.chars().count() > INLINE_TEXT_MAX_CHARS {
            return;
        }
        if draft.style.content != text {
            draft.style.content = text.to_string();
            self.touch(cx);
        }
    }

    /// ⌘Return in the inline editor (`finishText`).
    pub fn commit_inline_text(&mut self, cx: &mut Context<Self>) {
        if self.tab().session.text_draft.is_some() {
            self.tab().session.finish_text();
            self.inline_text_open = false;
            self.touch(cx);
        }
    }

    /// Escape in the inline editor (`cancelText`): the draft is dropped and
    /// the layer keeps whatever it had.
    pub fn cancel_inline_text(&mut self, cx: &mut Context<Self>) {
        if self.tab().session.text_draft.is_some() {
            self.tab().session.cancel_text();
            self.inline_text_open = false;
            self.touch(cx);
        }
    }

    /// The overlay's geometry, if a draft is open. Pure apart from reading
    /// the session.
    pub fn inline_text_overlay(&self) -> Option<InlineTextOverlay> {
        let draft = self.current_tab().session.text_draft.as_ref()?;
        let measured = inline_text_measured_size(&draft.style);
        let ((ox, oy), (w, h)) = inline_text_box(
            draft.origin,
            draft.transform,
            draft.style.box_size,
            measured,
            self.zoom,
            self.pan,
        );
        let rgb = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
        Some(InlineTextOverlay {
            origin: (ox, oy),
            size: (w.max(24.0), h.max(24.0)),
            font_px: (draft.style.font_size * self.zoom).max(6.0) as f32,
            color: (rgb(draft.style.red) << 16) | (rgb(draft.style.green) << 8) | rgb(draft.style.blue),
        })
    }

    /// The overlay element: a bordered box over the canvas, holding the
    /// multi-line input. Mouse presses inside it stop propagating so the
    /// canvas never sees them as Type-tool clicks.
    pub fn render_inline_text_overlay(&self) -> Option<gpui_kit::AnyElement> {
        let overlay = self.inline_text_overlay()?;
        let input = self.inline_text_input.clone();
        Some(
            gpui_kit::div()
                .id("inline-text-editor")
                .absolute()
                .left(px(overlay.origin.0))
                .top(px(overlay.origin.1))
                .w(px(overlay.size.0))
                .h(px(overlay.size.1))
                .border_1()
                .border_color(gpui_kit::rgba(0x4da6ffdd))
                .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                    cx.stop_propagation();
                })
                .child(
                    Textarea::new(&input)
                        .appearance(false)
                        .bordered(false)
                        .aria_label("Canvas text")
                        .text_size(px(overlay.font_px))
                        .text_color(gpui_kit::rgb(overlay.color)),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{inline_text_box, inline_text_key, inline_text_measured_size, InlineTextKey};
    use compositor_core::document::LayerTextStyle;
    use compositor_core::geometry::{CGPoint, CGSize};
    use compositor_core::transform::LayerTransform;

    #[test]
    fn boxed_text_uses_the_draft_transform_through_the_view_transform() {
        let transform = LayerTransform::new(CGPoint::new(10.0, 20.0), CGSize::new(200.0, 80.0));
        let box_size = CGSize::new(200.0, 80.0);
        let ((ox, oy), (w, h)) = inline_text_box(
            CGPoint::new(10.0, 20.0),
            Some(transform),
            Some(box_size),
            CGSize::new(999.0, 999.0),
            2.0,
            (5.0, 7.0),
        );
        // Origin maps through doc_to_screen, size scales by zoom; the
        // measurement must not leak into a boxed draft.
        assert_eq!((ox, oy), (25.0, 47.0));
        assert_eq!((w, h), (400.0, 160.0));
    }

    #[test]
    fn point_text_falls_back_to_the_measurement() {
        let style = LayerTextStyle {
            content: "Hi".into(),
            font_size: 72.0,
            ..LayerTextStyle::default()
        };
        let measured = inline_text_measured_size(&style);
        assert!(measured.width >= 16.0 && measured.height >= 16.0);
        let ((ox, oy), (w, h)) = inline_text_box(
            CGPoint::new(4.0, 6.0),
            None,
            None,
            measured,
            1.0,
            (0.0, 0.0),
        );
        assert_eq!((ox, oy), (4.0, 6.0));
        assert_eq!((w, h), (measured.width as f32, measured.height as f32));
    }

    #[test]
    fn point_text_with_a_layer_transform_scales_from_the_layer() {
        // Editing existing point text: the draft carries the layer's
        // transform; the overlay follows it, not the raw measurement.
        let transform = LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(300.0, 120.0));
        let ((ox, oy), (w, h)) = inline_text_box(
            CGPoint::new(0.0, 0.0),
            Some(transform),
            None,
            CGSize::new(60.0, 24.0),
            0.5,
            (0.0, 0.0),
        );
        assert_eq!((ox, oy), (0.0, 0.0));
        assert_eq!((w, h), (150.0, 60.0));
    }

    #[test]
    fn inline_keys_route_finish_cancel_and_spacing() {
        // ⌘Return finishes (⌘ maps to Ctrl on Windows).
        assert_eq!(
            inline_text_key("enter", true, false, false, false),
            Some(InlineTextKey::Finish)
        );
        // Plain Return is the text system's newline, not a route.
        assert_eq!(inline_text_key("enter", false, false, false, false), None);
        // Escape cancels.
        assert_eq!(
            inline_text_key("escape", false, false, false, false),
            Some(InlineTextKey::Cancel)
        );
        // Option-arrows step the spacing; Shift makes each step ten.
        assert_eq!(
            inline_text_key("right", false, true, false, false),
            Some(InlineTextKey::Tracking(1.0))
        );
        assert_eq!(
            inline_text_key("left", false, true, false, true),
            Some(InlineTextKey::Tracking(-10.0))
        );
        assert_eq!(
            inline_text_key("up", false, true, false, false),
            Some(InlineTextKey::Leading(false, 1.0))
        );
        assert_eq!(
            inline_text_key("down", false, true, false, true),
            Some(InlineTextKey::Leading(true, 10.0))
        );
        // Plain arrows move the caret instead.
        assert_eq!(inline_text_key("left", false, false, false, false), None);
    }
}
