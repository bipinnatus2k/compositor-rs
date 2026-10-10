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
//!
//! Two typeface metrics the committed raster carries have no editing-time
//! counterpart in gpui's text stack:
//!
//! - **Tracking is not shown while editing.** The core steps every glyph by
//!   `advance + tracking` (tools/text.rs:287), but gpui's `TextStyle` has no
//!   letter-spacing knob and its shaping (`TextSystem::shape_line`) takes no
//!   tracking argument, so the overlay lays the text out at zero tracking.
//!   Per-character drawing was weighed and rejected: the input engine paints
//!   its own caret and selection quads at its zero-tracking positions and
//!   neither can be hidden through the public API, so self-drawn glyphs would
//!   sit next to an engine caret that drifts a full `step` per character.
//!   The live feedback is instead the **measurement** the commit itself uses
//!   (`EditorSession.textBoxSize`): the overlay recomputes its frame from the
//!   draft every frame, so Option-arrow tracking steps move the editor box
//!   (and flip the boxed text's overflow plus marker) immediately, at exactly
//!   the box the commit will rasterize; the glyphs spread only at commit.
//! - **Alignment** (`LayerTextStyle.alignment`) is likewise a commit-time
//!   layout. The engine knows how to *paint* centered or right-aligned lines
//!   (`Line::paint` takes a `TextAlign`), but the only setter,
//!   `InputBaseState::set_text_align` (gpui-base input/base/state.rs:637),
//!   early-returns for multi-line inputs, and this overlay's `TextareaState`
//!   is one — the styled `.text_align()` the component extracts reaches that
//!   gate and stops. The overlay stays left-aligned; the draft carries the
//!   alignment and the commit lays it out.

/// A point in viewport pixels — where the editor box sits on screen.
///
/// The overlay's own geometry type: core's `CGPoint` is document space (f64),
/// this is what the canvas paints at, after zoom and pan.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct ScreenPoint {
    pub x: f32,
    pub y: f32,
}

impl ScreenPoint {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A size in viewport pixels — the editor box's on-screen extent, or an
/// inset measured along each axis.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct ScreenSize {
    pub width: f32,
    pub height: f32,
}

impl ScreenSize {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

use gpui_kit::MouseButton;
use gpui_kit::component::input::Textarea;
use gpui_kit::{Context, Window};
use gpui_kit::{
    Focusable, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, px, relative,
};

use compositor_core::canvas_viewport::CanvasViewport;
use compositor_core::document::LayerTextStyle;
use compositor_core::geometry::{CGPoint, CGSize};
use compositor_core::transform::{HANDLES, LayerTransform};

use crate::canvas::editor_canvas::{CanvasDrag, EditorCanvasView};

/// The overlay input's cap, mirroring Swift's
/// `shouldChangeTextIn` guard (100,000 characters).
pub const INLINE_TEXT_MAX_CHARS: usize = 100_000;

/// The screen-space box the inline editor occupies: origin and size in
/// viewport pixels, plus the on-canvas typeface (size and color follow the
/// draft's style at the current zoom). `inset` is the text content's inset
/// from the box in viewport pixels (`LayerTextStyle.padding` scaled the way
/// the box itself is scaled) and `line_ratio` is the overlay's line stride
/// as a fraction of the font size, so the rows land where the commit's do.
#[derive(Clone, PartialEq, Debug)]
pub struct InlineTextOverlay {
    pub origin: ScreenPoint,
    pub size: ScreenSize,
    pub inset: ScreenSize,
    pub line_ratio: f32,
    pub font_px: f32,
    pub color: u32,
    pub font_family: String,
}

/// The transform the editor actually shows (`synchronize`'s shownTransform,
/// InlineTextEditor.swift:107-118). Boxed text sits where the draft's
/// transform puts it. Point text with no layer transform is a fresh box at
/// the draft's origin measuring whatever `measured` says. Point text already
/// on a layer grows as it is typed, keeping whatever scale the layer was
/// given: the new size is the fresh measurement times the layer's own scale
/// (`factor = transform.size.width / asset.image.width`), and because a
/// rotated layer turns about its center, growing it would swing its corner
/// away — the top-left corner is put back where it was, which is where the
/// commit leaves it too (`applyText` scales the raster against the old asset
/// with the same anchor correction, TypeTool.swift:118-127). The draft's own
/// transform is never rewritten while typing: the factor is measured against
/// the layer's original raster, so updating it every keystroke would
/// compound the scale. `asset_width` is the draft layer's raster width, 0
/// when there is no layer or asset (no growth).
pub fn inline_text_shown_transform(
    origin: CGPoint,
    transform: Option<LayerTransform>,
    box_size: Option<CGSize>,
    measured: CGSize,
    asset_width: f64,
) -> LayerTransform {
    let logical = box_size.unwrap_or(measured);
    let grew_from_layer = box_size.is_none() && transform.is_some() && asset_width > 0.0;
    let mut transform = transform.unwrap_or_else(|| LayerTransform::new(origin, logical));
    if grew_from_layer {
        let factor = transform.size.width / asset_width;
        let anchor = transform.point(CGPoint::ZERO);
        transform.size = CGSize::new(logical.width * factor, logical.height * factor);
        let moved = transform.point(CGPoint::ZERO);
        transform.origin.x += anchor.x - moved.x;
        transform.origin.y += anchor.y - moved.y;
    }
    transform
}

/// `InlineTextEditor.synchronize`'s geometry, as a pure function: the shown
/// transform mapped through the view transform (origin through
/// `viewport.viewPoint`, size scaled by the points-per-pixel ratio).
pub fn inline_text_box(
    origin: CGPoint,
    transform: Option<LayerTransform>,
    box_size: Option<CGSize>,
    measured: CGSize,
    asset_width: f64,
    viewport: CanvasViewport,
    document_size: CGSize,
) -> ((f32, f32), (f32, f32)) {
    let transform = inline_text_shown_transform(origin, transform, box_size, measured, asset_width);
    let shown = viewport.view_point(transform.origin, document_size);
    let per_pixel = viewport.points_per_pixel();
    (
        (shown.x as f32, shown.y as f32),
        (
            (transform.size.width * per_pixel) as f32,
            (transform.size.height * per_pixel) as f32,
        ),
    )
}

/// The measured logical size of point text (`EditorSession.textBoxSize`),
/// at least big enough to hold a caret.
pub fn inline_text_measured_size(style: &LayerTextStyle) -> CGSize {
    let size = compositor_core::tools::text::text_box_size(style);
    CGSize::new(size.width.max(16.0), size.height.max(16.0))
}

/// The text content's inset from the editor box, in viewport pixels. The
/// committed raster starts its glyphs `LayerTextStyle.padding` layer pixels
/// from the box's edges (first baseline at `PADDING + ascent`,
/// tools/text.rs:255, pen at `PADDING`, :259), and Swift insets its text
/// view by that same padding inside the logical box
/// (InlineTextEditor.swift:137-138). The inset here is that padding scaled
/// the way the box itself is scaled onto the screen — zoom times whatever
/// scale the layer's transform carries — so the editing glyphs start where
/// the commit would put them. The scale reads off the box's screen size over
/// its logical size; for a rotated box that is the axis-aligned bounds'
/// scale, matching the axis-aligned text this port draws (see the module
/// note on `render_inline_text_overlay`). A degenerate logical size falls
/// back to the plain zoom.
pub fn inline_text_content_inset(box_screen: ScreenSize, logical: CGSize, zoom: f64) -> ScreenSize {
    let scale_x = if logical.width > 0.0 {
        box_screen.width as f64 / logical.width
    } else {
        zoom
    };
    let scale_y = if logical.height > 0.0 {
        box_screen.height as f64 / logical.height
    } else {
        zoom
    };
    ScreenSize::new(
        (LayerTextStyle::PADDING * scale_x) as f32,
        (LayerTextStyle::PADDING * scale_y) as f32,
    )
}

/// The overlay's line stride as a fraction of the font size. Core's rows
/// advance by `style.line_height()` layer pixels per line — the style's
/// explicit leading, else 120% of the size (document.rs:408-418) — so the
/// fraction reproduces either case, and through gpui's `relative()` line
/// height it stays correct whatever the on-screen font size is. (gpui rounds
/// the stride to whole pixels, so sub-pixel drift can accumulate across many
/// rows; the core raster itself stays exact.)
pub fn inline_text_line_ratio(style: &LayerTextStyle) -> f32 {
    (style.line_height() / style.font_size) as f32
}

/// The word-wrapped line widths of the content inside `container_width`,
/// mirroring core's `layout_lines` (tools/text.rs:140): by-word wrapping, a
/// line breaks at the last space that fits, measured with the style's font,
/// size and tracking. Core keeps this layout private, so the app repeats it
/// here — the one algorithm deliberately duplicated (see the module note).
fn wrapped_line_widths(style: &LayerTextStyle, container_width: f64) -> Vec<f64> {
    let font = compositor_core::tools::text::load_font(&style.font_name);
    let size = style.font_size as f32;
    let advance_of = |text: &str| -> f64 {
        match &font {
            Some(font) => {
                let mut width = 0.0;
                for ch in text.chars() {
                    width += font.metrics(ch, size).advance_width as f64 + style.tracking;
                }
                width
            }
            None => text.chars().count() as f64 * style.font_size * 0.6,
        }
    };
    let mut widths = Vec::new();
    for paragraph in style.content.split('\n') {
        let mut current = String::new();
        let mut current_width = 0.0;
        for word in paragraph.split_inclusive(' ') {
            let word_width = advance_of(word.trim_end_matches(' '))
                + advance_of(" ") * word.matches(' ').count() as f64;
            if !current.is_empty() && current_width + word_width > container_width {
                widths.push(current_width);
                current = word.trim_start().to_string();
                current_width = advance_of(current.trim_start());
            } else {
                current.push_str(word);
                current_width += word_width;
            }
        }
        widths.push(current_width);
    }
    if widths.is_empty() {
        widths.push(0.0);
    }
    widths
}

/// Whether the text does not fit its fixed box — `draw`'s overflow marker,
/// InlineTextEditor.swift:311-326: glyphs the layout cannot place inside the
/// text container are clipped (`NSMaxRange(glyphRange) < numberOfGlyphs`),
/// and a black plus is drawn on the bottom-right handle, as in Photoshop.
/// The container is the box inset by the style's padding; it overflows when
/// the wrapped lines' height passes the container's height or when a line is
/// wider than it (an unbreakable word). Point text grows with its content,
/// so it never overflows — false without a box, like the Swift editor whose
/// point-text container always matches the measurement.
pub fn inline_text_overflows(style: &LayerTextStyle) -> bool {
    let Some(box_size) = style.box_size else {
        return false;
    };
    let container_width = box_size.width - 2.0 * LayerTextStyle::PADDING;
    let container_height = box_size.height - 2.0 * LayerTextStyle::PADDING;
    if container_width < 1.0 || container_height < 1.0 {
        return true;
    }
    let widths = wrapped_line_widths(style, container_width);
    let total_height = widths.len() as f64 * style.line_height();
    total_height > container_height || widths.iter().any(|width| *width > container_width)
}

/// The editor box's four screen corners (TL, TR, BR, BL in `HANDLES` corner
/// order) after rotating the box about its center by `rotation_degrees`.
/// Swift rotates the whole editor with `frameRotation = transform.rotation`,
/// pinning the rotated top-left corner to the frame's anchor; because the
/// screen transform is zoom and pan only, that is exactly this rotation of
/// the axis-aligned frame about its center (InlineTextEditor.swift:124-134).
pub fn inline_text_box_corners(
    origin: (f32, f32),
    size: (f32, f32),
    rotation_degrees: f64,
) -> [(f32, f32); 4] {
    let radians = (rotation_degrees % 360.0) * std::f64::consts::PI / 180.0;
    let (sr, cr) = radians.sin_cos();
    let (sr, cr) = (sr as f32, cr as f32);
    let (cx, cy) = (origin.0 + size.0 / 2.0, origin.1 + size.1 / 2.0);
    let (hw, hh) = (size.0 / 2.0, size.1 / 2.0);
    // Clockwise on the y-down screen, like a layer's own rotation.
    let corner = |ox: f32, oy: f32| (cx + ox * cr - oy * sr, cy + ox * sr + oy * cr);
    [
        corner(-hw, -hh),
        corner(hw, -hh),
        corner(hw, hh),
        corner(-hw, hh),
    ]
}

/// A press's point in the editor's own logical units: out of the document,
/// undoing the transform's rotation about the box center, then from the box's
/// placement pixels into the logical size the text is laid out at
/// (`convert(event.locationInWindow, from: nil)` through `frameRotation`).
pub fn inline_text_local_point(
    transform: &LayerTransform,
    logical: CGSize,
    doc: CGPoint,
) -> CGPoint {
    let center = transform.center();
    let dx = doc.x - center.x;
    let dy = doc.y - center.y;
    let (sr, cr) = transform.radians().sin_cos();
    // The inverse of the transform's clockwise rotation.
    let ux = dx * cr + dy * sr;
    let uy = -dx * sr + dy * cr;
    CGPoint::new(
        (ux + transform.size.width / 2.0) * logical.width / transform.size.width,
        (uy + transform.size.height / 2.0) * logical.height / transform.size.height,
    )
}

/// The resize handle's size in the box's own logical units
/// (`handleSize = max(2, 6 / max(0.01, scale * transform.width / logical.width))`,
/// InlineTextEditor.swift:141) — 6 screen points divided by the screen
/// pixels per logical pixel.
pub fn inline_text_handle_size(transform: &LayerTransform, logical: CGSize, zoom: f64) -> f64 {
    (6.0 / (zoom * transform.size.width / logical.width).max(0.01)).max(2.0)
}

/// Which resize handle is at a point in the box's own logical units, in
/// `HANDLES` order (0, 2, 4, 6 the corners; 1, 3, 5, 7 the edge midpoints).
/// A band `edgeReach` wide along each edge counts as that edge
/// (`handle(at:)`, InlineTextEditor.swift:276-293), capped so a small box
/// keeps a middle to type in; nil anywhere else, which is the text.
pub fn inline_text_handle_at(local: CGPoint, bounds: CGSize, handle_size: f64) -> Option<usize> {
    let reach = (handle_size * 10.0 / 6.0).min(bounds.width.min(bounds.height) / 3.0);
    if !(local.x >= -reach
        && local.x <= bounds.width + reach
        && local.y >= -reach
        && local.y <= bounds.height + reach)
    {
        return None;
    }
    let left = local.x <= reach;
    let right = local.x >= bounds.width - reach;
    let top = local.y <= reach;
    let bottom = local.y >= bounds.height - reach;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(0),
        (_, true, true, _) => Some(2),
        (_, true, _, true) => Some(4),
        (true, _, _, true) => Some(6),
        (_, _, true, _) => Some(1),
        (_, true, _, _) => Some(3),
        (_, _, _, true) => Some(5),
        (true, _, _, _) => Some(7),
        _ => None,
    }
}

/// What dragging a handle leaves behind: the draft's new `boxSize` and the
/// transform that keeps the dragged edge anchored.
pub struct InlineTextResize {
    pub box_size: CGSize,
    pub transform: LayerTransform,
}

/// The drag's resize math (`mouseDragged`, InlineTextEditor.swift:344-375):
/// the pointer's document-space move is rotated into the box's own axes, the
/// dragged edges follow it clamped to a 16-logical-px minimum, and the new
/// box size (rounded) scales the transform so the opposite edge stays put.
/// None when the result would leave an invalid box or transform.
pub fn inline_text_resize(
    old: &LayerTransform,
    source: CGSize,
    handle: usize,
    start: CGPoint,
    current: CGPoint,
) -> Option<InlineTextResize> {
    let unit = HANDLES.get(handle)?;
    let (sr, cr) = old.radians().sin_cos();
    let dx = current.x - start.x;
    let dy = current.y - start.y;
    let local_x = dx * cr + dy * sr;
    let local_y = -dx * sr + dy * cr;
    let (mut left, mut top) = (0.0f64, 0.0f64);
    let (mut right, mut bottom) = (old.size.width, old.size.height);
    let (min_w, min_h) = (
        16.0 * old.size.width / source.width,
        16.0 * old.size.height / source.height,
    );
    if unit.x == 0.0 {
        left = local_x.min(right - min_w);
    }
    if unit.x == 1.0 {
        right = (left + min_w).max(right + local_x);
    }
    if unit.y == 0.0 {
        top = local_y.min(bottom - min_h);
    }
    if unit.y == 1.0 {
        bottom = (top + min_h).max(bottom + local_y);
    }
    let box_size = CGSize::new(
        ((right - left) * source.width / old.size.width).round(),
        ((bottom - top) * source.height / old.size.height).round(),
    );
    // `draft.style.boxIsValid` (LayerTextStyle.boxIsValid).
    let probe = LayerTextStyle {
        box_size: Some(box_size),
        ..LayerTextStyle::default()
    };
    if !probe.box_is_valid() {
        return None;
    }
    let mut transform = *old;
    transform.size = CGSize::new(
        box_size.width * old.size.width / source.width,
        box_size.height * old.size.height / source.height,
    );
    let anchor = old.point(CGPoint::new(left / old.size.width, top / old.size.height));
    let moved = transform.point(CGPoint::ZERO);
    transform.origin.x += anchor.x - moved.x;
    transform.origin.y += anchor.y - moved.y;
    if !transform.is_valid() {
        return None;
    }
    Some(InlineTextResize {
        box_size,
        transform,
    })
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

/// The inline editor's methods live on the canvas view (ticket 39): Swift's
/// `InlineTextEditor` is a child NSView of `CanvasView`, and the port's
/// overlay state (the input entity, the two-way binding bookkeeping, the
/// caret phase) is canvas state. The methods that report through shell-side
/// pipelines (the text-style edits) route through the canvas's `app` seam.
impl EditorCanvasView {
    /// The draft's layer raster width (`layer?.asset.image.width`), 0 when
    /// the draft has no layer or asset yet — the scale point-text growth
    /// measures against.
    pub(crate) fn inline_text_asset_width(
        &self,
        draft: &compositor_core::session::TextDraft,
        cx: &gpui_kit::App,
    ) -> f64 {
        draft
            .layer_id
            .and_then(|id| self.session.read(cx).document.as_ref()?.layer(id).cloned())
            .and_then(|layer| layer.asset)
            .map(|asset| asset.image.width as f64)
            .unwrap_or(0.0)
    }

    /// The editor's shown geometry for a draft (`synchronize`): the logical
    /// size the text lays out at and the transform displayed — for point text
    /// on a layer, the growth-compensated one (`inline_text_shown_transform`).
    fn inline_text_shown(
        &self,
        draft: &compositor_core::session::TextDraft,
        cx: &gpui_kit::App,
    ) -> (CGSize, LayerTransform) {
        let measured = inline_text_measured_size(&draft.style);
        let logical = draft.style.box_size.unwrap_or(measured);
        let asset_width = self.inline_text_asset_width(draft, cx);
        let transform = inline_text_shown_transform(
            draft.origin,
            draft.transform,
            draft.style.box_size,
            measured,
            asset_width,
        );
        (logical, transform)
    }

    /// Whether the inline editor's input currently owns the keyboard.
    pub(crate) fn inline_text_focused(&self, window: &mut Window, cx: &gpui_kit::App) -> bool {
        self.session.read(cx).text_draft.is_some()
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
    pub(crate) fn sync_inline_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.session.read(cx).text_draft.clone() else {
            self.inline_text_open = false;
            self.inline_text_pushed = None;
            return;
        };
        let content = draft.style.content.clone();
        if !self.inline_text_open {
            self.inline_text_open = true;
            self.inline_text_pushed = Some(content.clone());
            self.inline_text_caret_phase = std::time::Instant::now();
            self.inline_text_input.update(cx, |state, cx| {
                state.set_value(content.clone(), window, cx);
                state.focus(window, cx);
            });
            self.spawn_caret_blink(cx);
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
    pub(crate) fn inline_text_changed(&mut self, text: &str, cx: &mut Context<Self>) {
        self.inline_text_pushed = Some(text.to_string());
        self.inline_text_caret_phase = std::time::Instant::now();
        let changed = self.session.update(cx, |session, _| {
            let Some(draft) = session.text_draft.as_mut() else {
                return false;
            };
            if text.chars().count() > INLINE_TEXT_MAX_CHARS {
                return false;
            }
            if draft.style.content != text {
                draft.style.content = text.to_string();
                // Point text's shown box grows with the measurement on the
                // next frame (`inline_text_shown_transform`); the draft's own
                // transform stays as the layer left it — the commit scales
                // the fresh raster against the old asset (`apply_text`),
                // exactly as Swift's `textDidChange` only refreshes the
                // display.
                true
            } else {
                false
            }
        });
        if changed {
            self.touch(cx);
        }
    }

    /// ⌘Return in the inline editor (`finishText`).
    pub(crate) fn commit_inline_text(&mut self, cx: &mut Context<Self>) {
        if self.session.read(cx).text_draft.is_some() {
            self.session.update(cx, |session, _| session.finish_text());
            self.inline_text_open = false;
            self.touch(cx);
        }
    }

    /// Keeps the inline caret blinking while its editor is open: a background
    /// loop wakes at the blink cadence and nudges a redraw (`caretBlinkTimer`).
    fn spawn_caret_blink(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                smol::Timer::after(std::time::Duration::from_millis(530)).await;
                let ok = this
                    .update(cx, |this, cx| {
                        if !this.inline_text_open {
                            return false;
                        }
                        this.inline_text_caret_phase = std::time::Instant::now();
                        cx.notify();
                        true
                    })
                    .ok();
                if ok == Some(false) {
                    break;
                }
            }
        })
        .detach();
    }

    /// The inline caret paints on the first half of the blink cycle and on
    /// every keystroke (the phase resets with each change).
    fn inline_text_caret_visible(&self) -> bool {
        if !self.inline_text_open {
            return false;
        }
        self.inline_text_caret_phase.elapsed().as_millis() % 1060 < 530
    }

    /// Escape in the inline editor (`cancelText`): the draft is dropped and
    /// the layer keeps whatever it had.
    pub(crate) fn cancel_inline_text(&mut self, cx: &mut Context<Self>) {
        if self.session.read(cx).text_draft.is_some() {
            self.session.update(cx, |session, _| session.cancel_text());
            self.inline_text_open = false;
            self.touch(cx);
        }
    }

    /// The overlay's geometry, if a draft is open. The input div covers the
    /// axis-aligned bounds of the (possibly rotated) frame — gpui has no
    /// element-level rotation, so the text itself stays upright (see the
    /// module note on `render_inline_text_overlay`).
    fn inline_text_overlay(&self, cx: &gpui_kit::App) -> Option<InlineTextOverlay> {
        let draft = self.session.read(cx).text_draft.as_ref()?;
        let measured = inline_text_measured_size(&draft.style);
        let logical = draft.style.box_size.unwrap_or(measured);
        let viewport = self.canvas_viewport(cx);
        let per_pixel = viewport.points_per_pixel();
        let (doc_w, doc_h) = self.doc_size(cx);
        let ((ox, oy), (w, h)) = inline_text_box(
            draft.origin,
            draft.transform,
            draft.style.box_size,
            measured,
            self.inline_text_asset_width(draft, cx),
            viewport,
            CGSize::new(doc_w, doc_h),
        );
        let rotation = draft.transform.map(|t| t.rotation).unwrap_or(0.0);
        let corners = inline_text_box_corners((ox, oy), (w, h), rotation);
        let (min_x, min_y) = corners.iter().fold((f32::MAX, f32::MAX), |(a, b), (x, y)| {
            (a.min(*x), b.min(*y))
        });
        let (max_x, max_y) = corners.iter().fold((f32::MIN, f32::MIN), |(a, b), (x, y)| {
            (a.max(*x), b.max(*y))
        });
        let rgb = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
        let size = ScreenSize::new((max_x - min_x).max(24.0), (max_y - min_y).max(24.0));
        Some(InlineTextOverlay {
            origin: ScreenPoint::new(min_x, min_y),
            size,
            inset: inline_text_content_inset(size, logical, per_pixel),
            line_ratio: inline_text_line_ratio(&draft.style),
            font_px: (draft.style.font_size * per_pixel).max(6.0) as f32,
            color: (rgb(draft.style.red) << 16)
                | (rgb(draft.style.green) << 8)
                | rgb(draft.style.blue),
            font_family: draft.style.font_name.clone(),
        })
    }

    /// The self-drawn caret: an opaque bar pinned to the exact bounds the
    /// input reports for its built-in caret (`cursor_layout`, window space →
    /// canvas-area space), covering it with the Swift editor's solid bar.
    /// The built-in one blinks underneath and stays covered either way.
    fn render_inline_caret(&self, cx: &gpui_kit::App) -> Option<gpui_kit::AnyElement> {
        if !self.inline_text_caret_visible() {
            return None;
        }
        let (bounds, _line_height) = self.inline_text_input.read(cx).cursor_layout()?;
        let origin = self.canvas_bounds.origin;
        let left = bounds.origin.x - origin.x;
        let top = bounds.origin.y - origin.y;
        let (left, top) = (f32::from(left), f32::from(top));
        Some(
            gpui_kit::div()
                .id("inline-text-caret")
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(2.0))
                .h(px(f32::from(bounds.size.height) + 2.0))
                .bg(gpui_kit::rgb(crate::ui::BG_CARET))
                .into_any_element(),
        )
    }

    /// The overlay element: the multi-line input over the canvas. The frame
    /// outline and the eight resize handles are painted by the canvas's paint
    /// pass (through `CanvasScene::inline_text_frame`), rotated about the box
    /// center like Swift's `frameRotation`; gpui's divs have no element-level
    /// rotation, so the text itself stays axis-aligned — the one place this
    /// port visibly deviates from the Swift editor. Mouse presses inside the
    /// div stop propagating so the canvas never sees them as Type-tool
    /// clicks, except on a resize handle, which starts a resize drag
    /// (`InlineTextEditor.mouseDown`'s resize branch).
    pub(crate) fn render_inline_text_overlay(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::AnyElement> {
        let overlay = self.inline_text_overlay(cx)?;
        let input = self.inline_text_input.clone();
        let on_press = cx.listener(|this, event: &gpui_kit::MouseDownEvent, _window, cx| {
            let local = this.local_position(event.position);
            let doc = this.screen_to_doc(cx, local.0, local.1);
            if this.session.read(cx).text_draft.is_some()
                && let Some(handle) = this.inline_text_handle_hit(doc, cx)
            {
                this.begin_inline_text_resize(doc, handle, cx);
            }
            cx.stop_propagation();
        });
        // Option-arrow spacing while the input owns the keyboard
        // (`CanvasTextView.keyDown`): the chords are bound to these actions
        // (`keys::inline_text_bindings`) so they outrank the text system's
        // own alt-arrow bindings; the handlers stop propagation, or the
        // caret would also jump by words / grow extra cursors.
        let on_tracking_tighten = cx.listener(Self::on_inline_tracking_tighten);
        let on_tracking_loosen = cx.listener(Self::on_inline_tracking_loosen);
        let on_tracking_tighten_more = cx.listener(Self::on_inline_tracking_tighten_more);
        let on_tracking_loosen_more = cx.listener(Self::on_inline_tracking_loosen_more);
        let on_leading_tighten = cx.listener(Self::on_inline_leading_tighten);
        let on_leading_loosen = cx.listener(Self::on_inline_leading_loosen);
        let on_leading_tighten_more = cx.listener(Self::on_inline_leading_tighten_more);
        let on_leading_loosen_more = cx.listener(Self::on_inline_leading_loosen_more);
        // The text content sits inset from the box by the style's padding
        // scaled to the screen (`inline_text_content_inset`) — where the
        // committed raster starts its glyphs (Swift insets its text view the
        // same way, InlineTextEditor.swift:137-138). The input engine adds
        // its own multi-line paddings from the component Size on every
        // render (Input::render → `set_editor_paddings`), so the content div
        // is offset and sized to cancel exactly those — read from the same
        // default Size the Textarea is built with — leaving the glyphs on
        // the inset, and the wrap width equal to the box minus both insets,
        // which is the container the core wraps the commit's lines in
        // (`text_image`'s `container_width`, tools/text.rs:246).
        let engine = gpui_kit::component::Size::default();
        let engine_px = f32::from(engine.input_px());
        let engine_py = f32::from(engine.input_py());
        let content_left = overlay.origin.x + overlay.inset.width - engine_px;
        let content_top = overlay.origin.y + overlay.inset.height - engine_py;
        let content_size = ScreenSize::new(
            (overlay.size.width - overlay.inset.width * 2.0 + engine_px * 2.0).max(8.0),
            (overlay.size.height - overlay.inset.height * 2.0 + engine_py * 2.0).max(8.0),
        );
        Some(
            gpui_kit::div()
                .id("inline-text-editor")
                .absolute()
                .left(px(overlay.origin.x))
                .top(px(overlay.origin.y))
                .w(px(overlay.size.width))
                .h(px(overlay.size.height))
                .on_mouse_down(MouseButton::Left, on_press)
                .on_action(on_tracking_tighten)
                .on_action(on_tracking_loosen)
                .on_action(on_tracking_tighten_more)
                .on_action(on_tracking_loosen_more)
                .on_action(on_leading_tighten)
                .on_action(on_leading_loosen)
                .on_action(on_leading_tighten_more)
                .on_action(on_leading_loosen_more)
                .child(
                    gpui_kit::div()
                        .absolute()
                        .left(px(content_left))
                        .top(px(content_top))
                        .w(px(content_size.width))
                        .h(px(content_size.height))
                        .child(
                            Textarea::new(&input)
                                .appearance(false)
                                .bordered(false)
                                .aria_label("Canvas text")
                                // The draft's own typeface (`textAttributes`'s
                                // `.font: NSFont(name: style.fontName, ...)`). A name
                                // gpui cannot resolve falls back to the UI font.
                                .font_family(SharedString::from(overlay.font_family.clone()))
                                .text_size(px(overlay.font_px))
                                .text_color(gpui_kit::rgb(overlay.color))
                                // The core's row stride: the style's leading, else
                                // 120% of the size — not the component's flat
                                // 1.25rem (`relative` scales it with the font
                                // size, `inline_text_line_ratio`).
                                .line_height(relative(overlay.line_ratio)),
                        ),
                )
                .children(self.render_inline_caret(cx))
                .into_any_element(),
        )
    }

    /// Option-left tightens the tracking by one step; Shift makes ten
    /// (`CanvasTextView.keyDown`, InlineTextEditor.swift:18-19). The text
    /// style itself is shell-side (`ProjectWorkspaceView::adjust_text_tracking`).
    pub(crate) fn on_inline_tracking_tighten(
        &mut self,
        _: &crate::keys::InlineTextTrackingTighten,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_text_tracking(-1.0, cx);
        cx.stop_propagation();
    }

    /// Option-right loosens the tracking by one step; Shift makes ten.
    pub(crate) fn on_inline_tracking_loosen(
        &mut self,
        _: &crate::keys::InlineTextTrackingLoosen,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_text_tracking(1.0, cx);
        cx.stop_propagation();
    }

    /// Shift-Option-left tightens the tracking by ten.
    pub(crate) fn on_inline_tracking_tighten_more(
        &mut self,
        _: &crate::keys::InlineTextTrackingTightenMore,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_text_tracking(-10.0, cx);
        cx.stop_propagation();
    }

    /// Shift-Option-right loosens the tracking by ten.
    pub(crate) fn on_inline_tracking_loosen_more(
        &mut self,
        _: &crate::keys::InlineTextTrackingLoosenMore,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_text_tracking(10.0, cx);
        cx.stop_propagation();
    }

    /// Option-up closes the leading by one step; Shift makes ten
    /// (`adjust_text_leading`'s `down = false`).
    pub(crate) fn on_inline_leading_tighten(
        &mut self,
        _: &crate::keys::InlineTextLeadingTighten,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_text_leading(false, 1.0, cx);
        cx.stop_propagation();
    }

    /// Option-down opens the leading by one step; Shift makes ten.
    pub(crate) fn on_inline_leading_loosen(
        &mut self,
        _: &crate::keys::InlineTextLeadingLoosen,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_text_leading(true, 1.0, cx);
        cx.stop_propagation();
    }

    /// Shift-Option-up closes the leading by ten.
    pub(crate) fn on_inline_leading_tighten_more(
        &mut self,
        _: &crate::keys::InlineTextLeadingTightenMore,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_text_leading(false, 10.0, cx);
        cx.stop_propagation();
    }

    /// Shift-Option-down opens the leading by ten.
    pub(crate) fn on_inline_leading_loosen_more(
        &mut self,
        _: &crate::keys::InlineTextLeadingLoosenMore,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_text_leading(true, 10.0, cx);
        cx.stop_propagation();
    }

    /// The tracking step, routed to the shell's text-style edit.
    fn adjust_text_tracking(&mut self, delta: f64, cx: &mut Context<Self>) {
        self.app
            .update(cx, |app, cx| app.adjust_text_tracking(delta, cx))
            .ok();
    }

    /// The leading step, routed to the shell's text-style edit.
    fn adjust_text_leading(&mut self, down: bool, step: f64, cx: &mut Context<Self>) {
        self.app
            .update(cx, |app, cx| app.adjust_text_leading(down, step, cx))
            .ok();
    }

    /// Which resize handle a press at `doc` lands on, if a draft is open
    /// (`InlineTextEditor.hitTest`/`handle(at:)`). The editor is hit-tested
    /// against what is on screen, so point text that has grown while typing
    /// uses its shown transform, not the draft's stale one.
    pub(crate) fn inline_text_handle_hit(&self, doc: CGPoint, cx: &gpui_kit::App) -> Option<usize> {
        let draft = self.session.read(cx).text_draft.as_ref()?;
        let (logical, transform) = self.inline_text_shown(draft, cx);
        let per_pixel = self.canvas_viewport(cx).points_per_pixel();
        let handle_size = inline_text_handle_size(&transform, logical, per_pixel);
        let local = inline_text_local_point(&transform, logical, doc);
        inline_text_handle_at(local, logical, handle_size)
    }

    /// Starts a resize drag from a handle
    /// (`InlineTextEditor.mouseDown`): the drag starts from the transform
    /// actually shown — point text may have grown past the draft's stale
    /// one (`shownTransform ?? draft.transform ?? ...`). Point text first
    /// becomes a box of the size it has right now, which then holds and
    /// wraps the text — its scale and rotation are whatever the draft
    /// already carried.
    pub(crate) fn begin_inline_text_resize(
        &mut self,
        doc: CGPoint,
        handle: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(mut draft) = self.session.read(cx).text_draft.clone() else {
            return;
        };
        let (measured, transform) = self.inline_text_shown(&draft, cx);
        if draft.style.box_size.is_none() {
            draft.style.box_size = Some(measured);
            draft.transform = Some(transform);
            draft.origin = transform.origin;
            self.session
                .update(cx, |session, _| session.text_draft = Some(draft.clone()));
        }
        let source = draft.style.box_size.unwrap_or(measured);
        self.set_drag(
            Some(CanvasDrag::TextResize {
                handle,
                start: doc,
                old: transform,
                source,
                draft,
            }),
            cx,
        );
        self.touch(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InlineTextKey, ScreenSize, inline_text_box, inline_text_box_corners,
        inline_text_content_inset, inline_text_handle_at, inline_text_handle_size, inline_text_key,
        inline_text_line_ratio, inline_text_local_point, inline_text_measured_size,
        inline_text_overflows, inline_text_resize, inline_text_shown_transform,
    };
    use compositor_core::canvas_viewport::CanvasViewport;
    use compositor_core::document::{LayerTextStyle, TextAlignment};
    use compositor_core::geometry::{CGPoint, CGSize};
    use compositor_core::transform::{HANDLES, LayerTransform};

    /// A viewport with zero view size, the given zoom, and a raw pan — the
    /// pan-form mapping the old `doc_to_screen_at(zoom, pan)` tests encoded,
    /// since with a zero document rect the origin is exactly `pan`.
    fn viewport_with(zoom: f64, pan: (f64, f64)) -> CanvasViewport {
        let mut viewport = CanvasViewport::default();
        viewport.set_zoom(zoom, CGPoint::ZERO, CGSize::ZERO);
        viewport.pan = CGSize::new(pan.0, pan.1);
        viewport
    }

    #[test]
    fn boxed_text_uses_the_draft_transform_through_the_view_transform() {
        let transform = LayerTransform::new(CGPoint::new(10.0, 20.0), CGSize::new(200.0, 80.0));
        let box_size = CGSize::new(200.0, 80.0);
        let ((ox, oy), (w, h)) = inline_text_box(
            CGPoint::new(10.0, 20.0),
            Some(transform),
            Some(box_size),
            CGSize::new(999.0, 999.0),
            0.0,
            viewport_with(2.0, (5.0, 7.0)),
            CGSize::ZERO,
        );
        // Origin maps through the viewport, size scales by the per-pixel
        // ratio; the measurement must not leak into a boxed draft.
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
            0.0,
            viewport_with(1.0, (0.0, 0.0)),
            CGSize::ZERO,
        );
        assert_eq!((ox, oy), (4.0, 6.0));
        assert_eq!((w, h), (measured.width as f32, measured.height as f32));
    }

    #[test]
    fn content_inset_tracks_the_boxs_screen_scale() {
        // At 1:1 the 12-layer-px padding is 12 viewport px.
        assert_eq!(
            inline_text_content_inset(ScreenSize::new(200.0, 80.0), CGSize::new(200.0, 80.0), 1.0),
            ScreenSize::new(12.0, 12.0)
        );
        // Zoom scales it.
        assert_eq!(
            inline_text_content_inset(ScreenSize::new(400.0, 160.0), CGSize::new(200.0, 80.0), 2.0),
            ScreenSize::new(24.0, 24.0)
        );
        // A layer scaled 2x doubles it at the same zoom: the box is twice
        // the logical size on screen.
        assert_eq!(
            inline_text_content_inset(ScreenSize::new(400.0, 160.0), CGSize::new(200.0, 80.0), 1.0),
            ScreenSize::new(24.0, 24.0)
        );
        // A degenerate logical size falls back to the zoom.
        assert_eq!(
            inline_text_content_inset(ScreenSize::new(100.0, 50.0), CGSize::ZERO, 3.0),
            ScreenSize::new(36.0, 36.0)
        );
    }

    #[test]
    fn tracking_steps_move_the_shown_box_like_the_commit() {
        // Option-right steps the tracking; the overlay's frame (the measured
        // point-text box, recomputed from the draft every frame) grows by the
        // step for every character in the widest line — and the committed
        // raster grows by exactly the same amount, so what moved on screen
        // while editing is the box the commit makes.
        let base = LayerTextStyle {
            content: "Hi".into(),
            font_size: 40.0,
            ..LayerTextStyle::default()
        };
        let mut stepped = base.clone();
        stepped.tracking = 2.0;
        let mut tightened = base.clone();
        tightened.tracking = -2.0;
        let base_box = inline_text_measured_size(&base);
        let stepped_box = inline_text_measured_size(&stepped);
        let tightened_box = inline_text_measured_size(&tightened);
        // Two characters, one +2 step each.
        assert!((stepped_box.width - base_box.width - 4.0).abs() < 0.001);
        assert!((base_box.width - tightened_box.width - 4.0).abs() < 0.001);
        // Height is a row stride: tracking never moves it.
        assert_eq!(stepped_box.height, base_box.height);
        // The commit's rasters land on the same boxes the overlay shows.
        let base_raster = compositor_core::tools::text::text_image(&base).unwrap();
        let stepped_raster = compositor_core::tools::text::text_image(&stepped).unwrap();
        assert_eq!(
            stepped_raster.width as f64 - base_raster.width as f64,
            4.0,
            "commit raster tracks the same step"
        );
        assert_eq!(stepped_raster.height, base_raster.height);
    }

    #[test]
    fn the_overflow_marker_flips_with_tracking() {
        // Boxed text that fits its container at zero tracking picks up a
        // step's width per character; past the container width the overflow
        // plus marker appears live (`draw`'s glyph-range check), without
        // waiting for a commit.
        let fits = LayerTextStyle {
            content: "AAAAAAAAAAAAAAAAAAAA".into(),
            font_size: 12.0,
            box_size: Some(CGSize::new(200.0, 100.0)),
            ..LayerTextStyle::default()
        };
        assert!(!inline_text_overflows(&fits));
        let mut spread = fits.clone();
        spread.tracking = 2.0;
        assert!(
            inline_text_overflows(&spread),
            "20 characters at +2 tracking pass the 176px container"
        );
    }

    #[test]
    fn tracked_right_aligned_draft_commits_into_the_shown_box() {
        // The commit/cancel round trip: the geometry the overlay shows (the
        // measured box and its content inset) is the geometry the commit
        // rasterizes into, with the draft's tracking and alignment carried
        // through — ink lands inside the raster exactly where the inset and
        // the box say the content lives.
        let style = LayerTextStyle {
            content: "Hi there".into(),
            font_size: 40.0,
            tracking: 3.0,
            alignment: TextAlignment::Right,
            ..LayerTextStyle::default()
        };
        let measured = inline_text_measured_size(&style);
        let inset = inline_text_content_inset(
            ScreenSize::new(measured.width as f32, measured.height as f32),
            measured,
            1.0,
        );
        // 1:1 the inset is the style's padding — the first glyph's left edge.
        assert_eq!(inset.width, LayerTextStyle::PADDING as f32);
        let raster = compositor_core::tools::text::text_image(&style).unwrap();
        assert_eq!(raster.width as f64, measured.width);
        assert_eq!(raster.height as f64, measured.height);
        let padding = LayerTextStyle::PADDING;
        // Where the ink actually sits.
        let ink_bounds = |raster: &compositor_core::raster::Bitmap| {
            let mut min_x = raster.width;
            let mut max_x = 0usize;
            for y in 0..raster.height {
                for x in 0..raster.width {
                    if raster.pixel(x, y)[3] > 0 {
                        min_x = min_x.min(x);
                        max_x = max_x.max(x);
                    }
                }
            }
            (min_x, max_x)
        };
        // Right-aligned: the line's advance ends at the container's right
        // inset edge (pen = padding + (container - width) + width), so the
        // ink ends there minus at most the last glyph's advance and the
        // trailing tracking the box adds.
        let (_, max_x) = ink_bounds(&raster);
        let slack = 0.5 * style.font_size + 2.0 * style.tracking + 4.0;
        assert!(
            (max_x + 1) as f64 >= raster.width as f64 - padding - slack,
            "right-aligned ink reaches the right inset edge: max_x {max_x} of {}",
            raster.width
        );
        // The same draft left-aligned starts its ink at the inset itself.
        let mut left = style.clone();
        left.alignment = TextAlignment::Left;
        left.tracking = 0.0;
        let left_raster = compositor_core::tools::text::text_image(&left).unwrap();
        let (min_x, _) = ink_bounds(&left_raster);
        assert!(
            min_x as f64 >= padding - 1.0,
            "no ink in the left padding: min_x {min_x}"
        );
        assert!(
            min_x as f64 <= padding + 0.75 * left.font_size,
            "left-aligned ink starts at the inset: min_x {min_x}"
        );
        // And the alignment switch is commit-visible: the right-aligned
        // raster of the same draft keeps its ink off the left padding.
        let (right_min_x, _) = ink_bounds(&raster);
        assert!(
            right_min_x as f64 >= padding - 1.0,
            "right-aligned ink clears the left padding: {right_min_x}"
        );
    }

    #[test]
    fn line_ratio_matches_the_core_stride() {
        // Auto leading: 120% of the size (document.rs:408-410).
        let auto = LayerTextStyle {
            font_size: 72.0,
            ..LayerTextStyle::default()
        };
        assert!((inline_text_line_ratio(&auto) - 1.2).abs() < 0.0001);
        // An explicit leading overrides the auto one (document.rs:412-418).
        let explicit = LayerTextStyle {
            font_size: 60.0,
            leading: 30.0,
            ..LayerTextStyle::default()
        };
        assert!((inline_text_line_ratio(&explicit) - 0.5).abs() < 0.0001);
    }

    #[test]
    fn point_text_with_a_layer_transform_scales_from_the_layer() {
        // Editing existing point text: the draft carries the layer's
        // transform; while the text matches the raster the overlay shows the
        // transform itself (measurement 60 x 24, asset 60 wide → factor 5).
        let transform = LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(300.0, 120.0));
        let ((ox, oy), (w, h)) = inline_text_box(
            CGPoint::new(0.0, 0.0),
            Some(transform),
            None,
            CGSize::new(60.0, 24.0),
            60.0,
            viewport_with(0.5, (0.0, 0.0)),
            CGSize::ZERO,
        );
        assert_eq!((ox, oy), (0.0, 0.0));
        assert_eq!((w, h), (150.0, 60.0));
    }

    #[test]
    fn point_text_grows_with_the_fresh_measurement_keeping_the_layer_scale() {
        // synchronize, InlineTextEditor.swift:109-118: the shown size is the
        // fresh measurement times the layer's own scale
        // (factor = transform.width / asset.width), and the rotated top-left
        // corner is put back where it was.
        let transform = LayerTransform::new(CGPoint::new(100.0, 50.0), CGSize::new(300.0, 120.0));
        // One character typed: the measurement grew 60 → 90 wide, 24 → 36 tall.
        let shown = inline_text_shown_transform(
            transform.origin,
            Some(transform),
            None,
            CGSize::new(90.0, 36.0),
            60.0,
        );
        assert_eq!(shown.size, CGSize::new(450.0, 180.0));
        // The top-left corner never moves (axis-aligned box: it is the origin).
        let corner = shown.point(CGPoint::ZERO);
        let before = transform.point(CGPoint::ZERO);
        assert!((corner.x - before.x).abs() < 0.001 && (corner.y - before.y).abs() < 0.001);
        // The draft's own transform is untouched (the function is pure).
        assert_eq!(transform.size, CGSize::new(300.0, 120.0));
    }

    #[test]
    fn grown_point_text_keeps_its_rotated_top_left_corner() {
        // A rotated layer turns about its center, so growing it swings its
        // corner away — the anchor correction puts the corner back
        // (InlineTextEditor.swift:112-117).
        let mut transform =
            LayerTransform::new(CGPoint::new(100.0, 50.0), CGSize::new(300.0, 120.0));
        transform.rotation = 30.0;
        let shown = inline_text_shown_transform(
            transform.origin,
            Some(transform),
            None,
            CGSize::new(90.0, 36.0),
            60.0,
        );
        assert_eq!(shown.size, CGSize::new(450.0, 180.0));
        let corner = shown.point(CGPoint::ZERO);
        let before = transform.point(CGPoint::ZERO);
        assert!((corner.x - before.x).abs() < 0.001, "corner {corner:?}");
        assert!((corner.y - before.y).abs() < 0.001, "corner {corner:?}");
        // So the origin had to move to compensate the swing about the center.
        assert!(shown.origin != transform.origin);
        // Rotation and flips are the layer's own, untouched.
        assert_eq!(shown.rotation, 30.0);
    }

    #[test]
    fn growth_needs_a_layer_and_an_asset() {
        // No layer transform: point text is a fresh box at the origin sized
        // to the measurement — no factor applies.
        let shown = inline_text_shown_transform(
            CGPoint::new(4.0, 6.0),
            None,
            None,
            CGSize::new(90.0, 36.0),
            60.0,
        );
        assert_eq!(shown.size, CGSize::new(90.0, 36.0));
        assert_eq!(shown.origin, CGPoint::new(4.0, 6.0));
        // A layer without a raster (asset width 0) cannot be scaled against.
        let transform = LayerTransform::new(CGPoint::ZERO, CGSize::new(300.0, 120.0));
        let shown = inline_text_shown_transform(
            transform.origin,
            Some(transform),
            None,
            CGSize::new(90.0, 36.0),
            0.0,
        );
        assert_eq!(shown, transform);
        // Boxed text never grows, whatever the measurement says.
        let shown = inline_text_shown_transform(
            transform.origin,
            Some(transform),
            Some(CGSize::new(300.0, 120.0)),
            CGSize::new(90.0, 36.0),
            60.0,
        );
        assert_eq!(shown, transform);
    }

    #[test]
    fn boxed_text_overflow_follows_the_container() {
        // Text that does not fit its fixed box overflows
        // (InlineTextEditor.swift:311-326); point text grows, so it never
        // does.
        let fits = LayerTextStyle {
            content: "Hi".into(),
            font_size: 12.0,
            box_size: Some(CGSize::new(200.0, 100.0)),
            ..LayerTextStyle::default()
        };
        assert!(!inline_text_overflows(&fits));
        // Too many lines for the container height: overflow, whatever font
        // is installed (line height 14.4, 10 lines in a 76-tall container).
        let too_tall = LayerTextStyle {
            content: "a\na\na\na\na\na\na\na\na\na".into(),
            font_size: 12.0,
            box_size: Some(CGSize::new(200.0, 100.0)),
            ..LayerTextStyle::default()
        };
        assert!(inline_text_overflows(&too_tall));
        // One unbreakable word wider than the container.
        let too_wide = LayerTextStyle {
            content: "AAAAAAAAAAAAAA".into(),
            font_size: 72.0,
            box_size: Some(CGSize::new(200.0, 400.0)),
            ..LayerTextStyle::default()
        };
        assert!(inline_text_overflows(&too_wide));
        // Point text has no fixed container: never an overflow.
        let point = LayerTextStyle {
            content: "AAAAAAAAAAAAAA".into(),
            font_size: 72.0,
            ..LayerTextStyle::default()
        };
        assert!(!inline_text_overflows(&point));
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

    #[test]
    fn rotated_corners_land_where_the_transform_puts_them() {
        // The rotated frame's corners are the transform's own corner points
        // mapped to the screen (Swift pins the rotated top-left to the
        // anchor; InlineTextEditor.swift:124-134). Try a few angles.
        for rotation in [0.0, 90.0, -37.5, 415.0] {
            let mut transform =
                LayerTransform::new(CGPoint::new(10.0, 20.0), CGSize::new(200.0, 80.0));
            transform.rotation = rotation;
            let viewport = viewport_with(2.0, (5.0, 7.0));
            let ((ox, oy), (w, h)) = inline_text_box(
                transform.origin,
                Some(transform),
                Some(transform.size),
                CGSize::new(999.0, 999.0),
                999.0,
                viewport,
                CGSize::ZERO,
            );
            let corners = inline_text_box_corners((ox, oy), (w, h), rotation);
            let units = [
                CGPoint::new(0.0, 0.0),
                CGPoint::new(1.0, 0.0),
                CGPoint::new(1.0, 1.0),
                CGPoint::new(0.0, 1.0),
            ];
            for (corner, unit) in corners.iter().zip(units) {
                let doc = transform.point(unit);
                let shown = viewport.view_point(doc, CGSize::ZERO);
                let (sx, sy) = (shown.x as f32, shown.y as f32);
                assert!((corner.0 - sx).abs() < 0.001, "rotation {rotation}");
                assert!((corner.1 - sy).abs() < 0.001, "rotation {rotation}");
            }
        }
    }

    #[test]
    fn local_point_undoes_the_rotation_and_the_layer_scale() {
        // A 90-degree box half the logical size: a press on the document
        // lands at the logical spot the editor's own coordinates say.
        let mut transform = LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(100.0, 50.0));
        transform.rotation = 90.0;
        let logical = CGSize::new(200.0, 100.0);
        // The logical top-left corner is the transform's own top-left corner.
        let corner = transform.point(CGPoint::ZERO);
        let local = inline_text_local_point(&transform, logical, corner);
        assert!((local.x - 0.0).abs() < 0.001 && (local.y - 0.0).abs() < 0.001);
        // The logical bottom-right corner likewise.
        let corner = transform.point(CGPoint::new(1.0, 1.0));
        let local = inline_text_local_point(&transform, logical, corner);
        assert!((local.x - logical.width).abs() < 0.001);
        assert!((local.y - logical.height).abs() < 0.001);
        // Round trip through the rotation for a point in between.
        let doc = transform.point(CGPoint::new(0.25, 0.75));
        let local = inline_text_local_point(&transform, logical, doc);
        assert!((local.x - 50.0).abs() < 0.001 && (local.y - 75.0).abs() < 0.001);
    }

    #[test]
    fn handle_hits_follow_the_edge_bands() {
        // handle(at:): a band along each edge counts as that edge
        // (InlineTextEditor.swift:276-293); the reach is handleSize * 10/6
        // (= 10 at this size), which the bands below exercise through the
        // `handle_size` argument alone.
        let bounds = CGSize::new(300.0, 200.0);
        let handle_size = 6.0;
        // Corners.
        assert_eq!(
            inline_text_handle_at(CGPoint::new(2.0, 3.0), bounds, handle_size),
            Some(0)
        );
        assert_eq!(
            inline_text_handle_at(CGPoint::new(295.0, 1.0), bounds, handle_size),
            Some(2)
        );
        assert_eq!(
            inline_text_handle_at(CGPoint::new(298.0, 198.0), bounds, handle_size),
            Some(4)
        );
        assert_eq!(
            inline_text_handle_at(CGPoint::new(1.0, 195.0), bounds, handle_size),
            Some(6)
        );
        // Edge midpoints.
        assert_eq!(
            inline_text_handle_at(CGPoint::new(150.0, 5.0), bounds, handle_size),
            Some(1)
        );
        assert_eq!(
            inline_text_handle_at(CGPoint::new(295.0, 100.0), bounds, handle_size),
            Some(3)
        );
        assert_eq!(
            inline_text_handle_at(CGPoint::new(150.0, 195.0), bounds, handle_size),
            Some(5)
        );
        assert_eq!(
            inline_text_handle_at(CGPoint::new(5.0, 100.0), bounds, handle_size),
            Some(7)
        );
        // The middle is the text, not a handle.
        assert_eq!(
            inline_text_handle_at(CGPoint::new(150.0, 100.0), bounds, handle_size),
            None
        );
        // Beyond the bands — even outside the box — nothing hits.
        assert_eq!(
            inline_text_handle_at(CGPoint::new(150.0, 50.0), bounds, handle_size),
            None
        );
        assert_eq!(
            inline_text_handle_at(CGPoint::new(320.0, 100.0), bounds, handle_size),
            None
        );
        // A small box caps the reach at a third of the shorter side, so the
        // middle stays typable.
        let small = CGSize::new(24.0, 24.0);
        assert_eq!(
            inline_text_handle_at(CGPoint::new(12.0, 1.0), small, handle_size),
            Some(1)
        );
        assert_eq!(
            inline_text_handle_at(CGPoint::new(12.0, 12.0), small, handle_size),
            None
        );
    }

    #[test]
    fn handle_size_is_six_screen_points_over_the_zoom() {
        // handleSize = max(2, 6 / max(0.01, zoom * transform.width / logical.width)).
        let transform = LayerTransform::new(CGPoint::ZERO, CGSize::new(200.0, 80.0));
        let logical = CGSize::new(200.0, 80.0);
        // At 1:1 the handle is 6 logical points.
        assert_eq!(inline_text_handle_size(&transform, logical, 1.0), 6.0);
        // Deeper zoom shrinks it toward the 2-point floor.
        assert_eq!(inline_text_handle_size(&transform, logical, 4.0), 2.0);
        // A scaled-up layer divides by the layer's own scale too.
        let scaled = LayerTransform::new(CGPoint::ZERO, CGSize::new(400.0, 160.0));
        assert_eq!(inline_text_handle_size(&scaled, logical, 1.0), 3.0);
    }

    #[test]
    fn corner_drag_moves_the_dragged_corner_and_keeps_the_opposite() {
        // Drag the top-left handle up-left: the corner follows the pointer
        // and the bottom-right corner stays put (InlineTextEditor.swift:344-375).
        let old = LayerTransform::new(CGPoint::new(10.0, 20.0), CGSize::new(200.0, 80.0));
        let source = CGSize::new(200.0, 80.0);
        let resized = inline_text_resize(
            &old,
            source,
            0,
            CGPoint::new(50.0, 50.0),
            CGPoint::new(30.0, 40.0),
        )
        .unwrap();
        // The corner moved by (-20, -10): new box 220 x 90.
        assert_eq!(resized.box_size, CGSize::new(220.0, 90.0));
        assert_eq!(resized.transform.size, CGSize::new(220.0, 90.0));
        // The new top-left corner is the dragged position.
        assert_eq!(resized.transform.origin, CGPoint::new(-10.0, 10.0));
        let moved = resized.transform.point(CGPoint::ZERO);
        assert!((moved.x - (-10.0)).abs() < 0.001 && (moved.y - 10.0).abs() < 0.001);
        // The opposite corner stayed at the old bottom-right.
        let br = resized.transform.point(CGPoint::new(1.0, 1.0));
        assert!((br.x - 210.0).abs() < 0.001 && (br.y - 100.0).abs() < 0.001);
    }

    #[test]
    fn edge_drag_changes_one_axis_only() {
        // The right-edge midpoint handle grows the width alone; origin (the
        // top-left corner) and height stay.
        let old = LayerTransform::new(CGPoint::new(10.0, 20.0), CGSize::new(200.0, 80.0));
        let resized = inline_text_resize(
            &old,
            CGSize::new(200.0, 80.0),
            3,
            CGPoint::new(210.0, 60.0),
            CGPoint::new(240.0, 100.0),
        )
        .unwrap();
        assert_eq!(resized.box_size, CGSize::new(230.0, 80.0));
        assert_eq!(resized.transform.origin, CGPoint::new(10.0, 20.0));
        let tr = resized.transform.point(CGPoint::new(1.0, 0.0));
        assert!((tr.x - 240.0).abs() < 0.001);
    }

    #[test]
    fn drag_past_the_opposite_edge_clamps_at_sixteen_logical_pixels() {
        // Dragging a corner across the box stops at the 16-logical-pixel
        // minimum (minW/minH), both in the box size and the transform.
        let old = LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(200.0, 80.0));
        let resized = inline_text_resize(
            &old,
            CGSize::new(200.0, 80.0),
            0,
            CGPoint::new(0.0, 0.0),
            CGPoint::new(2000.0, 2000.0),
        )
        .unwrap();
        assert_eq!(resized.box_size, CGSize::new(16.0, 16.0));
        assert_eq!(resized.transform.size, CGSize::new(16.0, 16.0));
        // A degenerate box size (a zero-size source poisons the math) fails
        // `boxIsValid` and leaves the draft alone.
        assert!(
            inline_text_resize(
                &old,
                CGSize::new(0.0, 80.0),
                0,
                CGPoint::ZERO,
                CGPoint::new(10.0, 0.0)
            )
            .is_none()
        );
    }

    #[test]
    fn resize_drags_follow_the_box_rotation() {
        // A 90-degree box: a downward drag is along the box's own x axis, so
        // it narrows the width, not the height.
        let mut old = LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(100.0, 50.0));
        old.rotation = 90.0;
        let resized = inline_text_resize(
            &old,
            CGSize::new(100.0, 50.0),
            0,
            CGPoint::ZERO,
            CGPoint::new(0.0, 10.0),
        )
        .unwrap();
        assert_eq!(resized.box_size, CGSize::new(90.0, 50.0));
        assert_eq!(resized.transform.size, CGSize::new(90.0, 50.0));
        // The handle order is the one the drag math uses: corners first.
        assert_eq!(HANDLES[0], CGPoint::new(0.0, 0.0));
        assert_eq!(HANDLES[2], CGPoint::new(1.0, 0.0));
        assert_eq!(HANDLES[4], CGPoint::new(1.0, 1.0));
        assert_eq!(HANDLES[6], CGPoint::new(0.0, 1.0));
    }
}
