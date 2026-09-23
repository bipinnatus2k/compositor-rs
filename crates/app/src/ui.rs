//! Small shared UI helpers: the app palette, option dropdowns, labeled
//! sliders, checkboxes and text rows.

use std::rc::Rc;

use gpui_kit::{
    px, Context, ElementId, Entity, IntoElement, ParentElement, Pixels, SharedString, Styled,
    WeakEntity, Window,
};
use gpui_kit::component::{
    button::Button,
    checkbox::Checkbox,
    menu::{DropdownMenu, PopupMenuItem, PopupMenu},
    slider::{Slider, SliderState},
    Sizable,
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
        .dropdown_menu(move |menu, _, _| build_menu(&app, menu, &entries))
}

fn build_menu(app: &WeakEntity<AppState>, mut menu: PopupMenu, entries: &[MenuEntry]) -> PopupMenu {
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
    div_row()
        .child(
            gpui_kit::div()
                .text_size(px(11.0))
                .text_color(rgb_u32(FG_DIM))
                .child(SharedString::from(label.to_string())),
        )
        .child(gpui_kit::div().w(slider_width).child(Slider::new(state)))
        .child(
            gpui_kit::div()
                .w(px(38.0))
                .text_size(px(11.0))
                .text_color(rgb_u32(FG_TEXT))
                .child(SharedString::from(value)),
        )
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
        .text_size(px(11.0))
}

/// A plain text label in header/panel style.
pub fn text_label(label: impl Into<String>, dim: bool) -> gpui_kit::Div {
    gpui_kit::div()
        .text_size(px(11.0))
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
