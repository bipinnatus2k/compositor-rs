//! The right panel: blend mode + opacity for the active layer, the layer
//! list (top-to-bottom), and the footer layer actions.

use gpui_kit::{
    px, prelude::FluentBuilder as _, rgb, Context, ClickEvent, IntoElement, InteractiveElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;

use compositor_core::blend::LayerBlendMode;
use compositor_core::document::ImageLayer;

use crate::state::AppState;
use crate::ui::{dropdown, text_label, MenuEntry};

const THUMB: f32 = 30.0;

fn click_count(event: &ClickEvent) -> usize {
    match event {
        ClickEvent::Mouse(mouse) => mouse.up.click_count,
        _ => 0,
    }
}

impl AppState {
    /// Puts the active layer's opacity into the header slider.
    pub fn sync_opacity_slider(&mut self, window: &mut gpui_kit::Window, cx: &mut Context<Self>) {
        let Some(layer) = self.session.active_layer() else { return };
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
        let can_edit = self.session.can_edit_appearance();
        let blend_label = self
            .session
            .active_layer()
            .map(|layer| {
                self.session
                    .displayed_blend_mode(layer)
                    .label()
                    .to_string()
            })
            .unwrap_or_else(|| "Normal".to_string());
        let opacity = self
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

        // The layer list, top to bottom.
        let mut list = gpui_kit::div()
            .id("layer-list")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        if let Some(document) = self.session.document.clone() {
            let active_id = self.session.active_layer_id;
            for layer in document.layers.iter().rev() {
                let id = layer.id;
                let is_active = active_id == Some(id);
                let index = document.layer_index(id).unwrap_or(0);
                let row = self.render_layer_row(layer, is_active, index, cx);
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

        // Footer buttons.
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
                .child(footer_button("layer-add", "+", "New Blank Layer", cx.listener(
                    |this, _, _, cx| {
                        this.session.add_blank_layer();
                        this.touch(cx);
                    },
                )))
                .child(footer_button("layer-mask", "Mask", "Add Reveal-All Mask", cx.listener(
                    |this, _, _, cx| {
                        this.session.add_layer_mask(true);
                        this.touch(cx);
                    },
                )))
                .child(footer_button("layer-group", "Grp", "New Group", cx.listener(
                    |this, _, _, cx| {
                        this.add_group(cx);
                    },
                )))
                .child(footer_button("layer-dup", "Dup", "Duplicate Layer", cx.listener(
                    |this, _, _, cx| {
                        this.session.duplicate_active_layer();
                        this.touch(cx);
                    },
                )))
                .child(footer_button("layer-del", "Del", "Delete Layer", cx.listener(
                    |this, _, _, cx| {
                        this.session.delete_active_layer();
                        this.touch(cx);
                    },
                )))
                .child(footer_button("layer-up", "Up", "Move Layer Up", cx.listener(
                    |this, _, _, cx| {
                        this.session.move_active_layer(1);
                        this.touch(cx);
                    },
                )))
                .child(footer_button("layer-down", "Dn", "Move Layer Down", cx.listener(
                    |this, _, _, cx| {
                        this.session.move_active_layer(-1);
                        this.touch(cx);
                    },
                ))),
        );

        let _ = (&app, can_edit);
        panel.into_any_element()
    }

    /// The 24 blend modes, grouped with separators like the original menu.
    fn blend_menu_entries(&self) -> Vec<MenuEntry> {
        let current = self
            .session
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
                        state.session.set_layer_blend_mode(mode);
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
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let id = layer.id;
        let visible = layer.is_visible;
        let rename_input = self.rename == Some(id);

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
                this.session.select_layer(Some(id));
                this.sync_opacity_slider(window, cx);
                if double {
                    this.begin_rename(id, window, cx);
                }
                this.touch(cx);
            }))
            .when(is_active, |row| {
                row.bg(gpui_kit::rgba(0x4da6ff33))
            });

        // Eye.
        row = row.child(
            gpui_kit::div()
                .id(SharedString::from(format!("layer-eye-{}", index)))
                .w(px(18.0))
                .h(px(18.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .text_size(px(10.0))
                .text_color(if visible {
                    gpui_kit::rgb(crate::ui::FG_TEXT)
                } else {
                    gpui_kit::rgb(0x555555)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.session.toggle_layer_visibility(id);
                    this.touch(cx);
                }))
                .child(SharedString::from(if visible { "\u{25CF}" } else { "\u{25CB}" })),
        );

        // Thumbnail.
        let thumbnail = self.thumbnail_image(id, layer);
        row = row.child(
            gpui_kit::div()
                .w(px(THUMB))
                .h(px(THUMB))
                .flex_shrink_0()
                .rounded_sm()
                .overflow_hidden()
                .border_1()
                .border_color(gpui_kit::rgb(0x444444))
                .bg(rgb(0x555555))
                .when_some(thumbnail, |cell, image| {
                    cell.child(
                        gpui_kit::img(image).size_full(),
                    )
                }),
        );

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
                        }
                    }))
                    .child(SharedString::from(name)),
            );
        }
        row = row.child(name_cell);

        // Mask indicator.
        if layer.mask.is_some() {
            row = row.child(
                gpui_kit::div()
                    .px_1()
                    .rounded_xs()
                    .text_size(px(9.0))
                    .text_color(gpui_kit::rgb(crate::ui::ACCENT))
                    .border_1()
                    .border_color(gpui_kit::rgb(crate::ui::ACCENT))
                    .child("M"),
            );
        }
        if layer.is_group {
            row = row.child(
                gpui_kit::div()
                    .px_1()
                    .rounded_xs()
                    .text_size(px(9.0))
                    .text_color(gpui_kit::rgb(crate::ui::FG_DIM))
                    .child("G"),
            );
        }

        row.into_any_element()
    }

    /// New Group: a folder inserted above the active layer, which becomes
    /// its first child.
    pub fn add_group(&mut self, cx: &mut Context<Self>) {
        self.session.add_group();
        self.touch(cx);
    }
}

fn footer_button(
    id: &'static str,
    label: &'static str,
    tooltip: &'static str,
    listener: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static,
) -> Button {
    Button::new(id)
        .label(label)
        .ghost()
        .small()
        .tooltip(tooltip)
        .on_click(listener)
}
