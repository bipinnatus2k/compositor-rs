//! Port of the Type tool's option bar (`TypeControls.swift`): the bar's
//! render body plus its font-catalog and alignment helpers. The bar is a
//! view entity (ADR 0003 step E, ticket 42) owning the numeric fields; the
//! bar shell itself stays in ui/navigation_tool_header.rs.

use std::rc::Rc;

use gpui_kit::component::Selectable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    App, AppContext, Context, Entity, Focusable, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window,
    px,
};

use compositor_core::session::EditorSession;

use crate::state::{
    AppState, TextStyleField, clamp_font_size, format_number, leading_from_text, leading_to_text,
    number_input, parse_field,
};
use crate::ui::{AppCallback, Dropdown, MenuEntry, app_dispatch, app_on, div_row, text_label};

/// The Type tool's option bar (`TypeControls.swift`): a view entity since
/// ticket 42 — it owns the size/tracking/leading field entities and their
/// write subscriptions (`subscribe_inputs`' text-style slice). Only in the
/// render tree while the Type tool is active, so it keeps no session
/// observer; its session field is re-pointed on tab changes by
/// [`AppState::sync_session_observer`].
pub struct TypeControlsView {
    /// The active tab's session — the readouts, the field writes and the
    /// draft buttons speak to it directly (`TypeControls`' `@Bindable
    /// session`).
    session: Entity<EditorSession>,
    /// The window shell (the transitional root): the repaint epoch, the
    /// color picker opening and the shared header row stay there. No
    /// AppState dependencies beyond what the moved handlers already used.
    app: WeakEntity<AppState>,
    /// The numeric fields (TypeControls' size/tracking/leading bindings):
    /// seeded from the current text style at construction, exactly as
    /// `AppState::new` seeded them when the fields lived there.
    type_size_input: Entity<InputState>,
    type_tracking_input: Entity<InputState>,
    type_leading_input: Entity<InputState>,
    /// Keep-alive for the field subscriptions: dropping a Subscription
    /// unsubscribes, so the bundle is held, never read.
    #[allow(dead_code)]
    subscriptions: Vec<Subscription>,
}

impl TypeControlsView {
    pub(crate) fn new(
        session: Entity<EditorSession>,
        app: WeakEntity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The seeds are the current style (Swift's bindings read
        // `session.currentTextStyle`), after the persisted defaults were
        // applied to the tab at creation.
        let style = session.read(cx).current_text_style();
        let type_size_input = number_input(style.font_size, window, cx);
        let type_tracking_input = number_input(style.tracking, window, cx);
        let type_leading_input =
            cx.new(|cx| InputState::new(window, cx).default_value(leading_to_text(style.leading)));
        let mut view = TypeControlsView {
            session,
            app,
            type_size_input,
            type_tracking_input,
            type_leading_input,
            subscriptions: Vec::new(),
        };
        view.subscribe_inputs(cx);
        view
    }

    /// Re-points the session on tab changes (`AppState::sync_session_observer`,
    /// same as the canvas view and the layers panel). The field
    /// subscriptions read this field at event time, so the re-point keeps
    /// them writing to the active tab.
    pub(crate) fn set_session(&mut self, session: Entity<EditorSession>) {
        self.session = session;
    }

    /// Wiring for the numeric fields: typing writes straight into the
    /// session (`subscribe_inputs`' text-style slice, moved with the
    /// fields — Swift's live `onChange(of: text)`).
    fn subscribe_inputs(&mut self, cx: &mut Context<Self>) {
        let text_fields = [
            (self.type_size_input.clone(), TextStyleField::Size),
            (self.type_tracking_input.clone(), TextStyleField::Tracking),
            (self.type_leading_input.clone(), TextStyleField::Leading),
        ];
        for (input, field) in text_fields {
            let watched = input.clone();
            let subscription = cx.subscribe(
                &input,
                move |this, _input, event: &InputEvent, cx: &mut Context<Self>| {
                    if let InputEvent::Change = event {
                        let text = watched.read(cx).value().to_string();
                        this.text_style_field_changed(field, &text, cx);
                    }
                },
            );
            self.subscriptions.push(subscription);
        }
    }

    /// The view's route to the app-side repaint epoch (`touch`): the frame
    /// cache and `revision` stay on AppState (ticket 39's field audit), so
    /// an interaction that changed the session asks the shell through the
    /// seam.
    fn touch(&mut self, cx: &mut Context<Self>) {
        self.app.update(cx, |app, cx| app.touch(cx)).ok();
    }

    /// A typed change in one of the bar's fields writes straight into the
    /// style — the draft while text is being edited, the defaults otherwise
    /// (`session.changeTextStyle`).
    fn text_style_field_changed(
        &mut self,
        field: TextStyleField,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        match field {
            TextStyleField::Size => {
                if let Some(value) = parse_field(text) {
                    let value = clamp_font_size(value);
                    self.session.update(cx, |session, _| {
                        session.change_text_style(move |style| style.font_size = value)
                    });
                }
            }
            TextStyleField::Tracking => {
                if let Some(value) = parse_field(text) {
                    self.session.update(cx, |session, _| {
                        session.change_text_style(move |style| style.tracking = value)
                    });
                }
            }
            TextStyleField::Leading => {
                let value = leading_from_text(text);
                self.session.update(cx, |session, _| {
                    session.change_text_style(move |style| style.leading = value)
                });
            }
        }
        self.touch(cx);
    }

    /// Pushes the current style back into the bar's fields (Swift's
    /// `sync`, run when a value may have moved under an unfocused field).
    fn sync_text_style_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let style = self.session.read(cx).current_text_style();
        let fields = [
            (&self.type_size_input, format_number(style.font_size)),
            (&self.type_tracking_input, format_number(style.tracking)),
            (&self.type_leading_input, leading_to_text(style.leading)),
        ];
        for (input, text) in fields {
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            if !focused && input.read(cx).value() != text {
                input.update(cx, |state, cx| state.set_value(text, window, cx));
            }
        }
    }

    /// The canvas's inline editor re-binds after a draft change (the
    /// buttons' `sync_inline_text` follow-up, through the shell).
    fn sync_inline_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.app
            .update(cx, |app, cx| {
                app.canvas_view
                    .update(cx, |canvas, cx| canvas.sync_inline_text(window, cx));
            })
            .ok();
    }

    /// `TypeControls.swift`'s bar controls: font, size, color, alignment,
    /// tracking, leading, and the draft buttons (Cancel/Done while a text
    /// draft is open, Edit Text otherwise).
    pub(crate) fn type_controls(&mut self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let app = self.app.clone();
        let style = self.session.read(cx).current_text_style();
        let font = style.font_name.clone();
        let names = font_menu_names(&font, available_font_names(), FONT_MENU_CAP);
        let entries = names
            .into_iter()
            .map(|name| {
                let chosen = name.clone();
                MenuEntry::new(name.clone(), move |state, _, cx| {
                    let chosen = chosen.clone();
                    state.tab().session.update(cx, |session, _| {
                        session.change_text_style(move |style| {
                            style.font_name = chosen;
                        })
                    });
                    state.touch(cx);
                })
                .checked(style.font_name == name)
            })
            .collect();
        let rgb = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
        let color = self.session.read(cx).type_color();
        let color_rgb = (rgb(color.red) << 16) | (rgb(color.green) << 8) | rgb(color.blue);
        let (has_document, has_draft, active_is_text) = {
            let session = self.session.read(cx);
            (
                session.document.is_some(),
                session.text_draft.is_some(),
                session
                    .active_layer()
                    .and_then(|layer| layer.live_text())
                    .is_some(),
            )
        };

        let mut controls = vec![
            Dropdown::new("type-font", font)
                .with_width(px(150.0))
                .with_entries(entries)
                .with_dispatch(app_dispatch(&app))
                .into_any_element(),
            // The unit hugs its field (Swift unitSuffix HStack spacing 2,
            // ToolHeaderStyle.swift:13-18): a tighter gap than the row's
            // control spacing.
            div_row()
                .gap_1()
                .child(
                    Input::new(&self.type_size_input)
                        .id("type-size")
                        .small()
                        .w(px(52.0)),
                )
                .child(text_label("px", true))
                .into_any_element(),
            gpui_kit::div()
                .id("type-color")
                .w(px(36.0))
                .h(px(18.0))
                .rounded_xs()
                .border_1()
                .border_color(gpui_kit::rgb(crate::ui::BORDER_INSET))
                .bg(gpui_kit::rgb(color_rgb))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.app
                        .update(cx, |app, cx| app.open_text_color_picker_ui(window, cx))
                        .ok();
                }))
                .into_any_element(),
            AlignmentButtons::new(style.alignment)
                .with_on_select(app_on(&app, |state, alignment, _, cx| {
                    state.tab().session.update(cx, |session, _| {
                        session.change_text_style(move |style| {
                            style.alignment = alignment;
                        })
                    });
                    state.touch(cx);
                }))
                .into_any_element(),
            div_row()
                .child(text_label("Tracking", true))
                .child(
                    Input::new(&self.type_tracking_input)
                        .id("type-tracking")
                        .small()
                        .w(px(48.0)),
                )
                .into_any_element(),
            div_row()
                .child(text_label("Leading", true))
                .child(
                    Input::new(&self.type_leading_input)
                        .id("type-leading")
                        .small()
                        .w(px(52.0)),
                )
                .into_any_element(),
        ];
        if has_draft {
            controls.push(
                Button::new("type-cancel")
                    .label("Cancel")
                    .small()
                    .disabled(!has_document)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.session.update(cx, |session, _| session.cancel_text());
                        this.sync_inline_text(window, cx);
                        this.sync_text_style_inputs(window, cx);
                        this.touch(cx);
                    }))
                    .into_any_element(),
            );
            controls.push(
                Button::new("type-done")
                    .label("Done")
                    .small()
                    .primary()
                    .disabled(!has_document)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.session.update(cx, |session, _| session.finish_text());
                        this.sync_inline_text(window, cx);
                        this.sync_text_style_inputs(window, cx);
                        this.touch(cx);
                    }))
                    .into_any_element(),
            );
        } else {
            controls.push(
                Button::new("type-edit")
                    .label("Edit Text")
                    .small()
                    .disabled(!has_document || !active_is_text)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.session
                            .update(cx, |session, _| session.edit_active_text());
                        this.sync_inline_text(window, cx);
                        this.sync_text_style_inputs(window, cx);
                        this.touch(cx);
                    }))
                    .into_any_element(),
            );
        }
        controls
    }
}

pub(crate) const FONT_MENU_CAP: usize = 250;

/// The current font plus the installed catalog, sorted case-insensitively;
/// the current name always survives the cap (Swift's
/// `Set(availableFonts + [selected]).sorted()`).
pub fn font_menu_names(current: &str, catalog: &[String], cap: usize) -> Vec<String> {
    let mut names: Vec<String> = catalog
        .iter()
        .filter(|name| name.as_str() != current)
        .cloned()
        .collect();
    names.truncate(cap.saturating_sub(1));
    names.push(current.to_string());
    names.sort_by_key(|name| name.to_lowercase());
    names
}

/// The installed-font catalog: file stems from the font directories core's
/// text engine scans (`tools/text.rs`). A style's `font_name` is matched
/// against font file names there, so stems work as menu entries; the
/// list is cached because the directory scan is not cheap.
pub(crate) fn available_font_names() -> &'static Vec<String> {
    static FONTS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        const DIRECTORIES: &[&str] = &[
            "C:\\Windows\\Fonts",
            "/System/Library/Fonts",
            "/System/Library/Fonts/Supplemental",
            "/Library/Fonts",
            "/usr/share/fonts",
            "/usr/local/share/fonts",
        ];
        let mut names: Vec<String> = Vec::new();
        for directory in DIRECTORIES {
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let extension = path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(|extension| extension.to_ascii_lowercase());
                if !matches!(extension.as_deref(), Some("ttf" | "otf" | "ttc")) {
                    continue;
                }
                if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
        names.sort_by_key(|name| name.to_lowercase());
        names.dedup();
        names
    })
}

/// The Type bar's alignment segment (`TextAlignment.allCases`): ghost
/// buttons with the selected one lit. The widget set here has no SF Symbol
/// glyphs, so the labels carry the meaning; a click reports the picked
/// alignment.
#[derive(IntoElement)]
pub struct AlignmentButtons {
    selected: compositor_core::document::TextAlignment,
    on_select: AppCallback<compositor_core::document::TextAlignment>,
}

impl AlignmentButtons {
    pub fn new(selected: compositor_core::document::TextAlignment) -> Self {
        AlignmentButtons {
            selected,
            on_select: Rc::new(|_, _, _| {}),
        }
    }

    pub fn with_on_select(
        mut self,
        on_select: AppCallback<compositor_core::document::TextAlignment>,
    ) -> Self {
        self.on_select = on_select;
        self
    }
}

impl RenderOnce for AlignmentButtons {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let on_select = self.on_select;
        let mut row = gpui_kit::div().flex().flex_row().gap_0p5().flex_shrink_0();
        for (alignment, label) in [
            (compositor_core::document::TextAlignment::Left, "Left"),
            (compositor_core::document::TextAlignment::Center, "Center"),
            (compositor_core::document::TextAlignment::Right, "Right"),
        ] {
            let on_select = on_select.clone();
            row = row.child(
                Button::new(SharedString::from(format!("type-align-{}", label)))
                    .label(label)
                    .ghost()
                    .compact()
                    .selected(self.selected == alignment)
                    .on_click(move |_, window, cx| on_select(alignment, window, cx)),
            );
        }
        row
    }
}

#[cfg(test)]
mod tests {
    use super::font_menu_names;

    #[test]
    fn font_menu_names_include_the_current_face_once() {
        let catalog = vec![
            "Arial".to_string(),
            "Calibri".to_string(),
            "segoeui".to_string(),
        ];
        // A catalog face: the list stays sorted, no duplicate.
        let names = font_menu_names("Calibri", &catalog, 250);
        assert_eq!(names, vec!["Arial", "Calibri", "segoeui"]);
        // A face the catalog lacks (Helvetica on this host): it is added and
        // sorted in.
        let names = font_menu_names("Helvetica", &catalog, 250);
        assert_eq!(names, vec!["Arial", "Calibri", "Helvetica", "segoeui"]);
    }

    #[test]
    fn font_menu_names_keep_the_current_face_under_the_cap() {
        let catalog: Vec<String> = ["ahab", "bob", "zed", "zig"]
            .iter()
            .map(|name| name.to_string())
            .collect();
        // The cap keeps the first faces plus the current one at the end.
        let names = font_menu_names("yarr", &catalog, 3);
        assert_eq!(names, vec!["ahab", "bob", "yarr"]);
        // The current face is always present, even when it would overflow.
        let names = font_menu_names("zed", &catalog, 2);
        assert_eq!(names, vec!["ahab", "zed"]);
        assert_eq!(font_menu_names("only", &[], 1), vec!["only"]);
    }
}
