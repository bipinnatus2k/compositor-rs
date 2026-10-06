//! The floating panel system (`UI/FloatingPanel.swift`): movable, non-modal
//! panels for tool dialogs. They never dim the editor, open centered on the
//! canvas the first time, and reopen wherever they were last left (for the
//! app session). The close button reports through the close callback so
//! callers can treat it as Cancel (Swift's `onClose`).
//!
//! Divergences from Swift (see docs/workspace-api-design.md 偏差 #2): panels
//! are in-window overlays rather than real `NSPanel`s, positions are window
//! coordinates instead of screen coordinates, and `hidesOnDeactivate` does
//! not exist. `snap_to_window` keeps a dragged panel inside the main window.
//!
//! The state model (`FloatingPanels`) is gpui-window-free and unit tested
//! here; the rendering shell (`render_panel`) is a thin container the
//! wiring batch feeds sheet content into.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::{
    AnyElement, ClickEvent, Context, Global, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, SharedString,
    Styled, WeakEntity, Window, anchored, canvas, div, point, px,
};

use crate::ui::text_label;

/// Camera Raw's three grading wheels are the widest thing a docked panel
/// holds; their own sliders are narrowed to fit this rather than the panel
/// being widened to fit them (FloatingPanel.swift:125).
pub const DOCKED_WIDTH: f32 = 440.0;

/// The panel identifiers (`ContentView.swift:11-15`,
/// `KeyboardShortcuts.swift:133`, `ColorPickerSheet.swift:167`).
pub mod panel_names {
    pub const LEVELS_PANEL: &str = "levelsPanel";
    pub const ADJUSTMENT_PANEL: &str = "adjustmentPanel";
    pub const SELECTION_AMOUNT_PANEL: &str = "selectionAmountPanel";
    pub const FILTER_PANEL: &str = "filterPanel";
    pub const EFFECTS_PANEL: &str = "effectsPanel";
    pub const KEYBOARD_SHORTCUTS: &str = "keyboardShortcuts";
    pub const COLOR_PICKER_PANEL: &str = "colorPickerPanel";
}

/// Where a panel opens the first time (or each time, for docked styles)
/// (`FloatingPanelPlacement`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelPlacement {
    /// Centered on the canvas, or the last position the user left the panel.
    Automatic,
    /// Full height of the main document window, flush against its right edge.
    DockedToMainWindowRight,
}

/// One open panel's registry entry; `open`'s order is the stacking order
/// (the last entry is the topmost).
#[derive(Clone, Debug)]
pub struct PanelState {
    pub name: &'static str,
    pub title: SharedString,
    pub placement: PanelPlacement,
}

/// The state side of `FloatingPanelController`. Positions are the remembered
/// top-left corners in window coordinates per panel, so a panel reopens
/// where it was left (for the app session, like Swift's static `positions`).
#[derive(Default)]
pub struct FloatingPanels {
    /// Stacking order: the last entry is the topmost (`makeKeyAndOrderFront`).
    open: Vec<PanelState>,
    positions: HashMap<&'static str, (f32, f32)>,
}

impl FloatingPanels {
    pub fn new() -> Self {
        Self::default()
    }

    /// `show(title:content:placement:)` (:38-78): if the panel is already
    /// visible it is brought to the top (`makeKeyAndOrderFront`) and gets the
    /// new title, but keeps its frame — content changes must not shift the
    /// panel (:59-61). An invisible panel joins the stack as the topmost.
    pub fn show(
        &mut self,
        name: &'static str,
        title: impl Into<SharedString>,
        placement: PanelPlacement,
    ) {
        let title = title.into();
        if let Some(ix) = self.open.iter().position(|p| p.name == name) {
            let mut entry = self.open.remove(ix);
            entry.title = title;
            entry.placement = placement;
            self.open.push(entry);
            return;
        }
        self.open.push(PanelState {
            name,
            title,
            placement,
        });
    }

    /// `close()` hides the panel. Returns whether it was visible, so the
    /// caller can route the Cancel only when a real close happened (Swift
    /// reports `onClose` from `windowWillClose` and skips it while merely
    /// dismissing :85-93, :160-165).
    pub fn close(&mut self, name: &str) -> bool {
        let before = self.open.len();
        self.open.retain(|p| p.name != name);
        self.open.len() != before
    }

    /// `isVisible`.
    pub fn is_visible(&self, name: &str) -> bool {
        self.open.iter().any(|p| p.name == name)
    }

    /// The panel's placement while open; `None` when not open.
    pub fn placement(&self, name: &str) -> Option<PanelPlacement> {
        self.open
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.placement)
    }

    /// Whether the open panel is docked (the renderer picks the right frame
    /// path from this).
    pub fn is_docked(&self, name: &str) -> bool {
        self.placement(name) == Some(PanelPlacement::DockedToMainWindowRight)
    }

    /// The topmost open panel, for render-order and focus decisions.
    pub fn top_most(&self) -> Option<&'static str> {
        self.open.last().map(|p| p.name)
    }

    /// All open panels in stacking order (bottom first), for rendering.
    pub fn open_panels(&self) -> &[PanelState] {
        &self.open
    }

    /// An open panel's title; `None` when the panel is not open.
    pub fn title_of(&self, name: &str) -> Option<SharedString> {
        self.open
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.title.clone())
    }

    /// The panel's top-left corner in window coordinates. `None` when the
    /// panel is not open, and `None` for a docked panel — its frame comes
    /// from [`docked_top_left`] / [`docked_size`] against the current window
    /// size, recomputed every frame like Swift's frame observer (:127-157).
    ///
    /// An `Automatic` panel with a remembered position reopens at that
    /// top-left. Without one it opens centered on the canvas area
    /// (`canvasCenter(in:)` :106-116): the *panel's center* lands on the
    /// canvas center (`setFrameOrigin(x: center.x - width/2, …)`), so the
    /// panel's size is needed. The computed origin is kept as the panel's
    /// position, which is what keeps a content-size change from shifting the
    /// panel afterwards (:59-61); Swift achieves the same by never moving an
    /// already visible panel's frame on re-show.
    pub fn position(
        &mut self,
        name: &str,
        canvas_center: (f32, f32),
        panel_size: (f32, f32),
    ) -> Option<(f32, f32)> {
        let panel = self.open.iter().find(|p| p.name == name)?;
        if panel.placement == PanelPlacement::DockedToMainWindowRight {
            return None;
        }
        if let Some(&(x, y)) = self.positions.get(&name) {
            return Some((x, y));
        }
        // Center the panel on the canvas center, then remember it.
        let origin = (
            canvas_center.0 - panel_size.0 / 2.0,
            canvas_center.1 - panel_size.1 / 2.0,
        );
        self.positions.insert(panel.name, origin);
        Some(origin)
    }

    /// `remember(_:)` (:99-104): record the top-left after the user moved
    /// the panel. A docked panel is never remembered — that would put the
    /// next Gaussian Blur, and every other filter that shares this panel, on
    /// the right edge.
    pub fn remember_position(&mut self, name: &'static str, x: f32, y: f32) {
        if self.is_docked(name) {
            return;
        }
        self.positions.insert(name, (x, y));
    }

    /// The saved position, if any (test helper for "reopens where it was
    /// left" assertions).
    pub fn saved_position(&self, name: &str) -> Option<(f32, f32)> {
        self.positions.get(&name).copied()
    }

    /// `refocus(_:)` (:171-173): return keyboard focus to a panel, e.g.
    /// after a click on the canvas. The state side brings the panel to the
    /// top (`makeKey` also orders front) and reports whether it is open at
    /// all; the actual focus-handle focus is the wiring batch's job.
    pub fn refocus(&mut self, name: &'static str) -> bool {
        if !self.is_visible(name) {
            return false;
        }
        let title = self.title(name);
        let placement = self
            .placement(name)
            .expect("just checked the panel is open");
        self.show(name, title, placement);
        true
    }

    /// The open panel's current title (for `refocus` re-showing); empty
    /// when the panel is not open.
    fn title(&self, name: &str) -> SharedString {
        self.title_of(name).unwrap_or_default()
    }
}

/// A docked panel's top-left corner: flush against the main window's right
/// edge, full height (`applyDockedFrame` :127-136).
pub fn docked_top_left(window_size: (f32, f32)) -> (f32, f32) {
    (window_size.0 - DOCKED_WIDTH, 0.0)
}

/// A docked panel's size: [`DOCKED_WIDTH`] wide, the full window height.
pub fn docked_size(window_size: (f32, f32)) -> (f32, f32) {
    (DOCKED_WIDTH, window_size.1)
}

// MARK: Rendering shell

/// Which panel an in-progress title-bar drag belongs to; a drag owns the
/// pointer until mouse-up even when it leaves its control, like SwiftUI's
/// window drag. Each `render_panel` gets a unique cookie so two panels'
/// installed window handlers never double-apply one drag.
#[derive(Default)]
struct PanelDragState {
    drag: Option<PanelDrag>,
}

struct PanelDrag {
    cookie: u64,
    /// The panel's top-left when the drag started (window coordinates).
    origin: Point<Pixels>,
    /// The pointer position when the drag started.
    grab: Point<Pixels>,
}

impl Global for PanelDragState {}

static NEXT_COOKIE: AtomicU64 = AtomicU64::new(1);

/// Renders one floating panel as an in-window overlay:
/// `anchored().position(pos).snap_to_window()` + `.occlude()`, a title row
/// that is the drag region with a ghost close button, and the content slot.
///
/// `top_left` is the resolved frame origin: the [`FloatingPanels::position`]
/// result for `Automatic` panels, [`docked_top_left`] for docked ones.
/// `fixed_size` is `Some` only for docked panels (440 × window height); an
/// `Automatic` panel hugs its content, like AppKit's `fittingSize`.
///
/// Callbacks (the wiring batch supplies them via `cx.listener`-style
/// closures over its entity):
/// - `on_close`: the title row's ✕ reports here — treat as Cancel
///   (Swift's `onClose`/`windowWillClose`).
/// - `on_moved`: called during and at the end of a title-bar drag with the
///   new top-left in window coordinates; store it with
///   [`FloatingPanels::remember_position`] (Swift's `windowDidMove`).
pub fn render_panel<E: 'static>(
    app: &WeakEntity<E>,
    top_left: Point<Pixels>,
    title: SharedString,
    fixed_size: Option<(Pixels, Pixels)>,
    content: AnyElement,
    on_close: impl Fn(&mut E, &mut Window, &mut Context<E>) + 'static,
    on_moved: impl Fn(&mut E, Point<Pixels>, &mut Window, &mut Context<E>) + 'static,
) -> AnyElement {
    let cookie = NEXT_COOKIE.fetch_add(1, Ordering::Relaxed);

    // The title row is the drag region (`NSPanel`'s titled style). The press
    // starts the drag; window-level move/up handlers (installed by the
    // invisible canvas below, following color_picker_ui's pattern) track it
    // so the panel keeps following the pointer outside the row.
    let app_down = app.clone();
    let on_moved_moved = on_moved;
    let on_close_click = on_close;

    let title_row = div()
        .flex()
        .flex_row()
        .items_center()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(gpui_kit::rgb(0x4a4a4a))
        .on_mouse_down(
            MouseButton::Left,
            move |event: &MouseDownEvent, _window, cx| {
                let state = cx.default_global::<PanelDragState>();
                state.drag = Some(PanelDrag {
                    cookie,
                    origin: top_left,
                    grab: event.position,
                });
            },
        )
        .child(
            div()
                .flex_1()
                .child(text_label(title, false).text_size(px(13.0))),
        )
        .child(
            Button::new(SharedString::from(format!("panel-close-{cookie}")))
                .label("✕")
                // The ✕ glyph alone announces as nothing to a screen reader.
                .accessibility_label("Close panel")
                .small()
                .ghost()
                .on_click(move |_event: &ClickEvent, window, cx| {
                    app_down
                        .update(cx, |state, cx| on_close_click(state, window, cx))
                        .ok();
                }),
        )
        // The invisible canvas exists only to install the window-level drag
        // handlers during paint.
        .child(drag_events_canvas(app.clone(), cookie, on_moved_moved));

    let shell = div()
        .flex()
        .flex_col()
        .rounded_md()
        .border_1()
        .border_color(gpui_kit::rgb(0x4a4a4a))
        .bg(gpui_kit::rgb(0x2d2d2d))
        .overflow_hidden()
        .occlude()
        .child(title_row)
        .child(content);

    let shell = match fixed_size {
        Some((width, height)) => shell.w(width).h(height),
        None => shell,
    };

    anchored()
        .position(top_left)
        .snap_to_window()
        .child(shell)
        .into_any_element()
}

/// Installs the window-level move/up handlers for one panel's title-bar drag
/// (the `color_picker_ui.rs` window-level drag pattern: a press on the title
/// row wrote the drag into [`PanelDragState`], and only the matching
/// cookie's handlers act on it).
fn drag_events_canvas<E: 'static>(
    app: WeakEntity<E>,
    cookie: u64,
    on_moved: impl Fn(&mut E, Point<Pixels>, &mut Window, &mut Context<E>) + 'static,
) -> AnyElement {
    canvas(
        |_bounds, _window, _cx| {},
        move |_bounds, _prepaint, window, _cx| {
            let app_move = app.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, _phase, window, cx| {
                let dragging = cx
                    .try_global::<PanelDragState>()
                    .and_then(|state| state.drag.as_ref())
                    .map(|drag| drag.cookie);
                if dragging != Some(cookie) {
                    return;
                }
                let drag = cx
                    .global::<PanelDragState>()
                    .drag
                    .as_ref()
                    .expect("checked above");
                let moved = drag
                    .origin
                    .add(event.position - drag.grab)
                    .clamp_to_window(window);
                app_move
                    .update(cx, |state, cx| on_moved(state, moved, window, cx))
                    .ok();
            });
            window.on_mouse_event(move |_event: &MouseUpEvent, _phase, _window, cx| {
                let state = cx.default_global::<PanelDragState>();
                if state.drag.as_ref().map(|drag| drag.cookie) == Some(cookie) {
                    state.drag = None;
                }
            });
        },
    )
    .absolute()
    .inset_0()
    .into_any_element()
}

trait PanelOriginExt {
    fn add(self, delta: Point<Pixels>) -> Point<Pixels>;
    fn clamp_to_window(self, window: &Window) -> Point<Pixels>;
}

impl PanelOriginExt for Point<Pixels> {
    fn add(self, delta: Point<Pixels>) -> Point<Pixels> {
        point(self.x + delta.x, self.y + delta.y)
    }

    /// A dragged panel stays inside the main window: keep at least a
    /// sliver of the title row reachable (`snap_to_window` clamps the
    /// rendering the same way).
    fn clamp_to_window(self, window: &Window) -> Point<Pixels> {
        let viewport = window.viewport_size();
        let max_x = (viewport.width - px(60.)).max(px(0.));
        let max_y = (viewport.height - px(32.)).max(px(0.));
        point(self.x.clamp(px(0.), max_x), self.y.clamp(px(0.), max_y))
    }
}

// MARK: Tests (`FloatingPanelTests.swift`, state-side ports)

#[cfg(test)]
mod tests {
    use super::*;

    const CANVAS_CENTER: (f32, f32) = (200.0, 150.0);
    /// A representative content size (`fittingSize`).
    const PANEL_SIZE: (f32, f32) = (100.0, 50.0);

    /// `dockedPlacementLeavesTheSavedFilterPosition`: a filter panel parked
    /// where the user left it must not be displaced by a docked Camera Raw
    /// session in between.
    #[test]
    fn docked_placement_leaves_the_saved_filter_position() {
        let mut panels = FloatingPanels::new();
        panels.show(
            panel_names::FILTER_PANEL,
            "Gaussian Blur",
            PanelPlacement::Automatic,
        );
        // The user parks the panel.
        panels.remember_position(panel_names::FILTER_PANEL, 40.0, 240.0);
        panels.close(panel_names::FILTER_PANEL);

        // Camera Raw docks to the document window.
        panels.show(
            panel_names::FILTER_PANEL,
            "Camera Raw Filter",
            PanelPlacement::DockedToMainWindowRight,
        );
        assert!(panels.is_visible(panel_names::FILTER_PANEL));
        assert!(panels.is_docked(panel_names::FILTER_PANEL));
        panels.close(panel_names::FILTER_PANEL);

        // The filter panel reopens where it was left, not on the right edge.
        panels.show(
            panel_names::FILTER_PANEL,
            "Gaussian Blur",
            PanelPlacement::Automatic,
        );
        assert_eq!(
            panels.position(panel_names::FILTER_PANEL, CANVAS_CENTER, PANEL_SIZE),
            Some((40.0, 240.0))
        );
    }

    /// `remember(_:)`'s guard: a docked panel is never remembered — that
    /// would put the next Gaussian Blur on the right edge.
    #[test]
    fn docked_panels_are_never_remembered() {
        let mut panels = FloatingPanels::new();
        panels.show(
            panel_names::FILTER_PANEL,
            "Camera Raw Filter",
            PanelPlacement::DockedToMainWindowRight,
        );
        panels.remember_position(panel_names::FILTER_PANEL, 800.0, 0.0);
        assert_eq!(panels.saved_position(panel_names::FILTER_PANEL), None);
        panels.close(panel_names::FILTER_PANEL);

        panels.show(
            panel_names::FILTER_PANEL,
            "Gaussian Blur",
            PanelPlacement::Automatic,
        );
        // No memory → the canvas center, not the docked frame.
        assert_eq!(
            panels.position(panel_names::FILTER_PANEL, CANVAS_CENTER, PANEL_SIZE),
            Some((
                CANVAS_CENTER.0 - PANEL_SIZE.0 / 2.0,
                CANVAS_CENTER.1 - PANEL_SIZE.1 / 2.0
            ))
        );
    }

    /// `canvasCenter(in:)` centers the *panel's center* on the canvas
    /// center (`setFrameOrigin(x: center.x - size.width/2, …)` :110-115).
    #[test]
    fn first_automatic_open_centers_on_the_canvas() {
        let mut panels = FloatingPanels::new();
        panels.show(
            panel_names::LEVELS_PANEL,
            "Levels",
            PanelPlacement::Automatic,
        );
        let origin = panels
            .position(panel_names::LEVELS_PANEL, CANVAS_CENTER, PANEL_SIZE)
            .expect("open panel has a position");
        assert_eq!(origin.0, CANVAS_CENTER.0 - PANEL_SIZE.0 / 2.0);
        assert_eq!(origin.1, CANVAS_CENTER.1 - PANEL_SIZE.1 / 2.0);
        // The computed origin becomes the memory.
        assert_eq!(
            panels.saved_position(panel_names::LEVELS_PANEL),
            Some(origin)
        );
    }

    /// A remembered panel reopens at the last top-left.
    #[test]
    fn a_remembered_panel_reopens_where_it_was_left() {
        let mut panels = FloatingPanels::new();
        panels.show(
            panel_names::EFFECTS_PANEL,
            "Effects",
            PanelPlacement::Automatic,
        );
        panels.position(panel_names::EFFECTS_PANEL, CANVAS_CENTER, PANEL_SIZE);
        panels.remember_position(panel_names::EFFECTS_PANEL, 12.0, 34.0);
        panels.close(panel_names::EFFECTS_PANEL);
        panels.show(
            panel_names::EFFECTS_PANEL,
            "Effects",
            PanelPlacement::Automatic,
        );
        assert_eq!(
            panels.position(panel_names::EFFECTS_PANEL, CANVAS_CENTER, PANEL_SIZE),
            Some((12.0, 34.0))
        );
    }

    /// Content changes must not shift the panel (:59-61): the computed
    /// origin sticks, a later fitting-size change does not re-center it.
    #[test]
    fn content_size_changes_do_not_shift_the_panel() {
        let mut panels = FloatingPanels::new();
        panels.show(
            panel_names::LEVELS_PANEL,
            "Levels",
            PanelPlacement::Automatic,
        );
        let first = panels.position(panel_names::LEVELS_PANEL, CANVAS_CENTER, PANEL_SIZE);
        let taller = (PANEL_SIZE.0, PANEL_SIZE.1 + 200.0);
        assert_eq!(
            panels.position(panel_names::LEVELS_PANEL, CANVAS_CENTER, taller),
            first
        );
    }

    /// A docked panel's frame comes from the window, never from a stored
    /// position; it is 440 wide and full height, flush against the right
    /// edge (`applyDockedFrame` :127-136, `dockedWidth` :125).
    #[test]
    fn docked_panels_frame_from_the_window() {
        assert_eq!(DOCKED_WIDTH, 440.0);
        let window = (1180.0, 760.0);
        assert_eq!(docked_top_left(window), (1180.0 - 440.0, 0.0));
        assert_eq!(docked_size(window), (440.0, 760.0));

        let mut panels = FloatingPanels::new();
        panels.show(
            panel_names::FILTER_PANEL,
            "Camera Raw Filter",
            PanelPlacement::DockedToMainWindowRight,
        );
        // `position` defers to the window-based frame.
        assert_eq!(
            panels.position(panel_names::FILTER_PANEL, CANVAS_CENTER, PANEL_SIZE),
            None
        );
    }

    /// The close button reports `onClose`; `close` returns whether a real
    /// close happened so the caller can route the Cancel only then
    /// (`windowWillClose` :160-165).
    #[test]
    fn close_reports_whether_a_real_close_happened() {
        let mut panels = FloatingPanels::new();
        panels.show(
            panel_names::ADJUSTMENT_PANEL,
            "Levels",
            PanelPlacement::Automatic,
        );
        assert!(panels.is_visible(panel_names::ADJUSTMENT_PANEL));
        assert!(panels.close(panel_names::ADJUSTMENT_PANEL));
        assert!(!panels.is_visible(panel_names::ADJUSTMENT_PANEL));
        // Hiding an already hidden panel is a no-op, not a close report.
        assert!(!panels.close(panel_names::ADJUSTMENT_PANEL));
    }

    /// `show` on a visible panel brings it to the top and applies the new
    /// title, keeping one registry entry (`makeKeyAndOrderFront` :77).
    #[test]
    fn showing_a_visible_panel_tops_it_and_updates_the_title() {
        let mut panels = FloatingPanels::new();
        panels.show(
            panel_names::LEVELS_PANEL,
            "Levels",
            PanelPlacement::Automatic,
        );
        panels.show(
            panel_names::EFFECTS_PANEL,
            "Effects",
            PanelPlacement::Automatic,
        );
        assert_eq!(panels.top_most(), Some(panel_names::EFFECTS_PANEL));

        panels.show(
            panel_names::LEVELS_PANEL,
            "Levels (2)",
            PanelPlacement::Automatic,
        );
        assert_eq!(panels.top_most(), Some(panel_names::LEVELS_PANEL));
        let top = panels.open_panels().last().expect("two open");
        assert_eq!(top.name, panel_names::LEVELS_PANEL);
        assert_eq!(top.title.to_string(), "Levels (2)");
        // Still exactly one entry per name, both panels open.
        assert_eq!(panels.open_panels().len(), 2);
        assert!(panels.is_visible(panel_names::EFFECTS_PANEL));
    }

    /// `refocus` brings an open panel to the top and reports success;
    /// a closed panel cannot take focus (`refocus(_:)` :171-173).
    #[test]
    fn refocus_returns_keyboard_focus_to_an_open_panel() {
        let mut panels = FloatingPanels::new();
        panels.show(
            panel_names::LEVELS_PANEL,
            "Levels",
            PanelPlacement::Automatic,
        );
        panels.show(
            panel_names::EFFECTS_PANEL,
            "Effects",
            PanelPlacement::Automatic,
        );
        assert!(panels.refocus(panel_names::LEVELS_PANEL));
        assert_eq!(panels.top_most(), Some(panel_names::LEVELS_PANEL));

        assert!(!panels.refocus(panel_names::FILTER_PANEL));
    }

    /// The panel identifiers match Swift's controller names
    /// (`ContentView.swift:11-15`, `KeyboardShortcuts.swift:133`,
    /// `ColorPickerSheet.swift:167`).
    #[test]
    fn panel_names_match_swift() {
        assert_eq!(panel_names::LEVELS_PANEL, "levelsPanel");
        assert_eq!(panel_names::ADJUSTMENT_PANEL, "adjustmentPanel");
        assert_eq!(panel_names::SELECTION_AMOUNT_PANEL, "selectionAmountPanel");
        assert_eq!(panel_names::FILTER_PANEL, "filterPanel");
        assert_eq!(panel_names::EFFECTS_PANEL, "effectsPanel");
        assert_eq!(panel_names::KEYBOARD_SHORTCUTS, "keyboardShortcuts");
        assert_eq!(panel_names::COLOR_PICKER_PANEL, "colorPickerPanel");
    }
}
