//! Port of the Type tool's option bar (`TypeControls.swift`): the bar's
//! render body plus its font-catalog and alignment helpers.

use gpui_kit::component::Selectable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, px,
};

use crate::state::AppState;
use crate::ui::{MenuEntry, div_row, dropdown, text_label};

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
/// glyphs, so the labels carry the meaning.
pub(crate) fn alignment_buttons(
    app: &gpui_kit::WeakEntity<AppState>,
    selected: compositor_core::document::TextAlignment,
) -> gpui_kit::Div {
    let mut row = gpui_kit::div().flex().flex_row().gap_0p5().flex_shrink_0();
    for (alignment, label) in [
        (compositor_core::document::TextAlignment::Left, "Left"),
        (compositor_core::document::TextAlignment::Center, "Center"),
        (compositor_core::document::TextAlignment::Right, "Right"),
    ] {
        row = row.child(
            Button::new(SharedString::from(format!("type-align-{}", label)))
                .label(label)
                .ghost()
                .compact()
                .selected(selected == alignment)
                .on_click({
                    let app = app.clone();
                    move |_, _, cx| {
                        app.update(cx, |state, cx| {
                            state.tab().session.update(cx, |session, _| {
                                session.change_text_style(move |style| {
                                    style.alignment = alignment;
                                })
                            });
                            state.touch(cx);
                        })
                        .ok();
                    }
                }),
        );
    }
    row
}

impl AppState {
    /// `TypeControls.swift`'s bar controls: font, size, color, alignment,
    /// tracking, leading, and the draft buttons (Cancel/Done while a text
    /// draft is open, Edit Text otherwise).
    pub(crate) fn type_controls(&mut self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let app = cx.entity().downgrade();
        let style = self.tab().session.read(cx).current_text_style();
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
        let color = self.tab().session.read(cx).type_color();
        let color_rgb = (rgb(color.red) << 16) | (rgb(color.green) << 8) | rgb(color.blue);
        let (has_document, has_draft, active_is_text) = {
            let session = self.tab().session.read(cx);
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
            dropdown(&app, "type-font", font, px(150.0), entries).into_any_element(),
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
                    this.open_text_color_picker_ui(window, cx);
                }))
                .into_any_element(),
            alignment_buttons(&app, style.alignment).into_any_element(),
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
                        this.tab()
                            .session
                            .update(cx, |session, _| session.cancel_text());
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
                        this.tab()
                            .session
                            .update(cx, |session, _| session.finish_text());
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
                        this.tab()
                            .session
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
