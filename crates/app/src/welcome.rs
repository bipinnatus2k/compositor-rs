//! The empty tab's welcome view — Swift's tab-embedded new-canvas form
//! (`if session.document == nil { welcome }`, ContentView.swift:99-110, which
//! presents `NewCanvasSheet` in place of the canvas): a centered Width/Height
//! form whose Create canvas button builds the tab's document, with the
//! File > Open and File > Import Images flows beside it.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    Context, DragMoveEvent, ExternalPaths, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Styled, div, px, rgb,
};

use crate::project::Step;
use crate::sheets_ui::sheet_row;
use crate::state::AppState;
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
/// operation (the port's `is_project_busy`) freezes the whole sheet.
pub(crate) fn welcome_disabled(busy: &crate::tabs::ProjectBusy) -> bool {
    busy.is_importing || busy.is_project_busy
}

impl AppState {
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
        if welcome_disabled(&self.tab().busy) {
            return;
        }
        let Some((width, height)) = self.welcome_size(cx) else {
            return;
        };
        self.tab().session.update(cx, |session, _| {
            session.create_document(width, height, true)
        });
        self.request_refit();
        self.touch(cx);
    }

    /// The welcome view an empty tab renders instead of the canvas. The form
    /// sits centered like the modal sheets (same panel chrome, 470 px wide)
    /// over the canvas background, and keeps the canvas's file-drop import.
    pub fn render_welcome(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let valid = self.welcome_size(cx).is_some();
        // The sheet-level disable (NewCanvasSheet.swift:45): an import in
        // flight or a project operation freezes the whole form.
        let disabled = welcome_disabled(&self.tab().busy);

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
            .child(sheet_row(
                "Width",
                Input::new(&self.welcome_width_input)
                    .id("welcome-width")
                    .w(px(140.0))
                    .disabled(disabled),
            ))
            .child(sheet_row(
                "Height",
                Input::new(&self.welcome_height_input)
                    .id("welcome-height")
                    .w(px(140.0))
                    .disabled(disabled),
            ))
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
                    .on_click(cx.listener(
                        |this, _, window, cx| match this.workspace.open(None, cx) {
                            Step::Pending(pending) => this.present_pending(pending, window, cx),
                            Step::Done(_) => {}
                        },
                    )),
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
                        this.prompt_import_images(cx);
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
                |this, event: &DragMoveEvent<ExternalPaths>, _window, _cx| {
                    let local = this.local_position(event.event.position);
                    this.drop_point_doc = Some(this.screen_to_doc(local.0, local.1));
                },
            ))
            .on_drop::<ExternalPaths>(cx.listener(|this, dropped: &ExternalPaths, _window, cx| {
                let paths = dropped.paths().to_vec();
                this.receive_dropped_files(paths, cx);
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

#[cfg(test)]
mod tests {
    use super::{valid_dimension, welcome_dimension};
    use super::{welcome_can_create, welcome_disabled};
    use crate::tabs::ProjectBusy;

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
    /// Swift's guard) does not.
    #[test]
    fn welcome_disabled_follows_the_swift_busy_flags() {
        assert!(!welcome_disabled(&ProjectBusy::default()));
        assert!(welcome_disabled(&ProjectBusy {
            is_importing: true,
            ..ProjectBusy::default()
        }));
        assert!(welcome_disabled(&ProjectBusy {
            is_project_busy: true,
            ..ProjectBusy::default()
        }));
        assert!(!welcome_disabled(&ProjectBusy {
            has_brush_stroke: true,
            ..ProjectBusy::default()
        }));
    }
}
