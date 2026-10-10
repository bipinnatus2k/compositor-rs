//! The empty tab's welcome view — Swift's tab-embedded new-canvas form
//! (`if session.document == nil { welcome }`, ContentView.swift:99-110, which
//! presents `NewCanvasSheet` in place of the canvas): a centered Width/Height
//! form whose Create canvas button builds the tab's document, with the
//! File > Open and File > Import Images flows beside it. The form is a view
//! entity since ticket 42 (ADR 0003 step E); the canvas view's welcome
//! branch clones it exactly where `render_canvas` used to branch.

use compositor_core::session::EditorSession;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    Context, DragMoveEvent, Entity, ExternalPaths, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, Styled, WeakEntity, Window, div, px, rgb,
};

use crate::project::Step;
use crate::sheets_ui::SheetRow;
use crate::state::{AppState, number_input};
use crate::ui::text_label;

/// The dimension cap the core enforces (`create_document` silently refuses
/// anything outside 1..=30,000, so the form disables Create for those too —
/// Swift's `CanvasDocument.validDimension`).
const MAX_DIMENSION: usize = 30_000;

/// The sheet's copy, verbatim from Swift (NewCanvasSheet.swift:20-21, 31-34)
/// — the ticket forbids inventing titles.
pub(crate) const WELCOME_TITLE: &str = "New canvas";
pub(crate) const WELCOME_SUBTITLE: &str = "A blank space for your next composition.";
pub(crate) const OPEN_PROJECT_LABEL: &str = "Open project";
pub(crate) const IMPORT_IMAGE_LABEL: &str = "Import image";
pub(crate) const CREATE_CANVAS_LABEL: &str = "Create canvas";

/// A Width/Height value the welcome form accepts: a whole number from
/// 1 to 30,000 (`CanvasDocument.validDimension`).
pub(crate) fn valid_dimension(value: usize) -> bool {
    (1..=MAX_DIMENSION).contains(&value)
}

/// Parses a typed Width/Height value (`CanvasDocument.validDimension`:
/// NewCanvasSheet.swift:14-16).
pub(crate) fn welcome_dimension(text: &str) -> Option<usize> {
    text.trim()
        .parse::<usize>()
        .ok()
        .filter(|value| valid_dimension(*value))
}

/// Both fields parsed (`NewCanvasSheet.valid`, :14-16): the Create gate the
/// button and the Return key share.
pub(crate) fn welcome_can_create(width: &str, height: &str) -> Option<(usize, usize)> {
    Some((welcome_dimension(width)?, welcome_dimension(height)?))
}

/// The form-level disable (NewCanvasSheet.swift:45, `.disabled(session.
/// isImporting || session.showsBusy)`): an import in flight or any project
/// operation freezes the whole sheet. Both flags are session fields now
/// (stage-D ticket 34).
pub(crate) fn welcome_disabled(session: &EditorSession) -> bool {
    session.is_importing || session.is_project_busy
}

/// The empty tab's welcome form (`ContentView.welcome`, the tab-embedded
/// `NewCanvasSheet`): a view entity since ticket 42 — it owns the
/// Width/Height field entities (NewCanvasSheet's `@State width`/`height`,
/// NewCanvasSheet.swift:9-10) and renders the whole form. Rendered only by
/// the canvas view's welcome branch, so it keeps no session observer (the
/// branch re-renders with the window, as before); its session field is
/// re-pointed on tab changes by [`AppState::sync_session_observer`].
pub struct WelcomeView {
    /// The active tab's session — the busy gates and the Create write speak
    /// to it directly (`NewCanvasSheet`'s `session`).
    session: Entity<EditorSession>,
    /// The window shell (the transitional root): the Open/Import flows, the
    /// file-drop receive and the repaint epoch stay there. No AppState
    /// dependencies beyond what the moved handlers already used.
    app: WeakEntity<AppState>,
    /// The dimension fields (`NewCanvasSheet`'s `@State width`/`height`),
    /// defaulting to 1920×1080 like Swift. Built here because an
    /// `InputState` needs the window.
    welcome_width_input: Entity<InputState>,
    welcome_height_input: Entity<InputState>,
}

impl WelcomeView {
    pub(crate) fn new(
        session: Entity<EditorSession>,
        app: WeakEntity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The form's defaults: NewCanvasSheet's `@State width`/`height`
        // ("1920" / "1080").
        let welcome_width_input = number_input(1920.0, window, cx);
        let welcome_height_input = number_input(1080.0, window, cx);
        WelcomeView {
            session,
            app,
            welcome_width_input,
            welcome_height_input,
        }
    }

    /// Re-points the session on tab changes (`AppState::sync_session_observer`,
    /// same as the canvas view and the layers panel).
    pub(crate) fn set_session(&mut self, session: Entity<EditorSession>) {
        self.session = session;
    }

    /// The view's route to the app-side repaint epoch (`touch`): the frame
    /// cache and `revision` stay on AppState (ticket 39's field audit), so
    /// Create asks the shell through the seam.
    fn touch(&mut self, cx: &mut Context<Self>) {
        self.app.update(cx, |app, cx| app.touch(cx)).ok();
    }

    /// Both form fields parsed; `None` while either is not a valid
    /// dimension (the Create button's disabled state).
    fn welcome_size(&self, cx: &gpui_kit::App) -> Option<(usize, usize)> {
        let width = self.welcome_width_input.read(cx).value().to_string();
        let height = self.welcome_height_input.read(cx).value().to_string();
        welcome_can_create(&width, &height)
    }

    /// The Create action, shared by the button and the Return key
    /// (NewCanvasSheet.swift:34-39: `createDocument` with an empty layer —
    /// the tab gains its Layer 1 — and the viewport refits to the new size).
    /// Both gates live here: the fields must parse and the sheet must not be
    /// busy, so a Return can never slip past a disabled button.
    fn welcome_submit(&mut self, cx: &mut Context<Self>) {
        if welcome_disabled(self.session.read(cx)) {
            return;
        }
        let Some((width, height)) = self.welcome_size(cx) else {
            return;
        };
        self.session.update(cx, |session, _| {
            session.create_document(width, height, true)
        });
        self.touch(cx);
    }

    /// The welcome view an empty tab renders instead of the canvas. The form
    /// sits centered like the modal sheets (same panel chrome, 470 px wide)
    /// over the canvas background, and keeps the canvas's file-drop import.
    pub fn render_welcome(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let valid = self.welcome_size(cx).is_some();
        // The sheet-level disable (NewCanvasSheet.swift:45): an import in
        // flight or a project operation freezes the whole form.
        let disabled = welcome_disabled(self.session.read(cx));

        // The sheet's title bar (sheet_title/text sizes follow the other
        // sheets; the subtitle is NewCanvasSheet's secondary line).
        let header = div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(rgb(crate::ui::BORDER))
            .child(text_label(WELCOME_TITLE, false).text_size(px(14.0)))
            .child(text_label(WELCOME_SUBTITLE, true));

        // The two dimension fields (NewCanvasSheet's Width/Height TextFields
        // defaulting to 1920×1080) plus its validity hint line.
        let body = div()
            .flex()
            .flex_col()
            .px_3()
            .py_2()
            .gap_1()
            .child(
                SheetRow::new("Width").with_control(
                    Input::new(&self.welcome_width_input)
                        .id("welcome-width")
                        .w(px(140.0))
                        .disabled(disabled),
                ),
            )
            .child(
                SheetRow::new("Height").with_control(
                    Input::new(&self.welcome_height_input)
                        .id("welcome-height")
                        .w(px(140.0))
                        .disabled(disabled),
                ),
            )
            .child(if valid {
                text_label("Transparent canvas · sRGB", true)
            } else {
                text_label("Enter whole numbers from 1 to 30,000 pixels.", true)
                    .text_color(rgb(crate::ui::STATUS_WARNING))
            });

        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(rgb(crate::ui::BORDER))
            // File > Open's entry point, exactly as the menu action runs it
            // (workspace.rs `on_open`): the workspace asks for the two-phase
            // open dialog and the pending flow plays the file picker's part.
            .child(
                Button::new("welcome-open")
                    .label(OPEN_PROJECT_LABEL)
                    .small()
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.app
                            .update(cx, |app, cx| match app.workspace.open(None, cx) {
                                Step::Pending(pending) => app.present_pending(pending, window, cx),
                                Step::Done(_) => {}
                            })
                            .ok();
                    })),
            )
            // File > Import Images' entry point: the same shared flow the
            // menu action runs (workspace.rs `prompt_import_images`), so the
            // picker, the import and the refit cannot drift apart.
            .child(
                Button::new("welcome-import")
                    .label(IMPORT_IMAGE_LABEL)
                    .small()
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.app
                            .update(cx, |app, cx| app.prompt_import_images(cx))
                            .ok();
                    })),
            )
            // The primary action sits on the row's end, as in Swift.
            .child(div().flex_1())
            .child(
                Button::new("welcome-create")
                    .label(CREATE_CANVAS_LABEL)
                    .small()
                    .primary()
                    .disabled(!valid || disabled)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.welcome_submit(cx);
                    })),
            );

        // The canvas area itself: same background and file-drop import as the
        // painted canvas, with the form centered over it.
        div()
            .id("welcome-area")
            .relative()
            .flex_1()
            .overflow_hidden()
            .bg(rgb(crate::ui::BG_CANVAS_AREA))
            .drag_over::<ExternalPaths>(|style, _, _, _| {
                // File drags tint the area, as they tint the canvas.
                style.bg(rgb(crate::ui::BG_DROP_AREA))
            })
            .on_drag_move::<ExternalPaths>(cx.listener(
                |this, event: &DragMoveEvent<ExternalPaths>, _window, cx| {
                    // The drop point is canvas gesture state (ticket 39); the
                    // welcome form shares the canvas's file-drop import, so it
                    // records the point through the shell's canvas entity.
                    this.app
                        .update(cx, |app, cx| {
                            app.canvas_view.update(cx, |canvas, cx| {
                                let local = canvas.local_position(event.event.position);
                                canvas.drop_point_doc =
                                    Some(canvas.screen_to_doc(cx, local.0, local.1));
                            });
                        })
                        .ok();
                },
            ))
            .on_drop::<ExternalPaths>(cx.listener(|this, dropped: &ExternalPaths, _window, cx| {
                let paths = dropped.paths().to_vec();
                this.app
                    .update(cx, |app, cx| app.receive_dropped_files(paths, cx))
                    .ok();
            }))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .w(px(470.0))
                            .flex()
                            .flex_col()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(crate::ui::BORDER))
                            .bg(rgb(crate::ui::BG_PANEL))
                            .overflow_hidden()
                            // Create supports Return (NewCanvasSheet.swift:40,
                            // the button's `.configuredNativeShortcut(.return)`):
                            // keys typed in either field bubble past the text
                            // system (a single-line input propagates Enter), so
                            // the panel sees the bare Return and submits. The
                            // validity and busy gates live in `welcome_submit`,
                            // matching the disabled button this mirrors.
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                                if event.keystroke.key != "enter" {
                                    return;
                                }
                                let m = &event.keystroke.modifiers;
                                if m.control || m.alt || m.platform || m.shift {
                                    return;
                                }
                                this.welcome_submit(cx);
                            }))
                            .child(header)
                            .child(body)
                            .child(footer),
                    ),
            )
            .into_any_element()
    }
}

impl Render for WelcomeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_welcome(cx)
    }
}

impl AppState {
    /// The empty tab's welcome view (Swift `ContentView.welcome`): the form
    /// entity built once (its inputs need the window); the canvas view's
    /// welcome branch clones it into the tree (ticket 42).
    pub fn render_welcome(&self) -> Entity<WelcomeView> {
        self.welcome.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{valid_dimension, welcome_dimension};
    use super::{welcome_can_create, welcome_disabled};
    use compositor_core::session::EditorSession;

    #[test]
    fn welcome_dimension_takes_whole_sizes_from_one_to_thirty_thousand() {
        // The defaults and the guard rails (createDocument's range check).
        assert_eq!(welcome_dimension("1920"), Some(1920));
        assert_eq!(welcome_dimension(" 1080 "), Some(1080));
        assert_eq!(welcome_dimension("1"), Some(1));
        assert_eq!(welcome_dimension("30000"), Some(30_000));
        assert_eq!(welcome_dimension(""), None);
        assert_eq!(welcome_dimension("abc"), None);
        // A negative never parses into an unsigned dimension.
        assert_eq!(welcome_dimension("-5"), None);
        assert_eq!(welcome_dimension("0"), None);
        assert_eq!(welcome_dimension("30001"), None);
    }

    #[test]
    fn valid_dimension_mirrors_the_core_dimension_guard() {
        assert!(valid_dimension(1));
        assert!(valid_dimension(30_000));
        assert!(!valid_dimension(0));
        assert!(!valid_dimension(30_001));
    }

    /// The Create gate (`NewCanvasSheet.valid`, :14-16): both fields must
    /// parse, for the button and the Return key alike.
    #[test]
    fn welcome_can_create_needs_both_fields_valid() {
        assert_eq!(welcome_can_create("1920", "1080"), Some((1920, 1080)));
        assert_eq!(welcome_can_create(" 64 ", "9"), Some((64, 9)));
        // Either field invalid blocks the submit.
        assert_eq!(welcome_can_create("1920", "abc"), None);
        assert_eq!(welcome_can_create("", "1080"), None);
        assert_eq!(welcome_can_create("0", "1080"), None);
        assert_eq!(welcome_can_create("1920", "30001"), None);
    }

    /// The form-level disable (NewCanvasSheet.swift:45): an import in flight
    /// or a project operation freezes the sheet; a brush stroke (not in
    /// Swift's guard) does not. Both flags are session fields (stage-D
    /// ticket 34).
    #[test]
    fn welcome_disabled_follows_the_swift_busy_flags() {
        let mut session = EditorSession::new();
        assert!(!welcome_disabled(&session));

        session.is_importing = true;
        assert!(welcome_disabled(&session));
        session.is_importing = false;

        session.is_project_busy = true;
        assert!(welcome_disabled(&session));
        session.is_project_busy = false;

        session.has_brush_stroke = true;
        assert!(!welcome_disabled(&session));
    }
}
