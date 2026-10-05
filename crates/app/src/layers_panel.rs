//! The right panel: blend mode + opacity for the active layer, the layer
//! list (top-to-bottom), and the footer layer actions.

use std::sync::Arc;

use gpui_kit::{
    px, prelude::FluentBuilder as _, rgb, AnyElement, Context, ClickEvent, IntoElement,
    InteractiveElement, ParentElement, RenderImage, SharedString, StatefulInteractiveElement,
    Styled, Window,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable;

use compositor_core::blend::LayerBlendMode;
use compositor_core::document::ImageLayer;
use compositor_core::effects::LayerEffectKind;
use compositor_core::selection::SelectionMode;

use crate::state::AppState;
use crate::ui::{dropdown, text_label, MenuEntry};

const THUMB: f32 = 30.0;

fn click_count(event: &ClickEvent) -> usize {
    match event {
        ClickEvent::Mouse(mouse) => mouse.up.click_count,
        _ => 0,
    }
}

/// Shift adds, Option subtracts, plain replaces — the thumbnail-click
/// selection cascade (NativeLayerList.swift:785-789), written once.
fn selection_mode(shift: bool, alt: bool) -> SelectionMode {
    if shift {
        SelectionMode::Add
    } else if alt {
        SelectionMode::Subtract
    } else {
        SelectionMode::Replace
    }
}

/// The shared thumbnail cell: square, rounded, hairline border; the accent
/// ring marks the edit target, a disabled mask reads dimmed, and an image
/// fills the cell when one is on hand.
fn thumbnail_cell(
    element_id: String,
    is_target: bool,
    dimmed: bool,
    image: Option<Arc<RenderImage>>,
    on_click: impl Fn(&mut AppState, &ClickEvent, &mut Window, &mut Context<AppState>) + 'static,
    cx: &Context<AppState>,
) -> AnyElement {
    gpui_kit::div()
        .id(SharedString::from(element_id))
        .w(px(THUMB))
        .h(px(THUMB))
        .flex_shrink_0()
        .rounded_sm()
        .overflow_hidden()
        .border_1()
        .border_color(gpui_kit::rgb(0x444444))
        .bg(rgb(0x555555))
        .when(is_target, |cell| {
            cell.border_2()
                .border_color(gpui_kit::rgb(crate::ui::ACCENT))
        })
        .when(dimmed, |cell| cell.opacity(0.4))
        .when_some(image, |cell, image| {
            cell.child(gpui_kit::img(image).size_full())
        })
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            on_click(this, event, window, cx);
        }))
        .into_any_element()
}

impl AppState {
    /// Puts the active layer's opacity into the header slider.
    pub fn sync_opacity_slider(&mut self, window: &mut gpui_kit::Window, cx: &mut Context<Self>) {
        let Some(layer) = self.tab().session.active_layer() else { return };
        let value = (layer.opacity * 100.0) as f32;
        let slider = self.opacity_slider.clone();
        slider.update(cx, |state, cx| {
            state.set_value(gpui_kit::component::slider::SliderValue::Single(value), window, cx)
        });
    }

    pub fn render_layers_panel(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let app = cx.entity().downgrade();
        let mut panel = gpui_kit::div()
            .flex()
            .flex_col()
            .w(px(248.0))
            .flex_shrink_0()
            .bg(crate::ui::panel_bg())
            .border_l_1()
            .border_color(gpui_kit::black().opacity(0.4));

        // Header: title + blend mode + opacity.
        let can_edit = self.tab().session.can_edit_appearance();
        let session = self.current_tab();
        let blend_label = session
            .session
            .active_layer()
            .map(|layer| {
                session.session
                    .displayed_blend_mode(layer)
                    .label()
                    .to_string()
            })
            .unwrap_or_else(|| "Normal".to_string());
        let opacity = session
            .session
            .active_layer()
            .map(|layer| layer.opacity)
            .unwrap_or(1.0);

        panel = panel.child(
            gpui_kit::div()
                .flex()
                .flex_col()
                .gap_1()
                .px_2()
                .py_1p5()
                .border_b_1()
                .border_color(gpui_kit::black().opacity(0.4))
                .child(
                    text_label("Layers", false)
                        .text_size(px(12.0))
                        .flex_shrink_0(),
                )
                .child(
                    gpui_kit::div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(text_label("Blend", true))
                        .child(dropdown(
                            &app,
                            "blend-mode",
                            blend_label,
                            px(130.0),
                            self.blend_menu_entries(),
                        ))
                        .child(text_label("Opacity", true))
                        .child(
                            gpui_kit::div().w(px(60.0)).child(gpui_kit::component::slider::Slider::new(
                                &self.opacity_slider,
                            )),
                        )
                        .child(text_label(format!("{:.0}%", opacity * 100.0), false)),
                ),
        );

        // The layer list, top to bottom. The rows are the hierarchy's
        // (`layerRows`, session.rs layer_rows: `hierarchy_entries(true, …)`):
        // `top_first` reverses siblings, so a document without groups
        // enumerates exactly like the former flat `layers.iter().rev()` pass,
        // and a collapsed group simply omits its children.
        let mut list = gpui_kit::div()
            .id("layer-list")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        let collapsed = self.tab().session.collapsed_group_ids.clone();
        if let Some(document) = self.tab().session.document.clone() {
            let active_id = self.tab().session.active_layer_id;
            for entry in document.hierarchy_entries(true, &collapsed) {
                let layer = entry.layer;
                let id = layer.id;
                let is_active = active_id == Some(id);
                let index = document.layer_index(id).unwrap_or(0);
                let row = self.render_layer_row(layer, is_active, index, entry.depth, cx);
                list = list.child(row);
            }
        } else {
            list = list.child(
                gpui_kit::div()
                    .p_3()
                    .child(text_label("No document", true)),
            );
        }
        panel = panel.child(list);

        // Footer buttons (Lucide glyphs; tooltips keep the wording).
        let fx_entries = self.effects_menu_entries();
        panel = panel.child(
            gpui_kit::div()
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .gap_1()
                .px_1()
                .py_1()
                .border_t_1()
                .border_color(gpui_kit::black().opacity(0.4))
                .child(footer_button("layer-add", IconName::Plus, "New Blank Layer", cx.listener(
                    |this, _, _, cx| {
                        this.tab().session.add_blank_layer();
                        this.touch(cx);
                    },
                )))
                .child(footer_button("layer-mask", IconName::SquareDashedBottom, "Add Reveal-All Mask", cx.listener(
                    |this, _, _, cx| {
                        this.tab().session.add_layer_mask(true);
                        this.touch(cx);
                    },
                )))
                .child(footer_button("layer-group", IconName::FolderPlus, "New Group", cx.listener(
                    |this, _, _, cx| {
                        this.add_group(cx);
                    },
                )))
                // The effects menu (LayersPanel.swift's sparkles): one entry
                // per effect kind; choosing one adds it to the active layer
                // and opens its editor sheet.
                .child(
                    Button::new("layer-fx")
                        .icon(Icon::new(IconName::Sparkles))
                        .accessibility_label("Layer Effects")
                        .ghost()
                        .small()
                        .tooltip("Layer Effects")
                        // Same caret affordance as the ui::dropdown helper
                        // (gpui-component button.rs dropdown_caret).
                        .dropdown_caret(true)
                        .dropdown_menu({
                            let app = app.clone();
                            move |menu, _, _| crate::ui::build_menu(&app, menu, &fx_entries)
                        }),
                )
                .child(footer_button("layer-dup", IconName::Copy, "Duplicate Layer", cx.listener(
                    |this, _, _, cx| {
                        this.tab().session.duplicate_active_layer();
                        this.touch(cx);
                    },
                )))
                .child(footer_button("layer-del", IconName::Trash, "Delete Layer", cx.listener(
                    |this, _, _, cx| {
                        this.tab().session.delete_active_layer();
                        this.touch(cx);
                    },
                )))
                .child(footer_button("layer-up", IconName::ArrowUp, "Move Layer Up", cx.listener(
                    |this, _, _, cx| {
                        this.tab().session.move_active_layer(1);
                        this.touch(cx);
                    },
                )))
                .child(footer_button("layer-down", IconName::ArrowDown, "Move Layer Down", cx.listener(
                    |this, _, _, cx| {
                        this.tab().session.move_active_layer(-1);
                        this.touch(cx);
                    },
                ))),
        );

        let _ = (&app, can_edit);
        panel.into_any_element()
    }

    /// The layer-effect kinds (LayersPanel.swift's sparkles menu), disabled
    /// unless the active layer can take effects.
    fn effects_menu_entries(&self) -> Vec<MenuEntry> {
        let enabled = self.current_tab().session.can_edit_effects();
        LayerEffectKind::ALL
            .iter()
            .map(|kind| {
                let kind = *kind;
                MenuEntry::new(format!("{}…", kind.label()), move |state, window, cx| {
                    state.add_effect(kind, window, cx);
                })
                .disabled(!enabled)
            })
            .collect()
    }

    /// The 24 blend modes, grouped with separators like the original menu.
    fn blend_menu_entries(&self) -> Vec<MenuEntry> {
        let current = self
            .current_tab().session
            .active_layer()
            .map(|layer| layer.blend_mode);
        let mut entries = Vec::new();
        for (group_index, group) in LayerBlendMode::GROUPS.iter().enumerate() {
            if group_index > 0 {
                entries.push(MenuEntry::separator());
            }
            for mode in group.iter() {
                let mode = *mode;
                entries.push(
                    MenuEntry::new(mode.label(), move |state, _, cx| {
                        state.tab().session.set_layer_blend_mode(mode);
                        state.touch(cx);
                    })
                    .checked(current == Some(mode)),
                );
            }
        }
        entries
    }

    fn render_layer_row(
        &mut self,
        layer: &ImageLayer,
        is_active: bool,
        index: usize,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let id = layer.id;
        let visible = layer.is_visible;
        let is_group = layer.is_group;
        let has_mask = layer.mask.is_some();
        let mask_enabled = layer.mask.as_ref().map(|m| m.is_enabled).unwrap_or(false);
        let mask_selected = self.tab().session.is_mask_selected;
        let collapsed = self.tab().session.collapsed_group_ids.contains(&id);
        let in_folder = layer.parent_id.is_some();
        let rename_input = self.rename == Some(id);
        // Swift indents one 24px step per hierarchy level, capped at 8
        // (NativeLayerList.swift:625-631).
        let indent = (depth.min(8) as f32) * 24.0;
        let app = cx.entity().downgrade();

        let mut row = gpui_kit::div()
            .id(SharedString::from(format!("layer-row-{}", index)))
            .flex()
            .flex_row()
            .items_center()
            .gap_1p5()
            .px_2()
            .py_1()
            .mx_1()
            .rounded_sm()
            .on_click(cx.listener(move |this, event, window, cx| {
                let double = click_count(event) >= 2;
                if this.rename == Some(id) {
                    return;
                }
                this.tab().session.select_layer(Some(id));
                this.sync_opacity_slider(window, cx);
                if double {
                    if is_group {
                        // Double-clicking a folder toggles its expansion
                        // instead of renaming it (the row's disclosure).
                        this.tab().session.toggle_group_expansion(id);
                    } else {
                        this.begin_rename(id, window, cx);
                    }
                }
                this.touch(cx);
            }))
            .when(is_active, |row| {
                row.bg(gpui_kit::rgba(0x4da6ff33))
            });

        // Indent: one 24px step per level; the folder's chevron closes the
        // indent region.
        row = row.child(gpui_kit::div().w(px(indent)).flex_shrink_0());
        if is_group {
            row = row.child(
                gpui_kit::div()
                    .id(SharedString::from(format!("layer-chevron-{}", index)))
                    // A generous hit area: an 18px box inside a 24px indent
                    // step is easy to miss (Swift widens footer hit areas
                    // for the same reason, NativeLayerList.swift:79-82).
                    .w(px(24.0))
                    .h(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab().session.toggle_group_expansion(id);
                        this.touch(cx);
                        cx.stop_propagation();
                    }))
                    .child(
                        Icon::new(if collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .size(px(14.0))
                        .text_color(gpui_kit::rgb(crate::ui::FG_DIM)),
                    ),
            );
        }

        // Eye (Lucide eye / eye-off, both in the bundled set).
        row = row.child(
            gpui_kit::div()
                .id(SharedString::from(format!("layer-eye-{}", index)))
                .w(px(18.0))
                .h(px(18.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab().session.toggle_layer_visibility(id);
                    this.touch(cx);
                }))
                .child(
                    Icon::new(if visible { IconName::Eye } else { IconName::EyeOff })
                        .size(px(14.0))
                        .text_color(if visible {
                            gpui_kit::rgb(crate::ui::FG_TEXT)
                        } else {
                            gpui_kit::rgb(0x555555)
                        }),
                ),
        );

        // Thumbnail. A plain click makes the layer the edit target
        // (selectImage); Cmd-click loads the layer's (or the mask's) pixels
        // as the selection, Shift adds and Option subtracts (MaskTracing).
        // The active edit target's thumbnail carries the accent border
        // (NativeLayerList.swift:712-715).
        let thumbnail = self.thumbnail_image(id, layer);
        let layer_is_target = is_active && !(mask_selected && has_mask);
        row = row.child(thumbnail_cell(
            format!("layer-thumb-{}", index),
            layer_is_target,
            false,
            thumbnail,
            move |this, event, window, cx| {
                let modifiers = event.modifiers();
                if !modifiers.control && !modifiers.platform {
                    this.tab().session.select_layer_target(id, false);
                    this.sync_opacity_slider(window, cx);
                    this.touch(cx);
                    cx.stop_propagation();
                    return;
                }
                let mode = selection_mode(modifiers.shift, modifiers.alt);
                if this.tab().session.is_mask_selected {
                    this.tab().session.load_mask_selection(id, mode);
                } else {
                    this.tab().session.load_layer_selection(id, mode);
                }
                this.touch(cx);
            },
            cx,
        ));

        // The mask thumbnail (the row's maskThumbnail): the mask placed on
        // its canvas frame at the layer thumbnail's size; a plain click makes
        // the mask the edit target, Cmd-click loads its coverage as the
        // selection. A disabled mask reads dimmed.
        if layer.mask.is_some() {
            let mask_image = self.mask_thumbnail_image(layer);
            row = row.child(thumbnail_cell(
                format!("layer-mask-thumb-{}", index),
                is_active && mask_selected,
                !mask_enabled,
                mask_image,
                move |this, event, window, cx| {
                    let modifiers = event.modifiers();
                    if !modifiers.control && !modifiers.platform {
                        this.tab().session.select_layer_target(id, true);
                        this.sync_opacity_slider(window, cx);
                        this.touch(cx);
                        cx.stop_propagation();
                        return;
                    }
                    let mode = selection_mode(modifiers.shift, modifiers.alt);
                    this.tab().session.load_mask_selection(id, mode);
                    this.touch(cx);
                },
                cx,
            ));
        }

        // Name (or the rename input).
        let mut name_cell = gpui_kit::div().flex_1().min_w_0();
        if rename_input {
            if let Some(input) = self.rename_input.clone() {
                name_cell = name_cell.child(
                    gpui_kit::component::input::Input::new(&input)
                        .id("rename-input")
                        .h(px(20.0))
                        .text_size(px(11.0)),
                );
            }
        } else {
            let name = layer.name.clone();
            name_cell = name_cell.child(
                gpui_kit::div()
                    .id(SharedString::from(format!("layer-name-{}", index)))
                    .text_size(px(11.0))
                    .text_color(gpui_kit::rgb(crate::ui::FG_TEXT))
                    .overflow_hidden()
                    .text_ellipsis()
                    .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                        if event.click_count() >= 2 {
                            this.begin_rename(id, window, cx);
                            // A double-click on the name is a rename, not the
                            // row's folder toggle.
                            cx.stop_propagation();
                        }
                    }))
                    .child(SharedString::from(name)),
            );
        }
        row = row.child(name_cell);

        // The row's right-click menu (NativeLayerList.swift:604-613, cropped
        // to what the session models). Opening it first activates the row —
        // Swift's menu actions all run through selectImage().
        row.context_menu(move |menu, window, cx| {
            app_activate(&app, id, window, cx);
            let menu = menu
                .item(PopupMenuItem::new("Rename…").on_click({
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |state, cx| state.begin_rename(id, window, cx)).ok();
                    }
                }))
                .item({
                    let app = app.clone();
                    PopupMenuItem::new(if visible { "Hide" } else { "Show" }).on_click(
                        move |_, _, cx| {
                            app.update(cx, |state, cx| {
                                state.tab().session.toggle_layer_visibility(id);
                                state.touch(cx);
                            })
                            .ok();
                        },
                    )
                })
                .separator();
            let menu = if has_mask {
                menu.item({
                    let app = app.clone();
                    PopupMenuItem::new(if mask_enabled {
                        "Disable Mask"
                    } else {
                        "Enable Mask"
                    })
                    .on_click(move |_, _, cx| {
                        app.update(cx, |state, cx| {
                            state.tab().session.toggle_layer_mask();
                            state.touch(cx);
                        })
                        .ok();
                    })
                })
                .item({
                    let app = app.clone();
                    PopupMenuItem::new("Delete Mask").on_click(move |_, _, cx| {
                        app.update(cx, |state, cx| {
                            state.tab().session.delete_layer_mask();
                            state.touch(cx);
                        })
                        .ok();
                    })
                })
            } else {
                menu.item({
                    let app = app.clone();
                    PopupMenuItem::new("Add White Mask").on_click(move |_, _, cx| {
                        app.update(cx, |state, cx| {
                            state.tab().session.add_layer_mask(true);
                            state.touch(cx);
                        })
                        .ok();
                    })
                })
                .item({
                    let app = app.clone();
                    PopupMenuItem::new("Add Black Mask").on_click(move |_, _, cx| {
                        app.update(cx, |state, cx| {
                            state.tab().session.add_layer_mask(false);
                            state.touch(cx);
                        })
                        .ok();
                    })
                })
            };
            let menu = menu.separator();
            let menu = if in_folder {
                menu.item({
                    let app = app.clone();
                    PopupMenuItem::new("Move Out of Folder").on_click(move |_, _, cx| {
                        app.update(cx, |state, cx| {
                            state.tab().session.move_active_layer_out_of_group();
                            state.touch(cx);
                        })
                        .ok();
                    })
                })
            } else {
                menu
            };
            menu.item({
                let app = app.clone();
                PopupMenuItem::new("Duplicate Layer").on_click(move |_, _, cx| {
                    app.update(cx, |state, cx| {
                        state.tab().session.layer_via_copy();
                        state.touch(cx);
                    })
                    .ok();
                })
            })
            .item({
                let app = app.clone();
                PopupMenuItem::new("Delete Layer").on_click(move |_, window, cx| {
                    // deleteLayerOrMask: deletes the mask when it is the target,
                    // and asks about live-mask dependents otherwise
                    // (request_delete_layer).
                    app.update(cx, |state, cx| state.request_delete_layer(window, cx))
                        .ok();
                })
            })
        })
        .into_any_element()
    }

    /// The row's mask thumbnail: the mask's own pixels placed where they sit
    /// on a canvas-shaped frame filled with the mask's edge tone
    /// (`CanvasThumbnail.mask`, render.rs mask_canvas_thumbnail) — the same
    /// producer `thumbnail_image` uses when the mask is the row's subject.
    fn mask_thumbnail_image(&self, layer: &ImageLayer) -> Option<Arc<RenderImage>> {
        let mask = layer.mask.as_ref()?;
        let document = self.current_tab().session.document.as_ref()?;
        let canvas = compositor_core::geometry::CGSize::new(
            document.width as f64,
            document.height as f64,
        );
        let thumbnail = compositor_core::render::mask_canvas_thumbnail(
            mask,
            &layer.mask_transform(),
            canvas,
        )?;
        Some(crate::state::bgra_render_image(&thumbnail))
    }

    /// New Group: a folder inserted above the active layer, which becomes
    /// its first child.
    pub fn add_group(&mut self, cx: &mut Context<Self>) {
        self.tab().session.add_group();
        self.touch(cx);
    }
}

/// The right-click activation: the row becomes the active layer, with the
/// header's blend/opacity readouts following it, before any menu item runs.
fn app_activate(
    app: &gpui_kit::WeakEntity<AppState>,
    id: uuid::Uuid,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<gpui_kit::component::menu::PopupMenu>,
) {
    app.update(cx, |state, cx| {
        state.tab().session.select_layer(Some(id));
        state.sync_opacity_slider(window, cx);
        state.touch(cx);
    })
    .ok();
}

fn footer_button(
    id: &'static str,
    icon: IconName,
    tooltip: &'static str,
    listener: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static,
) -> Button {
    Button::new(id)
        .icon(Icon::new(icon))
        // Icon-only buttons have no label to read aloud; the tooltip wording
        // doubles as the announced one.
        .accessibility_label(tooltip)
        .ghost()
        .small()
        .tooltip(tooltip)
        .on_click(listener)
}

#[cfg(test)]
mod tests {
    use super::selection_mode;
    use compositor_core::selection::SelectionMode;

    /// Shift adds, Option subtracts, plain replaces; shift wins when both
    /// modifiers ride along (NativeLayerList.swift:785-789).
    #[test]
    fn selection_mode_follows_shift_then_alt() {
        assert!(matches!(selection_mode(false, false), SelectionMode::Replace));
        assert!(matches!(selection_mode(true, false), SelectionMode::Add));
        assert!(matches!(selection_mode(false, true), SelectionMode::Subtract));
        assert!(matches!(selection_mode(true, true), SelectionMode::Add));
    }
}
