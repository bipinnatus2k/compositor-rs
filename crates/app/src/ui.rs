//! The Swift `UI/` mirror: one file per Swift source where a direct
//! counterpart exists, plus this root's small shared widget structs —
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
    Disableable, Sizable,
    button::Button,
    checkbox::Checkbox,
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    slider::{Slider, SliderState},
};
use gpui_kit::{
    App, ClickEvent, Context, ElementId, Entity, IntoElement, MouseDownEvent, MouseMoveEvent,
    ParentElement, Pixels, RenderOnce, SharedString, Styled, WeakEntity, Window, px,
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
/// Transitional until AppState dissolves: menu entries will then speak in
/// session terms, and only the dispatch adapters (not the components)
/// change with it.
pub type AppHandler = Rc<dyn Fn(&mut AppState, &mut Window, &mut Context<AppState>)>;

/// Runs one picked menu entry's handler. Which entity owns the state the
/// handler mutates is the dispatcher's business, not the dropdown's.
pub type MenuDispatch = Rc<dyn Fn(&AppHandler, &mut Window, &mut App)>;

/// The app-shell dispatch: run the entry's handler inside the app entity.
pub fn app_dispatch(app: &WeakEntity<AppState>) -> MenuDispatch {
    let app = app.clone();
    Rc::new(move |handler, window, cx| {
        let handler = handler.clone();
        app.update(cx, |state, cx| handler(state, window, cx)).ok();
    })
}

/// A component's value callback at `App` level: the value a component
/// chose (a checkbox's new state, a picked anchor cell) with the window
/// and app contexts.
pub type AppCallback<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

/// The app-shell adapter for value callbacks: run an AppState-flavored
/// handler inside the app entity.
pub fn app_on<T: 'static>(
    app: &WeakEntity<AppState>,
    handler: impl Fn(&mut AppState, T, &mut Window, &mut Context<AppState>) + 'static,
) -> AppCallback<T> {
    let app = app.clone();
    let handler = Rc::new(handler);
    Rc::new(move |value, window, cx| {
        let handler = handler.clone();
        app.update(cx, move |state, cx| handler(state, value, window, cx))
            .ok();
    })
}

/// A component's click callback at `App` level.
pub type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// The app-shell adapter for click callbacks.
pub fn app_on_click(
    app: &WeakEntity<AppState>,
    handler: impl Fn(&mut AppState, &ClickEvent, &mut Window, &mut Context<AppState>) + 'static,
) -> ClickHandler {
    let app = app.clone();
    let handler = Rc::new(handler);
    Rc::new(move |event, window, cx| {
        let handler = handler.clone();
        app.update(cx, move |state, cx| handler(state, event, window, cx))
            .ok();
    })
}

/// A component's press callback at `App` level (press-capture drags start
/// here, so the event's position is the drag's origin).
pub type PressHandler = Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>;

/// The app-shell adapter for press callbacks.
pub fn app_on_press(
    app: &WeakEntity<AppState>,
    handler: impl Fn(&mut AppState, &MouseDownEvent, &mut Window, &mut Context<AppState>) + 'static,
) -> PressHandler {
    let app = app.clone();
    let handler = Rc::new(handler);
    Rc::new(move |event, window, cx| {
        let handler = handler.clone();
        app.update(cx, move |state, cx| handler(state, event, window, cx))
            .ok();
    })
}

/// A component's drag-move callback at `App` level (window-level move
/// events forwarded while the component's drag is live).
pub type DragMoveHandler = Rc<dyn Fn(&MouseMoveEvent, &mut Window, &mut App)>;

/// The app-shell adapter for drag-move callbacks.
pub fn app_on_drag_move(
    app: &WeakEntity<AppState>,
    handler: impl Fn(&mut AppState, &MouseMoveEvent, &mut Window, &mut Context<AppState>) + 'static,
) -> DragMoveHandler {
    let app = app.clone();
    let handler = Rc::new(handler);
    Rc::new(move |event, window, cx| {
        let handler = handler.clone();
        app.update(cx, move |state, cx| handler(state, event, window, cx))
            .ok();
    })
}

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

/// The app-shell menu assembly: one `PopupMenu` item per entry, each click
/// running the entry's handler inside the app entity. Shared by the
/// layers-panel footer menus' `dropdown_menu` closures.
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

/// A small dropdown button whose menu is dispatched through `MenuDispatch`.
#[derive(IntoElement)]
pub struct Dropdown {
    id: ElementId,
    label: SharedString,
    width: Pixels,
    disabled: bool,
    entries: Vec<MenuEntry>,
    dispatch: MenuDispatch,
}

impl Dropdown {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Dropdown {
            id: id.into(),
            label: label.into(),
            width: px(80.0),
            disabled: false,
            entries: Vec::new(),
            dispatch: Rc::new(|_, _, _| {}),
        }
    }

    pub fn with_width(mut self, width: Pixels) -> Self {
        self.width = width;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn with_entries(mut self, entries: Vec<MenuEntry>) -> Self {
        self.entries = entries;
        self
    }

    pub fn with_dispatch(mut self, dispatch: MenuDispatch) -> Self {
        self.dispatch = dispatch;
        self
    }
}

impl RenderOnce for Dropdown {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let dispatch = self.dispatch;
        Button::new(self.id)
            .label(self.label)
            .outline()
            .small()
            .w(self.width)
            .disabled(self.disabled)
            // Native caret affordance (gpui-component button.rs dropdown_caret);
            // no need to fake it with a "▾" text suffix.
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| menu_with_dispatch(menu, &self.entries, &dispatch))
    }
}

/// Wire one dropdown's entries into a `PopupMenu`, handing every click to
/// `dispatch` with the entry's handler.
fn menu_with_dispatch(
    mut menu: PopupMenu,
    entries: &[MenuEntry],
    dispatch: &MenuDispatch,
) -> PopupMenu {
    for entry in entries {
        if entry.separator {
            menu = menu.separator();
            continue;
        }
        let dispatch = dispatch.clone();
        let handler = entry.handler.clone();
        menu = menu.item(
            PopupMenuItem::new(entry.label.clone())
                .checked(entry.checked)
                .disabled(!entry.enabled)
                .on_click(move |_, window, cx| dispatch(&handler, window, cx)),
        );
    }
    menu
}

/// A checkbox with a label; the flipped value is reported through
/// `on_change` — the checkbox owns no truth of its own.
#[derive(IntoElement)]
pub struct LabeledCheckbox {
    id: ElementId,
    label: SharedString,
    checked: bool,
    on_change: AppCallback<bool>,
}

impl LabeledCheckbox {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        LabeledCheckbox {
            id: id.into(),
            label: label.into(),
            checked: false,
            on_change: Rc::new(|_, _, _| {}),
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn with_on_change(mut self, on_change: AppCallback<bool>) -> Self {
        self.on_change = on_change;
        self
    }
}

impl RenderOnce for LabeledCheckbox {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        Checkbox::new(self.id)
            .label(self.label)
            .checked(self.checked)
            .on_change(move |checked, window, cx| (self.on_change)(*checked, window, cx))
            // ToolHeaderStyle.swift:7: the control font is 12.
            .text_size(px(12.0))
            // The checkbox root lays its indicator out items_start
            // (gpui-component checkbox.rs), which top-aligns the box against the
            // text at this small size; centering overrides the refinement.
            .items_center()
    }
}

/// The header bar's labeled slider row: `Label [slider] value`.
#[derive(IntoElement)]
pub struct LabeledSlider {
    label: SharedString,
    slider: Entity<SliderState>,
    value: SharedString,
    slider_width: Pixels,
}

impl LabeledSlider {
    pub fn new(
        label: impl Into<SharedString>,
        slider: Entity<SliderState>,
        value: impl Into<SharedString>,
        slider_width: Pixels,
    ) -> Self {
        LabeledSlider {
            label: label.into(),
            slider,
            value: value.into(),
            slider_width,
        }
    }
}

impl RenderOnce for LabeledSlider {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        slider_row(self.label, None)
            .flex_shrink_0()
            .child(
                gpui_kit::div()
                    .w(self.slider_width)
                    .child(Slider::new(&self.slider)),
            )
            .child(
                gpui_kit::div()
                    .w(px(38.0))
                    // ToolHeaderStyle.swift:7: the control font is 12.
                    .text_size(px(12.0))
                    .text_color(rgb_u32(FG_TEXT))
                    .child(self.value),
            )
    }
}

/// The labeled-row spine the slider rows share: a flex row whose leading
/// child is the dim control-font label, in a fixed-width column when
/// `label_width` is given. The header bar adds a pixel-width slider and the
/// value readout ([`LabeledSlider`]); the sheets use [`crate::ui::sheets_ui::SheetRow`]
/// for the padded 110px variant.
fn slider_row(label: SharedString, label_width: Option<Pixels>) -> gpui_kit::Div {
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
        .child(label_cell.child(label))
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
