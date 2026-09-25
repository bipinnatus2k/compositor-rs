//! The left tool rail and the per-tool options header.

use std::rc::Rc;

use gpui_kit::{
    px, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Selectable, Sizable};

use compositor_core::document::NavigationTool;
use compositor_core::selection::{LassoKind, WandMode};
use compositor_core::session::{BlurToolMode, BrushToolMode, SpotHealingMode};
use compositor_core::tools::brush::GradientShape;

use crate::state::AppState;
use crate::ui::{MenuEntry, dropdown, labeled_slider, text_label};

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
    use super::tool_title;
    use compositor_core::document::NavigationTool;

    #[test]
    fn tool_title_names_tools_and_eraser_variant() {
        assert_eq!(tool_title(NavigationTool::Move, false), "Move");
        assert_eq!(tool_title(NavigationTool::Idle, false), "No tool");
        // The brush has two modes depending on the eraser toggle.
        assert_eq!(tool_title(NavigationTool::Brush, false), "Brush");
        assert_eq!(tool_title(NavigationTool::Brush, true), "Eraser");
    }
}

impl AppState {
    pub fn set_tool(&mut self, tool: NavigationTool, cx: &mut Context<Self>) {
        if tool == NavigationTool::Brush {
            self.session.brush_mode = BrushToolMode::Paint;
        }
        self.session.tool = tool;
        self.session.cancel_lasso();
        self.touch(cx);
    }

    pub fn set_tool_eraser(&mut self, cx: &mut Context<Self>) {
        self.session.tool = NavigationTool::Brush;
        self.session.brush_mode = BrushToolMode::Erase;
        self.session.cancel_lasso();
        self.touch(cx);
    }

    // MARK: Tool rail

    pub fn render_tool_rail(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let active = self.session.tool;
        let eraser_active = self.session.tool == NavigationTool::Brush
            && self.session.brush_mode == BrushToolMode::Erase;

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
        let foreground = self.session.palette_color(false);
        let background = self.session.palette_color(true);
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
        let brush = self.session.brush_settings;
        let tool = self.session.tool;
        let erasing = self.session.brush_mode == BrushToolMode::Erase;

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
                                state.session.brush_mode = BrushToolMode::Paint;
                                state.touch(cx);
                            })
                            .checked(!erasing),
                            crate::ui::MenuEntry::new("Erase", |state, _, cx| {
                                state.session.brush_mode = BrushToolMode::Erase;
                                state.touch(cx);
                            })
                            .checked(erasing),
                        ],
                    ));
                }
                if tool == NavigationTool::SpotHealing {
                    let mode = self.session.spot_healing_mode;
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
                                state.session.spot_healing_mode = value;
                                state.touch(cx);
                            })
                            .checked(mode == value)
                        })
                        .collect(),
                    ));
                }
                if tool == NavigationTool::Blur {
                    let mode = self.session.blur_mode;
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
                                state.session.blur_mode = value;
                                state.touch(cx);
                            })
                            .checked(mode == value)
                        })
                        .collect(),
                    ));
                }
                if tool == NavigationTool::CloneStamp {
                    let aligned = self.session.clone_settings.aligned;
                    let all_layers = self.session.clone_settings.sample_all_layers;
                    header = header
                        .child(checkbox(
                            &app,
                            "clone-aligned",
                            "Aligned",
                            aligned,
                            |state, checked, _, cx| {
                                state.session.clone_settings.aligned = checked;
                                state.touch(cx);
                            },
                        ))
                        .child(checkbox(
                            &app,
                            "clone-all-layers",
                            "Sample All Layers",
                            all_layers,
                            |state, checked, _, cx| {
                                state.session.clone_settings.sample_all_layers = checked;
                                state.touch(cx);
                            },
                        ))
                        .child(text_label("Option-click sets the source", true));
                }
            }
            NavigationTool::Marquee => {
                let kind = self.session.marquee_kind;
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
                            state.session.marquee_kind = LassoKind::Rectangle;
                            state.touch(cx);
                        })
                        .checked(kind == LassoKind::Rectangle),
                        crate::ui::MenuEntry::new("Ellipse", |state, _, cx| {
                            state.session.marquee_kind = LassoKind::Ellipse;
                            state.touch(cx);
                        })
                        .checked(kind == LassoKind::Ellipse),
                    ],
                ));
            }
            NavigationTool::Lasso => {
                let kind = self.session.lasso_kind;
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
                            state.session.lasso_kind = LassoKind::Freehand;
                            state.touch(cx);
                        })
                        .checked(kind == LassoKind::Freehand),
                        crate::ui::MenuEntry::new("Polygonal", |state, _, cx| {
                            state.session.lasso_kind = LassoKind::Polygonal;
                            state.touch(cx);
                        })
                        .checked(kind == LassoKind::Polygonal),
                    ],
                ));
            }
            NavigationTool::Wand => {
                let mode = self.session.wand_mode;
                let settings = self.session.wand_settings;
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
                                state.session.wand_mode = WandMode::Wand;
                                state.touch(cx);
                            })
                            .checked(mode == WandMode::Wand),
                            crate::ui::MenuEntry::new("Object", |state, _, cx| {
                                state.session.wand_mode = WandMode::Object;
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
                            state.session.wand_settings.contiguous = checked;
                            state.touch(cx);
                        },
                    ))
                    .child(checkbox(
                        &app,
                        "wand-all-layers",
                        "Sample All Layers",
                        settings.sample_all_layers,
                        |state, checked, _, cx| {
                            state.session.wand_settings.sample_all_layers = checked;
                            state.touch(cx);
                        },
                    ));
            }
            NavigationTool::Gradient => {
                let settings = self.session.gradient_settings;
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
                                state.session.gradient_settings.shape = GradientShape::Linear;
                                state.touch(cx);
                            })
                            .checked(settings.shape == GradientShape::Linear),
                            crate::ui::MenuEntry::new("Radial", |state, _, cx| {
                                state.session.gradient_settings.shape = GradientShape::Radial;
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
                            state.session.gradient_settings.foreground_to_background = checked;
                            state.touch(cx);
                        },
                    ))
                    .child(checkbox(
                        &app,
                        "gradient-reverse",
                        "Reverse",
                        settings.reversed,
                        |state, checked, _, cx| {
                            state.session.gradient_settings.reversed = checked;
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
                let choice = self.session.crop_ratio_choice.clone();
                let has_crop = self.session.crop_rect.is_some();
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
                        state.session.crop_ratio_choice = value.clone();
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
                                this.session.commit_crop();
                                this.touch(cx);
                            })),
                    )
                    .child(
                        Button::new("crop-cancel")
                            .label("Cancel")
                            .small()
                            .disabled(!has_crop)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.session.cancel_crop();
                                this.touch(cx);
                            })),
                    );
            }
            _ => {}
        }

        if tool == NavigationTool::Move {
            header = header.child(checkbox(
                &app,
                "move-auto-select",
                "Auto-select",
                true,
                |_, _, _, _| {},
            ));
        }
        if tool == NavigationTool::Hand {
            header = header.child(text_label("Drag to pan · scroll to zoom", true));
        }
        if tool == NavigationTool::Type {
            header = header.child(text_label("Click on the canvas to place a text layer", true));
        }
        if tool == NavigationTool::Shape {
            let kind = self.session.shape_kind;
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
                            state.session.shape_kind =
                                compositor_core::document::ShapeKind::Rectangle;
                            state.touch(cx);
                        })
                        .checked(kind == compositor_core::document::ShapeKind::Rectangle),
                        crate::ui::MenuEntry::new("Rounded Rectangle", |state, _, cx| {
                            state.session.shape_kind =
                                compositor_core::document::ShapeKind::RoundedRectangle;
                            state.touch(cx);
                        })
                        .checked(kind == compositor_core::document::ShapeKind::RoundedRectangle),
                        crate::ui::MenuEntry::new("Ellipse", |state, _, cx| {
                            state.session.shape_kind =
                                compositor_core::document::ShapeKind::Ellipse;
                            state.touch(cx);
                        })
                        .checked(kind == compositor_core::document::ShapeKind::Ellipse),
                        crate::ui::MenuEntry::new("Line", |state, _, cx| {
                            state.session.shape_kind = compositor_core::document::ShapeKind::Line;
                            state.touch(cx);
                        })
                        .checked(kind == compositor_core::document::ShapeKind::Line),
                    ],
                ))
                .child(text_label("Drag on the canvas · Shift-U switches shape", true));
        }

        header.into_any_element()
    }
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
