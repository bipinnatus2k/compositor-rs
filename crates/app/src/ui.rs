//! The Swift `UI/` mirror: one file per Swift source where a direct
//! counterpart exists, plus this root's small shared widget helpers —
//! the app palette, option dropdowns, labeled sliders, checkboxes and
//! text rows.

pub mod brush_controls;
pub mod canvas_size_sheet;
pub mod color_picker_sheet;
pub mod crop_controls;
pub mod curves_controls;
pub mod effects_sheet;
pub mod filter_sheet;
pub mod floating_panel;
pub mod hue_saturation_sheet;
pub mod image_size_sheet;
pub mod jpeg_export_sheet;
pub mod keyboard_shortcuts;
pub mod lasso_controls;
pub mod levels_sheet;
pub mod native_layer_list;
pub mod navigation_tool_header;
pub mod project_tabs;
pub mod shape_controls;
pub mod sheets;
pub mod transform_inspector;
pub mod trim_sheet;
pub mod type_controls;

use std::rc::Rc;

use gpui_kit::component::{
    Sizable,
    button::Button,
    checkbox::Checkbox,
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    slider::{Slider, SliderState},
};
use gpui_kit::{
    Context, ElementId, Entity, IntoElement, ParentElement, Pixels, SharedString, Styled,
    WeakEntity, Window, px,
};

use crate::state::AppState;

// The palette: Photoshop-like dark grays.
pub const BG_APP: u32 = 0x1e1e1e;
pub const BG_PANEL: u32 = 0x2d2d2d;
#[allow(dead_code)]
pub const BG_CONTROL: u32 = 0x373737;
pub const FG_TEXT: u32 = 0xd4d4d4;
pub const FG_DIM: u32 = 0x9a9a9a;
pub const ACCENT: u32 = 0x4da6ff;
/// The standard hairline border on cards, panels and sheets.
pub const BORDER: u32 = 0x4a4a4a;
/// Frame around sunken data content: editor plots and swatch cells.
pub const BORDER_INSET: u32 = 0x111111;
/// Layer-thumbnail frame.
pub const BORDER_THUMB: u32 = 0x444444;
/// Bright frame on data swatches.
pub const BORDER_BRIGHT: u32 = 0x606060;
/// The disabled/dimmed mid gray: eye-off glyphs, the empty-thumbnail fill,
/// anchor-dot frames.
pub const FG_DISABLED: u32 = 0x555555;
/// Sunken preview well (the camera-raw viewer's backdrop).
pub const BG_INSET: u32 = 0x1f1f1f;
/// Warning copy (camera-raw problem lines, welcome warnings).
pub const STATUS_WARNING: u32 = 0xe0a24a;
/// The active layer row's accent wash — ACCENT at ~20% alpha.
pub const ACCENT_WASH: u32 = 0x4da6ff33;
/// The sheet overlay's scrim.
pub const SCRIM: u32 = 0x00000099;
/// The canvas area's backdrop (the welcome landing surface included), and
/// the file-drop tints (area and tab strip).
pub const BG_CANVAS_AREA: u32 = 0x232326;
pub const BG_DROP_AREA: u32 = 0x2a2f3a;
pub const BG_DROP_STRIP: u32 = 0x33383f;
/// The inline-text caret's bar.
pub const BG_CARET: u32 = 0x1a1a1a;

#[allow(dead_code)]
pub fn app_bg() -> gpui_kit::Rgba {
    rgb_u32(BG_APP)
}

pub fn panel_bg() -> gpui_kit::Rgba {
    rgb_u32(BG_PANEL)
}

pub fn rgb_u32(hex: u32) -> gpui_kit::Rgba {
    gpui_kit::rgb(hex)
}

/// A handler that mutates the app state, run from menus and dropdowns.
pub type AppHandler = Rc<dyn Fn(&mut AppState, &mut Window, &mut Context<AppState>)>;

/// One entry of an option dropdown.
pub struct MenuEntry {
    pub label: String,
    pub checked: bool,
    pub enabled: bool,
    pub separator: bool,
    pub handler: AppHandler,
}

impl MenuEntry {
    pub fn new(
        label: impl Into<String>,
        handler: impl Fn(&mut AppState, &mut Window, &mut Context<AppState>) + 'static,
    ) -> Self {
        MenuEntry {
            label: label.into(),
            checked: false,
            enabled: true,
            separator: false,
            handler: Rc::new(handler),
        }
    }

    /// A separator row.
    pub fn separator() -> Self {
        MenuEntry {
            label: String::new(),
            checked: false,
            enabled: false,
            separator: true,
            handler: Rc::new(|_, _, _| {}),
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    #[allow(dead_code)]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.enabled = !disabled;
        self
    }
}

/// A small dropdown button whose menu calls back into the app state.
pub fn dropdown(
    app: &WeakEntity<AppState>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    width: Pixels,
    entries: Vec<MenuEntry>,
) -> impl gpui_kit::IntoElement {
    let app = app.clone();
    Button::new(id)
        .label(label)
        .outline()
        .small()
        .w(width)
        // Native caret affordance (gpui-component button.rs dropdown_caret);
        // no need to fake it with a "▾" text suffix.
        .dropdown_caret(true)
        .dropdown_menu(move |menu, _, _| build_menu(&app, menu, &entries))
}

pub(crate) fn build_menu(
    app: &WeakEntity<AppState>,
    mut menu: PopupMenu,
    entries: &[MenuEntry],
) -> PopupMenu {
    for entry in entries {
        if entry.separator {
            menu = menu.separator();
            continue;
        }
        let app = app.clone();
        let handler = entry.handler.clone();
        menu = menu.item(
            PopupMenuItem::new(entry.label.clone())
                .checked(entry.checked)
                .disabled(!entry.enabled)
                .on_click(move |_, window, cx| {
                    app.update(cx, |state, cx| handler(state, window, cx)).ok();
                }),
        );
    }
    menu
}

/// A compact labeled slider row: `Label [slider] value`.
pub fn labeled_slider(
    label: &str,
    state: &Entity<SliderState>,
    value: String,
    slider_width: Pixels,
) -> impl IntoElement {
    labeled_slider_row(label, None)
        .flex_shrink_0()
        .child(gpui_kit::div().w(slider_width).child(Slider::new(state)))
        .child(
            gpui_kit::div()
                .w(px(38.0))
                // ToolHeaderStyle.swift:7: the control font is 12.
                .text_size(px(12.0))
                .text_color(rgb_u32(FG_TEXT))
                .child(SharedString::from(value)),
        )
}

/// The labeled-slider row both slider surfaces share: a flex row whose
/// leading child is the dim control-font label, in a fixed-width column
/// when `label_width` is given. The options header adds a pixel-width
/// slider and the value readout ([`labeled_slider`]); the sheets and
/// panels add vertical padding and a full-width slider (sheets_ui.rs's
/// wrapper, whose shape is frozen by the panel bodies' call sites).
pub(crate) fn labeled_slider_row(label: &str, label_width: Option<Pixels>) -> gpui_kit::Div {
    let mut label_cell = gpui_kit::div()
        // ToolHeaderStyle.swift:7: the control font is 12.
        .text_size(px(12.0))
        .text_color(rgb_u32(FG_DIM));
    if let Some(width) = label_width {
        label_cell = label_cell.w(width).flex_shrink_0();
    }
    gpui_kit::div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .child(label_cell.child(SharedString::from(label.to_string())))
}

/// A checkbox with a label that writes the new value back through `handler`.
pub fn labeled_checkbox(
    app: &WeakEntity<AppState>,
    id: impl Into<ElementId>,
    label: &str,
    checked: bool,
    handler: impl Fn(&mut AppState, bool, &mut Window, &mut Context<AppState>) + 'static,
) -> impl IntoElement {
    let app = app.clone();
    let handler = Rc::new(handler);
    Checkbox::new(id)
        .label(label)
        .checked(checked)
        .on_change(move |checked, window, cx| {
            let handler = handler.clone();
            let checked = *checked;
            app.update(cx, move |state, cx| handler(state, checked, window, cx))
                .ok();
        })
        // ToolHeaderStyle.swift:7: the control font is 12.
        .text_size(px(12.0))
        // The checkbox root lays its indicator out items_start
        // (gpui-component checkbox.rs), which top-aligns the box against the
        // text at this small size; centering overrides the refinement.
        .items_center()
}

/// A plain text label in header/panel style.
pub fn text_label(label: impl Into<String>, dim: bool) -> gpui_kit::Div {
    gpui_kit::div()
        // ToolHeaderStyle.swift:7: the control font is 12.
        .text_size(px(12.0))
        .text_color(rgb_u32(if dim { FG_DIM } else { FG_TEXT }))
        .child(SharedString::from(label.into()))
}

/// A horizontal flex row with centered items and a small gap.
pub fn div_row() -> gpui_kit::Div {
    gpui_kit::div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .flex_shrink_0()
}
