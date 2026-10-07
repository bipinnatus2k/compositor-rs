//! Port of the Type tool's option bar (`TypeControls.swift`). The bar's
//! render body still lives in the options header (ui/navigation_tool_header.rs)
//! until the per-tool carve; this file holds its font-catalog and alignment
//! helpers.

use gpui_kit::component::Selectable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::{ParentElement, SharedString, Styled};

use crate::state::AppState;

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
                            state.tab().session.change_text_style(move |style| {
                                style.alignment = alignment;
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
