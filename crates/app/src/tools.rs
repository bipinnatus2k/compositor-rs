//! The left tool rail and the per-tool options header.

use std::rc::Rc;

use gpui_kit::{
    px, Context, Entity, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::{Disableable, Selectable, Sizable};

use compositor_core::document::NavigationTool;
use compositor_core::selection::{LassoKind, WandMode};
use compositor_core::session::{BlurToolMode, BrushToolMode, SpotHealingMode};
use compositor_core::tools::brush::GradientShape;
use compositor_core::transform::LayerSampling;

use crate::state::AppState;
use crate::ui::{div_row, dropdown, labeled_slider, text_label, MenuEntry};

/// The rail entries in display order: (tool, letter, name).
const TOOLS: [(NavigationTool, &str, &str); 16] = [
    (NavigationTool::Move, "V", "Move / Transform"),
    (NavigationTool::Marquee, "M", "Marquee"),
    (NavigationTool::Lasso, "L", "Lasso"),
    (NavigationTool::Wand, "W", "Magic Wand"),
    (NavigationTool::Crop, "C", "Crop"),
    (NavigationTool::Brush, "B", "Brush"),
    (NavigationTool::Brush, "E", "Eraser"),
    (NavigationTool::SpotHealing, "J", "Spot Healing Brush"),
    (NavigationTool::CloneStamp, "S", "Clone Stamp"),
    (NavigationTool::Blur, "R", "Smear"),
    (NavigationTool::Gradient, "G", "Gradient"),
    (NavigationTool::Shape, "U", "Shape"),
    (NavigationTool::Type, "T", "Type"),
    (NavigationTool::Eyedropper, "I", "Eyedropper"),
    (NavigationTool::Hand, "H", "Hand"),
    (NavigationTool::Zoom, "Z", "Zoom"),
];

fn tool_title(tool: NavigationTool, erasing: bool) -> &'static str {
    match tool {
        NavigationTool::Move => "Move",
        NavigationTool::Marquee => "Marquee",
        NavigationTool::Lasso => "Lasso",
        NavigationTool::Wand => "Magic",
        NavigationTool::Crop => "Crop",
        NavigationTool::Brush => {
            if erasing {
                "Eraser"
            } else {
                "Brush"
            }
        }
        NavigationTool::SpotHealing => "Spot Healing",
        NavigationTool::CloneStamp => "Clone Stamp",
        NavigationTool::Blur => "Smear",
        NavigationTool::Gradient => "Gradient",
        NavigationTool::Shape => "Shape",
        NavigationTool::Type => "Type",
        NavigationTool::Eyedropper => "Eyedropper",
        NavigationTool::Hand => "Hand",
        NavigationTool::Zoom => "Zoom",
        NavigationTool::Idle => "No tool",
    }
}

#[cfg(test)]
mod tests {
    use super::{font_menu_names, tool_title};
    use compositor_core::document::NavigationTool;

    #[test]
    fn tool_title_names_tools_and_eraser_variant() {
        assert_eq!(tool_title(NavigationTool::Move, false), "Move");
        assert_eq!(tool_title(NavigationTool::Idle, false), "No tool");
        // The brush has two modes depending on the eraser toggle.
        assert_eq!(tool_title(NavigationTool::Brush, false), "Brush");
        assert_eq!(tool_title(NavigationTool::Brush, true), "Eraser");
    }

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

impl AppState {
    pub fn set_tool(&mut self, tool: NavigationTool, cx: &mut Context<Self>) {
        if tool == NavigationTool::Brush {
            self.tab().session.brush_mode = BrushToolMode::Paint;
        }
        self.tab().session.tool = tool;
        self.tab().session.cancel_lasso();
        self.touch(cx);
    }

    pub fn set_tool_eraser(&mut self, cx: &mut Context<Self>) {
        self.tab().session.tool = NavigationTool::Brush;
        self.tab().session.brush_mode = BrushToolMode::Erase;
        self.tab().session.cancel_lasso();
        self.touch(cx);
    }

    // MARK: Tool rail

    pub fn render_tool_rail(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let active = self.tab().session.tool;
        let eraser_active = self.tab().session.tool == NavigationTool::Brush
            && self.tab().session.brush_mode == BrushToolMode::Erase;

        let mut rail = gpui_kit::div()
            .flex()
            .flex_col()
            .items_center()
            .w(px(44.0))
            .flex_shrink_0()
            .py_1()
            .bg(crate::ui::panel_bg());

        for (tool, letter, name) in TOOLS {
            let selected = match letter {
                "E" => eraser_active,
                "B" => active == NavigationTool::Brush && !eraser_active,
                _ => active == tool,
            };
            rail = rail.child(tool_button(tool, letter, name, selected, cx));
        }

        // Foreground / background swatches: clicking either opens the color
        // picker on it (ColorPalette.swift:60; the X key still swaps).
        let foreground = self.tab().session.palette_color(false);
        let background = self.tab().session.palette_color(true);
        rail = rail.child(
            gpui_kit::div()
                .mt_2()
                .id("palette")
                .w(px(28.0))
                .h(px(28.0))
                .relative()
                .child(
                    swatch(6.0, 6.0, background)
                        .id("swatch-background")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_color_picker_ui(true, window, cx);
                        })),
                )
                .child(
                    swatch(0.0, 0.0, foreground)
                        .id("swatch-foreground")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_color_picker_ui(false, window, cx);
                        })),
                ),
        );

        rail.into_any_element()
    }

    // MARK: Options header

    pub fn render_options_header(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let app = cx.entity().downgrade();
        let brush = self.tab().session.brush_settings;
        let tool = self.tab().session.tool;
        let erasing = self.tab().session.brush_mode == BrushToolMode::Erase;

        let mut header = gpui_kit::div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(px(36.0))
            .px_2()
            .gap_3()
            .flex_shrink_0()
            .overflow_hidden()
            .bg(crate::ui::panel_bg());

        header = header.child(
            text_label(tool_title(tool, erasing), false)
                .w(px(82.0))
                .text_size(px(12.0))
                .flex_shrink_0(),
        );

        match tool {
            NavigationTool::Brush
            | NavigationTool::SpotHealing
            | NavigationTool::Blur
            | NavigationTool::CloneStamp => {
                header = header
                    .child(labeled_slider(
                        "Size",
                        &self.brush_sliders[0],
                        format!("{:.0}", brush.diameter),
                        px(90.0),
                    ))
                    .child(labeled_slider(
                        "Hardness",
                        &self.brush_sliders[1],
                        format!("{:.0}%", brush.hardness * 100.0),
                        px(70.0),
                    ))
                    .child(labeled_slider(
                        "Opacity",
                        &self.brush_sliders[2],
                        format!("{:.0}%", brush.opacity * 100.0),
                        px(70.0),
                    ))
                    .child(labeled_slider(
                        "Smoothing",
                        &self.brush_sliders[3],
                        format!("{:.0}", brush.smoothing),
                        px(60.0),
                    ));
                if tool == NavigationTool::Brush {
                    header = header.child(dropdown(
                        &app,
                        "brush-mode",
                        if erasing { "Erase" } else { "Paint" },
                        px(80.0),
                        vec![
                            crate::ui::MenuEntry::new("Paint", |state, _, cx| {
                                state.tab().session.brush_mode = BrushToolMode::Paint;
                                state.touch(cx);
                            })
                            .checked(!erasing),
                            crate::ui::MenuEntry::new("Erase", |state, _, cx| {
                                state.tab().session.brush_mode = BrushToolMode::Erase;
                                state.touch(cx);
                            })
                            .checked(erasing),
                        ],
                    ));
                }
                if tool == NavigationTool::SpotHealing {
                    let mode = self.tab().session.spot_healing_mode;
                    let label = |mode: SpotHealingMode| match mode {
                        SpotHealingMode::ContentAware => "Content Aware",
                        SpotHealingMode::CreateTexture => "Create Texture",
                        SpotHealingMode::ProximityMatch => "Proximity Match",
                    };
                    header = header.child(dropdown(
                        &app,
                        "heal-mode",
                        label(mode),
                        px(150.0),
                        [
                            (SpotHealingMode::ContentAware, "Content Aware"),
                            (SpotHealingMode::CreateTexture, "Create Texture"),
                            (SpotHealingMode::ProximityMatch, "Proximity Match"),
                        ]
                        .iter()
                        .map(|(value, name)| {
                            let value = *value;
                            crate::ui::MenuEntry::new(*name, move |state, _, cx| {
                                state.tab().session.spot_healing_mode = value;
                                state.touch(cx);
                            })
                            .checked(mode == value)
                        })
                        .collect(),
                    ));
                }
                if tool == NavigationTool::Blur {
                    let mode = self.tab().session.blur_mode;
                    let label = |mode: BlurToolMode| match mode {
                        BlurToolMode::Liquify => "Liquify",
                        BlurToolMode::Blur => "Blur",
                        BlurToolMode::Smudge => "Smudge",
                    };
                    header = header.child(dropdown(
                        &app,
                        "blur-mode",
                        label(mode),
                        px(96.0),
                        [
                            (BlurToolMode::Liquify, "Liquify"),
                            (BlurToolMode::Blur, "Blur"),
                            (BlurToolMode::Smudge, "Smudge"),
                        ]
                        .iter()
                        .map(|(value, name)| {
                            let value = *value;
                            crate::ui::MenuEntry::new(*name, move |state, _, cx| {
                                state.tab().session.blur_mode = value;
                                state.touch(cx);
                            })
                            .checked(mode == value)
                        })
                        .collect(),
                    ));
                }
                if tool == NavigationTool::CloneStamp {
                    let aligned = self.tab().session.clone_settings.aligned;
                    let all_layers = self.tab().session.clone_settings.sample_all_layers;
                    header = header
                        .child(checkbox(
                            &app,
                            "clone-aligned",
                            "Aligned",
                            aligned,
                            |state, checked, _, cx| {
                                state.tab().session.clone_settings.aligned = checked;
                                state.touch(cx);
                            },
                        ))
                        .child(checkbox(
                            &app,
                            "clone-all-layers",
                            "Sample All Layers",
                            all_layers,
                            |state, checked, _, cx| {
                                state.tab().session.clone_settings.sample_all_layers = checked;
                                state.touch(cx);
                            },
                        ))
                        .child(text_label("Option-click sets the source", true));
                }
            }
            NavigationTool::Marquee => {
                let kind = self.tab().session.marquee_kind;
                header = header.child(dropdown(
                    &app,
                    "marquee-kind",
                    if kind == LassoKind::Ellipse {
                        "Ellipse"
                    } else {
                        "Rectangle"
                    },
                    px(100.0),
                    vec![
                        crate::ui::MenuEntry::new("Rectangle", |state, _, cx| {
                            state.tab().session.marquee_kind = LassoKind::Rectangle;
                            state.touch(cx);
                        })
                        .checked(kind == LassoKind::Rectangle),
                        crate::ui::MenuEntry::new("Ellipse", |state, _, cx| {
                            state.tab().session.marquee_kind = LassoKind::Ellipse;
                            state.touch(cx);
                        })
                        .checked(kind == LassoKind::Ellipse),
                    ],
                ));
            }
            NavigationTool::Lasso => {
                let kind = self.tab().session.lasso_kind;
                header = header.child(dropdown(
                    &app,
                    "lasso-kind",
                    if kind == LassoKind::Polygonal {
                        "Polygonal"
                    } else {
                        "Freehand"
                    },
                    px(100.0),
                    vec![
                        crate::ui::MenuEntry::new("Freehand", |state, _, cx| {
                            state.tab().session.lasso_kind = LassoKind::Freehand;
                            state.touch(cx);
                        })
                        .checked(kind == LassoKind::Freehand),
                        crate::ui::MenuEntry::new("Polygonal", |state, _, cx| {
                            state.tab().session.lasso_kind = LassoKind::Polygonal;
                            state.touch(cx);
                        })
                        .checked(kind == LassoKind::Polygonal),
                    ],
                ));
            }
            NavigationTool::Wand => {
                let mode = self.tab().session.wand_mode;
                let settings = self.tab().session.wand_settings;
                header = header
                    .child(dropdown(
                        &app,
                        "wand-mode",
                        match mode {
                            WandMode::Wand => "Wand",
                            WandMode::Object => "Object",
                        },
                        px(84.0),
                        vec![
                            crate::ui::MenuEntry::new("Wand", |state, _, cx| {
                                state.tab().session.wand_mode = WandMode::Wand;
                                state.touch(cx);
                            })
                            .checked(mode == WandMode::Wand),
                            crate::ui::MenuEntry::new("Object", |state, _, cx| {
                                state.tab().session.wand_mode = WandMode::Object;
                                state.touch(cx);
                            })
                            .checked(mode == WandMode::Object),
                        ],
                    ))
                    .child(labeled_slider(
                        "Tolerance",
                        &self.wand_slider,
                        format!("{}", settings.tolerance),
                        px(90.0),
                    ))
                    .child(checkbox(
                        &app,
                        "wand-contiguous",
                        "Contiguous",
                        settings.contiguous,
                        |state, checked, _, cx| {
                            state.tab().session.wand_settings.contiguous = checked;
                            state.touch(cx);
                        },
                    ))
                    .child(checkbox(
                        &app,
                        "wand-all-layers",
                        "Sample All Layers",
                        settings.sample_all_layers,
                        |state, checked, _, cx| {
                            state.tab().session.wand_settings.sample_all_layers = checked;
                            state.touch(cx);
                        },
                    ));
            }
            NavigationTool::Gradient => {
                let settings = self.tab().session.gradient_settings;
                header = header
                    .child(dropdown(
                        &app,
                        "gradient-shape",
                        match settings.shape {
                            GradientShape::Linear => "Linear",
                            GradientShape::Radial => "Radial",
                        },
                        px(84.0),
                        vec![
                            crate::ui::MenuEntry::new("Linear", |state, _, cx| {
                                state.tab().session.gradient_settings.shape = GradientShape::Linear;
                                state.touch(cx);
                            })
                            .checked(settings.shape == GradientShape::Linear),
                            crate::ui::MenuEntry::new("Radial", |state, _, cx| {
                                state.tab().session.gradient_settings.shape = GradientShape::Radial;
                                state.touch(cx);
                            })
                            .checked(settings.shape == GradientShape::Radial),
                        ],
                    ))
                    .child(checkbox(
                        &app,
                        "gradient-fg-bg",
                        "Foreground to Background",
                        settings.foreground_to_background,
                        |state, checked, _, cx| {
                            state.tab().session.gradient_settings.foreground_to_background = checked;
                            state.touch(cx);
                        },
                    ))
                    .child(checkbox(
                        &app,
                        "gradient-reverse",
                        "Reverse",
                        settings.reversed,
                        |state, checked, _, cx| {
                            state.tab().session.gradient_settings.reversed = checked;
                            state.touch(cx);
                        },
                    ))
                    .child(labeled_slider(
                        "Opacity",
                        &self.gradient_slider,
                        format!("{:.0}%", settings.opacity * 100.0),
                        px(80.0),
                    ));
            }
            NavigationTool::Zoom => {
                header = header
                    .child(zoom_button(&app, "zoom-fit", "Fit"))
                    .child(zoom_button(&app, "zoom-100", "100%"))
                    .child(zoom_button(&app, "zoom-in", "+"))
                    .child(zoom_button(&app, "zoom-out", "-"));
            }
            NavigationTool::Crop => {
                let choice = self.tab().session.crop_ratio_choice.clone();
                let has_crop = self.tab().session.crop_rect.is_some();
                let entries = [
                    ("Free", "Free"),
                    ("Original", "Original"),
                    ("1:1", "1:1"),
                    ("4:3", "4:3"),
                    ("16:9", "16:9"),
                ]
                .iter()
                .map(|(value, label)| {
                    let checked = choice == *value;
                    let value = value.to_string();
                    crate::ui::MenuEntry::new(*label, move |state, _, cx| {
                        state.tab().session.crop_ratio_choice = value.clone();
                        state.touch(cx);
                    })
                    .checked(checked)
                })
                .collect();
                header = header
                    .child(dropdown(&app, "crop-ratio", choice.clone(), px(92.0), entries))
                    .child(
                        Button::new("crop-apply")
                            .label("Apply")
                            .small()
                            .primary()
                            .disabled(!has_crop)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tab().session.commit_crop();
                                this.touch(cx);
                            })),
                    )
                    .child(
                        Button::new("crop-cancel")
                            .label("Cancel")
                            .small()
                            .disabled(!has_crop)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tab().session.cancel_crop();
                                this.touch(cx);
                            })),
                    );
            }
            _ => {}
        }

        if tool == NavigationTool::Move {
            // TransformInspector.swift: toggles, the numeric fields, flip,
            // sampling, then Cancel/Apply.
            let edit = self.tab().session.transform_edit.as_ref();
            let has_edit = edit.is_some();
            let corners = edit.map(|edit| edit.corners.is_some()).unwrap_or(false);
            let mask = edit.map(|edit| edit.mask).unwrap_or_else(|| {
                self.tab().session.is_mask_selected
                    && self
                        .tab().session
                        .active_layer()
                        .and_then(|layer| {
                            layer.mask.as_ref().map(|mask| !mask.is_linked)
                        })
                        .unwrap_or(false)
            });
            // The fields gray out while nothing can transform, or while a
            // corner distort owns the edit (the handles are the controls).
            let fields_enabled = (self.tab().session.can_transform() || has_edit) && !corners;
            header = header
                .child(
                    text_label(if mask { "Transform Mask" } else { "Transform" }, false)
                        .text_size(px(12.0))
                        .flex_shrink_0(),
                )
                .child(checkbox(
                    &app,
                    "transform-auto-select",
                    "Auto-select",
                    self.transform_auto_select,
                    |state, checked, _, cx| {
                        // transformAutoSelect (EditorSession.swift:198). The
                        // canvas click-through itself is canvas-side wiring.
                        state.transform_auto_select = checked;
                        state.persist_tool_defaults();
                        state.touch(cx);
                    },
                ))
                .child(checkbox(
                    &app,
                    "transform-show-controls",
                    "Show Controls",
                    self.tab().session.shows_transform_controls,
                    |state, checked, _, cx| {
                        state.tab().session.shows_transform_controls = checked;
                        state.touch(cx);
                    },
                ))
                .child(number_field("transform-x", "X", &self.transform_x_input, px(52.0)))
                .child(number_field("transform-y", "Y", &self.transform_y_input, px(52.0)))
                .child(number_field("transform-w", "W", &self.transform_w_input, px(52.0)))
                .child(number_field("transform-h", "H", &self.transform_h_input, px(52.0)))
                .child(checkbox(
                    &app,
                    "transform-lock-ratio",
                    "Lock",
                    self.tab().session.locks_transform_ratio,
                    |state, checked, _, cx| {
                        state.tab().session.locks_transform_ratio = checked;
                        state.touch(cx);
                    },
                ))
                .child(number_field(
                    "transform-scale",
                    "Scale %",
                    &self.transform_scale_input,
                    px(56.0),
                ))
                .child(number_field(
                    "transform-rotation",
                    "\u{b0}",
                    &self.transform_rotation_input,
                    px(48.0),
                ))
                .child(sampling_dropdown(&app, self.transform_displayed(), fields_enabled))
                .child(
                    Button::new("transform-flip-h")
                        .label("Flip H")
                        .small()
                        .outline()
                        .disabled(!fields_enabled)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.transform_flip(true, window, cx);
                        })),
                )
                .child(
                    Button::new("transform-flip-v")
                        .label("Flip V")
                        .small()
                        .outline()
                        .disabled(!fields_enabled)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.transform_flip(false, window, cx);
                        })),
                )
                .child(
                    Button::new("transform-cancel")
                        .label("Cancel")
                        .small()
                        .disabled(!has_edit)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.tab().session.cancel_transform();
                            this.sync_transform_inputs(window, cx);
                            this.touch(cx);
                        })),
                )
                .child(
                    Button::new("transform-apply")
                        .label("Apply")
                        .small()
                        .primary()
                        .disabled(!has_edit)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.tab().session.commit_transform();
                            this.sync_transform_inputs(window, cx);
                            this.touch(cx);
                        })),
                );
        }
        if tool == NavigationTool::Hand {
            header = header.child(text_label("Drag to pan · scroll to zoom", true));
        }
        if tool == NavigationTool::Type {
            // TypeControls.swift: font, size, color, alignment, tracking,
            // leading, and the draft buttons.
            let style = self.tab().session.current_text_style();
            let font = style.font_name.clone();
            let names = font_menu_names(&font, available_font_names(), FONT_MENU_CAP);
            let entries = names
                .into_iter()
                .map(|name| {
                    let chosen = name.clone();
                    MenuEntry::new(name.clone(), move |state, _, cx| {
                        let chosen = chosen.clone();
                        state.tab().session.change_text_style(move |style| {
                            style.font_name = chosen;
                        });
                        state.touch(cx);
                    })
                    .checked(style.font_name == name)
                })
                .collect();
            let rgb = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
            let color = self.tab().session.type_color();
            let color_rgb =
                (rgb(color.red) << 16) | (rgb(color.green) << 8) | rgb(color.blue);
            let has_document = self.tab().session.document.is_some();
            let has_draft = self.tab().session.text_draft.is_some();
            let active_is_text = self
                .tab().session
                .active_layer()
                .and_then(|layer| layer.live_text())
                .is_some();
            header = header
                .child(dropdown(&app, "type-font", font, px(150.0), entries))
                .child(
                    div_row()
                        .child(Input::new(&self.type_size_input).id("type-size").small().w(px(52.0)))
                        .child(text_label("px", true)),
                )
                .child(
                    gpui_kit::div()
                        .id("type-color")
                        .w(px(36.0))
                        .h(px(18.0))
                        .rounded_xs()
                        .border_1()
                        .border_color(gpui_kit::rgb(0x111111))
                        .bg(gpui_kit::rgb(color_rgb))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_text_color_picker_ui(window, cx);
                        })),
                )
                .child(alignment_buttons(&app, style.alignment))
                .child(
                    div_row()
                        .child(text_label("Tracking", true))
                        .child(
                            Input::new(&self.type_tracking_input)
                                .id("type-tracking")
                                .small()
                                .w(px(48.0)),
                        ),
                )
                .child(
                    div_row()
                        .child(text_label("Leading", true))
                        .child(
                            Input::new(&self.type_leading_input)
                                .id("type-leading")
                                .small()
                                .w(px(52.0)),
                        ),
                );
            if has_draft {
                header = header
                    .child(
                        Button::new("type-cancel")
                            .label("Cancel")
                            .small()
                            .disabled(!has_document)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.tab().session.cancel_text();
                                this.sync_text_style_inputs(window, cx);
                                this.touch(cx);
                            })),
                    )
                    .child(
                        Button::new("type-done")
                            .label("Done")
                            .small()
                            .primary()
                            .disabled(!has_document)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.tab().session.finish_text();
                                this.sync_text_style_inputs(window, cx);
                                this.touch(cx);
                            })),
                    );
            } else {
                header = header.child(
                    Button::new("type-edit")
                        .label("Edit Text")
                        .small()
                        .disabled(!has_document || !active_is_text)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.tab().session.edit_active_text();
                            this.sync_text_style_inputs(window, cx);
                            this.touch(cx);
                        })),
                );
            }
        }
        if tool == NavigationTool::Shape {
            let kind = self.tab().session.shape_kind;
            let label = |kind: compositor_core::document::ShapeKind| match kind {
                compositor_core::document::ShapeKind::Rectangle => "Rectangle",
                compositor_core::document::ShapeKind::RoundedRectangle => "Rounded Rectangle",
                compositor_core::document::ShapeKind::Ellipse => "Ellipse",
                compositor_core::document::ShapeKind::Line => "Line",
            };
            header = header
                .child(dropdown(
                    &app,
                    "shape-kind",
                    label(kind),
                    px(150.0),
                    vec![
                        crate::ui::MenuEntry::new("Rectangle", |state, _, cx| {
                            state.tab().session.shape_kind =
                                compositor_core::document::ShapeKind::Rectangle;
                            state.touch(cx);
                        })
                        .checked(kind == compositor_core::document::ShapeKind::Rectangle),
                        crate::ui::MenuEntry::new("Rounded Rectangle", |state, _, cx| {
                            state.tab().session.shape_kind =
                                compositor_core::document::ShapeKind::RoundedRectangle;
                            state.touch(cx);
                        })
                        .checked(kind == compositor_core::document::ShapeKind::RoundedRectangle),
                        crate::ui::MenuEntry::new("Ellipse", |state, _, cx| {
                            state.tab().session.shape_kind =
                                compositor_core::document::ShapeKind::Ellipse;
                            state.touch(cx);
                        })
                        .checked(kind == compositor_core::document::ShapeKind::Ellipse),
                        crate::ui::MenuEntry::new("Line", |state, _, cx| {
                            state.tab().session.shape_kind = compositor_core::document::ShapeKind::Line;
                            state.touch(cx);
                        })
                        .checked(kind == compositor_core::document::ShapeKind::Line),
                    ],
                ))
                .child(text_label("Drag on the canvas · Shift-U switches shape", true));
        }

        header.into_any_element()
    }

    /// Flip H / Flip V (`TransformInspector`'s flip buttons): one transform
    /// change each, then the fields follow.
    pub fn transform_flip(
        &mut self,
        horizontal: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.transform_change(
            move |value| {
                if horizontal {
                    value.flip_x = !value.flip_x;
                } else {
                    value.flip_y = !value.flip_y;
                }
            },
            cx,
        );
        self.sync_transform_inputs(window, cx);
    }
}

/// Cap on the Type header's font menu: the catalog holds every installed
/// face, and the popup only takes so many.
const FONT_MENU_CAP: usize = 250;

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
fn available_font_names() -> &'static Vec<String> {
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

/// One of the inspector's labeled numeric fields (TransformValueField's
/// `label + rounded field`). Typing writes through the field's Change
/// subscription; nothing here touches the session.
fn number_field(
    id: &'static str,
    label: &str,
    state: &Entity<InputState>,
    width: gpui_kit::Pixels,
) -> gpui_kit::Div {
    div_row()
        .child(text_label(label.to_string(), true))
        .child(Input::new(state).id(id).small().w(width))
}

/// The Type bar's alignment segment (`TextAlignment.allCases`): ghost
/// buttons with the selected one lit. The widget set here has no SF Symbol
/// glyphs, so the labels carry the meaning.
fn alignment_buttons(
    app: &gpui_kit::WeakEntity<AppState>,
    selected: compositor_core::document::TextAlignment,
) -> gpui_kit::Div {
    let mut row = gpui_kit::div().flex().flex_row().gap_0p5().flex_shrink_0();
    for (alignment, label) in [
        (
            compositor_core::document::TextAlignment::Left,
            "Left",
        ),
        (
            compositor_core::document::TextAlignment::Center,
            "Center",
        ),
        (
            compositor_core::document::TextAlignment::Right,
            "Right",
        ),
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

/// The Sampling picker (`LayerSampling.allCases`); disabled while a corner
/// distort owns the edit.
fn sampling_dropdown(
    app: &gpui_kit::WeakEntity<AppState>,
    value: Option<compositor_core::transform::LayerTransform>,
    enabled: bool,
) -> gpui_kit::AnyElement {
    let current = value
        .map(|value| value.sampling)
        .unwrap_or(LayerSampling::High);
    let entries: Vec<MenuEntry> = LayerSampling::ALL
        .iter()
        .map(|sampling| {
            let sampling = *sampling;
            MenuEntry::new(sampling.label(), move |state, _, cx| {
                state.transform_change(move |value| value.sampling = sampling, cx);
            })
            .checked(current == sampling)
        })
        .collect();
    let app = app.clone();
    let button = Button::new("transform-sampling")
        .label(current.label())
        .outline()
        .small()
        .w(px(120.0))
        .disabled(!enabled);
    button
        .dropdown_menu(move |menu, _, _| crate::ui::build_menu(&app, menu, &entries))
        .into_any_element()
}

/// A checkbox whose handler receives the app, the new value, window and ctx.
pub fn checkbox(
    app: &gpui_kit::WeakEntity<AppState>,
    id: &'static str,
    label: &str,
    checked: bool,
    handler: impl Fn(&mut AppState, bool, &mut gpui_kit::Window, &mut Context<AppState>) + 'static,
) -> gpui_kit::AnyElement {
    let app = app.clone();
    let handler = Rc::new(handler);
    gpui_kit::component::checkbox::Checkbox::new(id)
        .label(label)
        .checked(checked)
        .on_change(move |checked, window, cx| {
            let handler = handler.clone();
            let checked = *checked;
            app.update(cx, move |state, cx| handler(state, checked, window, cx))
                .ok();
        })
        .text_size(px(11.0))
        .into_any_element()
}

fn tool_button(
    tool: NavigationTool,
    letter: &'static str,
    name: &'static str,
    selected: bool,
    cx: &mut Context<AppState>,
) -> Button {
    let eraser = letter == "E";
    Button::new(SharedString::from(format!("tool-{}", letter)))
        .label(letter)
        .ghost()
        .compact()
        .selected(selected)
        .tooltip(name)
        .on_click(cx.listener(move |state, _, _, cx| {
            if eraser {
                state.set_tool_eraser(cx);
            } else {
                state.set_tool(tool, cx);
            }
        }))
}

fn swatch(offset_x: f32, offset_y: f32, color: compositor_core::document::PaletteColor) -> gpui_kit::Div {
    let value = ((color.red * 255.0).round() as u32) << 16
        | ((color.green * 255.0).round() as u32) << 8
        | (color.blue * 255.0).round() as u32;
    gpui_kit::div()
        .absolute()
        .top(px(offset_y))
        .left(px(offset_x))
        .w(px(18.0))
        .h(px(18.0))
        .rounded_sm()
        .border_1()
        .border_color(gpui_kit::rgb(0x606060))
        .bg(gpui_kit::rgb(value))
}

fn zoom_button(app: &gpui_kit::WeakEntity<AppState>, id: &'static str, label: &'static str) -> Button {
    let app = app.clone();
    let factor: f64 = if label == "+" {
        2.0
    } else if label == "-" {
        0.5
    } else {
        1.0
    };
    Button::new(id)
        .label(label)
        .outline()
        .small()
        .on_click(move |_, _, cx| {
            app.update(cx, |state, cx| match label {
                "Fit" => {
                    let viewport = state.viewport_size();
                    state.zoom_fit(viewport);
                    state.touch(cx);
                }
                "100%" => {
                    state.zoom = 1.0;
                    state.revision += 1;
                    state.touch(cx);
                }
                _ => {
                    state.zoom_at_viewport(factor);
                    state.touch(cx);
                }
            })
            .ok();
        })
}
