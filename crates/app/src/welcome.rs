//! The empty tab's welcome view — Swift's tab-embedded new-canvas form
//! (`if session.document == nil { welcome }`, ContentView.swift:99-110, which
//! presents `NewCanvasSheet` in place of the canvas): a centered Width/Height
//! form whose Create canvas button builds the tab's document, with the
//! File > Open and File > Import Images flows beside it.

use gpui_kit::{
    div, px, rgb, Context, DragMoveEvent, ExternalPaths, InteractiveElement, IntoElement,
    ParentElement, Styled,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, Sizable};

use crate::project::Step;
use crate::sheets_ui::sheet_row;
use crate::state::AppState;
use crate::ui::text_label;

/// The dimension cap the core enforces (`create_document` silently refuses
/// anything outside 1..=30,000, so the form disables Create for those too —
/// Swift's `CanvasDocument.validDimension`).
const MAX_DIMENSION: usize = 30_000;

/// One welcome form field: a whole number from 1 to 30,000.
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

impl AppState {
    /// Both form fields parsed; `None` while either is not a valid
    /// dimension (the Create button's disabled state).
    fn welcome_size(&self, cx: &gpui_kit::App) -> Option<(usize, usize)> {
        let width = self.welcome_width_input.read(cx).value().to_string();
        let height = self.welcome_height_input.read(cx).value().to_string();
        Some((welcome_dimension(&width)?, welcome_dimension(&height)?))
    }

    /// The welcome view an empty tab renders instead of the canvas. The form
    /// sits centered like the modal sheets (same panel chrome, 470 px wide)
    /// over the canvas background, and keeps the canvas's file-drop import.
    pub fn render_welcome(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let valid = self.welcome_size(cx).is_some();

        // The sheet's title bar (sheet_title/text sizes follow the other
        // sheets; the subtitle is NewCanvasSheet's secondary line).
        let header = div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(rgb(0x4a4a4a))
            .child(text_label("Welcome", false).text_size(px(14.0)))
            .child(text_label("A blank space for your next composition.", true));

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
                    .w(px(140.0)),
            ))
            .child(sheet_row(
                "Height",
                Input::new(&self.welcome_height_input)
                    .id("welcome-height")
                    .w(px(140.0)),
            ))
            .child(if valid {
                text_label("Transparent canvas · sRGB", true)
            } else {
                text_label("Enter whole numbers from 1 to 30,000 pixels.", true)
                    .text_color(rgb(0xe0a24a))
            });

        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(rgb(0x4a4a4a))
            // File > Open's entry point, exactly as the menu action runs it
            // (workspace.rs `on_open`): the workspace asks for the two-phase
            // open dialog and the pending flow plays the file picker's part.
            .child(
                Button::new("welcome-open")
                    .label("Open…")
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| {
                        match this.workspace.open(None) {
                            Step::Pending(pending) => this.present_pending(pending, window, cx),
                            Step::Done(_) => {}
                        }
                    })),
            )
            // File > Import Images' entry point, exactly as the menu action
            // runs it (workspace.rs `on_import_images`): the native
            // multi-select panel importing into this tab.
            .child(
                Button::new("welcome-import")
                    .label("Import Images…")
                    .small()
                    .on_click(cx.listener(|_this, _, _, cx| {
                        let receiver = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
                            files: true,
                            directories: false,
                            multiple: true,
                            prompt: None,
                        });
                        cx.spawn(async move |this, cx| {
                            let picked = receiver
                                .await
                                .ok()
                                .and_then(|r| r.ok())
                                .unwrap_or_default()
                                .unwrap_or_default();
                            this.update(cx, |this, cx| {
                                if picked.is_empty() {
                                    return;
                                }
                                let failures = this.tab().session.import_images(&picked, None);
                                if failures.is_empty() {
                                    this.set_status(
                                        format!("Imported {} images", picked.len()),
                                        cx,
                                    );
                                } else {
                                    this.set_status(failures.join("; "), cx);
                                }
                                this.fit_for = (0, 0);
                                this.touch(cx);
                            })
                            .ok();
                        })
                        .detach();
                    })),
            )
            // The primary action sits on the row's end, as in Swift.
            .child(div().flex_1())
            .child(
                Button::new("welcome-create")
                    .label("Create canvas")
                    .small()
                    .primary()
                    .disabled(!valid)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let Some((width, height)) = this.welcome_size(cx) else {
                            return;
                        };
                        // The startup document's shape (`createDocument` with
                        // an empty layer: the tab gains its Layer 1)...
                        this.tab().session.create_document(width, height, true);
                        // ...and the viewport refits to the new size, like
                        // every other document-arrives path.
                        this.request_refit();
                        this.touch(cx);
                    })),
            );

        // The canvas area itself: same background and file-drop import as the
        // painted canvas, with the form centered over it.
        div()
            .id("welcome-area")
            .relative()
            .flex_1()
            .overflow_hidden()
            .bg(rgb(0x232326))
            .drag_over::<ExternalPaths>(|style, _, _, _| {
                // File drags tint the area, as they tint the canvas.
                style.bg(rgb(0x2a2f3a))
            })
            .on_drag_move::<ExternalPaths>(cx.listener(
                |this, event: &DragMoveEvent<ExternalPaths>, _window, _cx| {
                    let local = this.local_position(event.event.position);
                    this.drop_point_doc = Some(this.screen_to_doc(local.0, local.1));
                },
            ))
            .on_drop::<ExternalPaths>(cx.listener(
                |this, dropped: &ExternalPaths, _window, cx| {
                    let paths = dropped.paths().to_vec();
                    this.receive_dropped_files(paths, cx);
                },
            ))
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
                            .border_color(rgb(0x4a4a4a))
                            .bg(rgb(0x2d2d2d))
                            .overflow_hidden()
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
}
