//! The left tool rail and the options header shell (NavigationToolHeader.swift
//! plus ContentView's `toolHeaders` group): the tool title and the dispatch
//! to each tool's controls. The per-tool control sets live beside their
//! Swift sources (`ui/brush_controls.rs`, `ui/lasso_controls.rs`,
//! `ui/crop_controls.rs`, `ui/shape_controls.rs`, `ui/type_controls.rs`,
//! `ui/transform_inspector.rs`). The Zoom/Hand branches are
//! NavigationToolHeader.swift's own subject and stay here, with the
//! Gradient branch beside them pending its own carve.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Selectable, Sizable};
use gpui_kit::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, px,
};

use compositor_core::document::NavigationTool;
use compositor_core::session::{BrushToolMode, EditorSession};
use compositor_core::tools::brush::GradientShape;

use crate::state::{AppState, CanvasDrag};
use crate::ui::{MenuEntry, dropdown, labeled_slider, text_label};

/// The rail entries in display order: (tool, id letter, Lucide icon, name,
/// eraser-variant flag). The letter only names the button element (kept so
/// existing element ids are stable); the glyph comes from the bundled
/// Lucide set (`gpui_kit::assets`). Brush and Eraser share the Brush tool.
const TOOLS: [(NavigationTool, &str, IconName, &str, bool); 16] = [
    (
        NavigationTool::Move,
        "V",
        IconName::Move,
        "Move / Transform",
        false,
    ),
    (
        NavigationTool::Marquee,
        "M",
        IconName::SquareDashed,
        "Marquee",
        false,
    ),
    (NavigationTool::Lasso, "L", IconName::Lasso, "Lasso", false),
    (
        NavigationTool::Wand,
        "W",
        IconName::WandSparkles,
        "Magic Wand",
        false,
    ),
    (NavigationTool::Crop, "C", IconName::Crop, "Crop", false),
    (NavigationTool::Brush, "B", IconName::Brush, "Brush", false),
    (NavigationTool::Brush, "E", IconName::Eraser, "Eraser", true),
    (
        NavigationTool::SpotHealing,
        "J",
        IconName::Bandage,
        "Spot Healing Brush",
        false,
    ),
    (
        NavigationTool::CloneStamp,
        "S",
        IconName::Stamp,
        "Clone Stamp",
        false,
    ),
    (NavigationTool::Blur, "R", IconName::Droplet, "Smear", false),
    (
        NavigationTool::Gradient,
        "G",
        IconName::Blend,
        "Gradient",
        false,
    ),
    (NavigationTool::Shape, "U", IconName::Square, "Shape", false),
    (NavigationTool::Type, "T", IconName::Type, "Type", false),
    (
        NavigationTool::Eyedropper,
        "I",
        IconName::Pipette,
        "Eyedropper",
        false,
    ),
    (NavigationTool::Hand, "H", IconName::Hand, "Hand", false),
    (NavigationTool::Zoom, "Z", IconName::ZoomIn, "Zoom", false),
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

/// EditorSession.swift:347-351: entering Crop with no frame presets the
/// full-document frame and resets the ratio choice to Free, so a ratio
/// left over from an earlier crop never reshapes the preset. The preset
/// itself moved into `EditorSession::select_tool` (ticket 16); this shim
/// only keeps the ticket-04 guard test's call shape.
#[cfg(test)]
fn crop_entry_preset(session: &mut EditorSession, tool: NavigationTool) {
    session.select_tool(tool);
}

/// The options header's shared "(value, label) → checked menu entry"
/// dropdown shape: one entry per value in the given order, the checkmark
/// on the entry matching `current` (a `String` field compares against its
/// `&'static str` presets), and a click assigning the value to the session
/// field `set` names. The per-tool control modules share it.
pub(crate) fn menu_entries<V, C, S>(
    values: impl IntoIterator<Item = (V, &'static str)>,
    current: C,
    set: S,
) -> Vec<MenuEntry>
where
    V: Copy + 'static,
    C: PartialEq<V>,
    S: Fn(&mut EditorSession, V) + Clone + 'static,
{
    values
        .into_iter()
        .map(|(value, label)| {
            let assign = set.clone();
            MenuEntry::new(label, move |state, _, cx| {
                assign(&mut state.tab().session, value);
                state.touch(cx);
            })
            .checked(current == value)
        })
        .collect()
}

impl AppState {
    pub fn set_tool(&mut self, tool: NavigationTool, cx: &mut Context<Self>) {
        // `selectTool` (EditorSession.swift:332-352) lives on the session
        // now. Its refusal guards (:334) stay here: the stroke and warp
        // drags and the project-busy flag are app-side state, and the
        // adjustment editor is the port's stand-in for Swift's `levels`
        // sheet. A refused switch (a failing text apply) leaves the shape
        // drag up, like every session-side gate.
        if matches!(
            self.drag,
            Some(CanvasDrag::Stroke(..)) | Some(CanvasDrag::Warp(..))
        ) {
            return;
        }
        if self.current_tab().busy.is_project_busy
            || self.current_tab().session.adjustment_editing_id.is_some()
        {
            return;
        }
        if !self.tab().session.select_tool(tool) {
            return;
        }
        // A shape drag dies with the switch (Swift's cancelShape :335); it
        // lives app-side, so dropping it here is the whole cancel.
        if matches!(self.drag, Some(CanvasDrag::Shape { .. })) {
            self.drag = None;
        }
        self.touch(cx);
    }

    pub fn set_tool_eraser(&mut self, cx: &mut Context<Self>) {
        self.set_tool(NavigationTool::Brush, cx);
        self.tab().session.brush_mode = BrushToolMode::Erase;
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

        for (tool, letter, icon, name, eraser_entry) in TOOLS {
            let selected = if eraser_entry {
                eraser_active
            } else if tool == NavigationTool::Brush {
                active == NavigationTool::Brush && !eraser_active
            } else {
                active == tool
            };
            rail = rail.child(tool_button(
                tool,
                letter,
                icon,
                name,
                eraser_entry,
                selected,
                cx,
            ));
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
        let tool = self.tab().session.tool;
        let erasing = self.tab().session.brush_mode == BrushToolMode::Erase;

        let mut header = gpui_kit::div()
            // Stateful so the bar can scroll sideways (overflow_x_scroll is a
            // StatefulInteractiveElement method, gpui div.rs).
            .id("options-header")
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            // ToolHeaderStyle.swift:5 (height 42) and BrushControls.swift:111
            // (horizontal padding 18).
            .h(px(42.0))
            .px(px(18.0))
            .gap_3()
            .flex_shrink_0()
            // Dense bars (Type, Move) run out of width; scroll sideways
            // instead of clipping the tail controls.
            .overflow_x_scroll()
            .bg(crate::ui::panel_bg());

        header = header.child(
            text_label(tool_title(tool, erasing), false)
                .w(px(82.0))
                // ToolHeaderStyle.swift:6: the title is 13pt semibold.
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .flex_shrink_0(),
        );

        match tool {
            NavigationTool::Brush
            | NavigationTool::SpotHealing
            | NavigationTool::Blur
            | NavigationTool::CloneStamp => {
                header = header.children(self.brush_controls(cx));
            }
            NavigationTool::Marquee => {
                header = header.children(self.marquee_controls(cx));
            }
            NavigationTool::Lasso => {
                header = header.children(self.lasso_controls(cx));
            }
            NavigationTool::Wand => {
                header = header.children(self.wand_controls(cx));
            }
            NavigationTool::Gradient => {
                // Swift home: GradientControls.swift. It is not in ticket
                // 32's file contract, so the controls stay with the shell.
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
                        menu_entries(
                            [
                                (GradientShape::Linear, "Linear"),
                                (GradientShape::Radial, "Radial"),
                            ],
                            settings.shape,
                            |session, value| session.gradient_settings.shape = value,
                        ),
                    ))
                    .child(checkbox(
                        &app,
                        "gradient-fg-bg",
                        "Foreground to Background",
                        settings.foreground_to_background,
                        |state, checked, _, cx| {
                            state
                                .tab()
                                .session
                                .gradient_settings
                                .foreground_to_background = checked;
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
                    .child(zoom_button(&app, "zoom-fit", ZoomCommand::Fit))
                    .child(zoom_button(&app, "zoom-100", ZoomCommand::ActualSize))
                    .child(zoom_button(&app, "zoom-in", ZoomCommand::In))
                    .child(zoom_button(&app, "zoom-out", ZoomCommand::Out));
            }
            NavigationTool::Crop => {
                header = header.children(self.crop_controls(cx));
            }
            _ => {}
        }

        if tool == NavigationTool::Move {
            header = header.children(self.transform_inspector(cx));
        }
        if tool == NavigationTool::Hand {
            header = header.child(text_label("Drag to pan · scroll to zoom", true));
        }
        if tool == NavigationTool::Type {
            header = header.children(self.type_controls(cx));
        }
        if tool == NavigationTool::Shape {
            header = header.children(self.shape_controls(cx));
        }

        header.into_any_element()
    }
}

/// A checkbox whose handler receives the app, the new value, window and ctx.
pub(crate) fn checkbox(
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
        // ToolHeaderStyle.swift:7: the control font is 12.
        .text_size(px(12.0))
        // The checkbox root lays its indicator out items_start
        // (gpui-component checkbox.rs), which top-aligns the box against the
        // text at this small size; centering overrides the refinement.
        .items_center()
        .into_any_element()
}

fn tool_button(
    tool: NavigationTool,
    letter: &'static str,
    icon: IconName,
    name: &'static str,
    eraser: bool,
    selected: bool,
    cx: &mut Context<AppState>,
) -> Button {
    Button::new(SharedString::from(format!("tool-{}", letter)))
        .icon(Icon::new(icon))
        // Icon-only buttons have no label to read aloud; the tooltip name
        // doubles as the announced one.
        .accessibility_label(name)
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

fn swatch(
    offset_x: f32,
    offset_y: f32,
    color: compositor_core::document::PaletteColor,
) -> gpui_kit::Div {
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
        .border_color(gpui_kit::rgb(crate::ui::BORDER_BRIGHT))
        .bg(gpui_kit::rgb(value))
}

/// What one zoom header button does (the zoom toolbar's Fit / 100% / +
/// / -). The command is explicit; the label is only the button's face.
#[derive(Clone, Copy)]
enum ZoomCommand {
    Fit,
    ActualSize,
    In,
    Out,
}

impl ZoomCommand {
    fn label(self) -> &'static str {
        match self {
            ZoomCommand::Fit => "Fit",
            ZoomCommand::ActualSize => "100%",
            ZoomCommand::In => "+",
            ZoomCommand::Out => "-",
        }
    }
}

fn zoom_button(
    app: &gpui_kit::WeakEntity<AppState>,
    id: &'static str,
    command: ZoomCommand,
) -> Button {
    let app = app.clone();
    Button::new(id)
        .label(command.label())
        .outline()
        .small()
        .on_click(move |_, _, cx| {
            app.update(cx, |state, cx| match command {
                ZoomCommand::Fit => {
                    let viewport = state.viewport_size();
                    state.zoom_fit(viewport);
                    state.touch(cx);
                }
                ZoomCommand::ActualSize => {
                    state.zoom = 1.0;
                    state.revision += 1;
                    state.touch(cx);
                }
                ZoomCommand::In => {
                    state.zoom_at_viewport(2.0);
                    state.touch(cx);
                }
                ZoomCommand::Out => {
                    state.zoom_at_viewport(0.5);
                    state.touch(cx);
                }
            })
            .ok();
        })
}

// MARK: Tests

#[cfg(test)]
mod tests {
    use super::{crop_entry_preset, menu_entries, tool_title};
    use compositor_core::document::NavigationTool;
    use compositor_core::geometry::CGRect;
    use compositor_core::session::EditorSession;

    #[test]
    fn tool_title_names_tools_and_eraser_variant() {
        assert_eq!(tool_title(NavigationTool::Move, false), "Move");
        assert_eq!(tool_title(NavigationTool::Idle, false), "No tool");
        // The brush has two modes depending on the eraser toggle.
        assert_eq!(tool_title(NavigationTool::Brush, false), "Brush");
        assert_eq!(tool_title(NavigationTool::Brush, true), "Eraser");
    }

    /// The shared dropdown builder yields the same menu items and checked
    /// state as the per-tool constructions it replaced: one entry per
    /// value in the given order, the checkmark on the current one, every
    /// entry enabled and no separators.
    #[test]
    fn menu_entries_keep_the_item_order_and_checked_state() {
        use compositor_core::session::SpotHealingMode;

        let entries = menu_entries(
            [
                (SpotHealingMode::ContentAware, "Content Aware"),
                (SpotHealingMode::CreateTexture, "Create Texture"),
                (SpotHealingMode::ProximityMatch, "Proximity Match"),
            ],
            SpotHealingMode::CreateTexture,
            |session, value| session.spot_healing_mode = value,
        );
        let labels: Vec<_> = entries.iter().map(|entry| entry.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Content Aware", "Create Texture", "Proximity Match"]
        );
        let checked: Vec<_> = entries.iter().map(|entry| entry.checked).collect();
        assert_eq!(checked, [false, true, false]);
        assert!(
            entries
                .iter()
                .all(|entry| entry.enabled && !entry.separator)
        );

        // A String field compares by content against its &'static str
        // presets, exactly like the inline crop construction did.
        let entries = menu_entries(
            [
                ("Free", "Free"),
                ("Original", "Original"),
                ("1:1", "1:1"),
                ("4:3", "4:3"),
                ("16:9", "16:9"),
            ],
            "4:3".to_string(),
            |session, value| session.crop_ratio_choice = value.to_string(),
        );
        let checked: Vec<_> = entries.iter().map(|entry| entry.checked).collect();
        assert_eq!(checked, [false, false, false, true, false]);
    }

    /// EditorSession.swift:347-351: entering Crop with no frame presets the
    /// full-document frame AND resets the ratio choice to Free — a ratio
    /// left over from an earlier crop must not reshape the preset.
    #[test]
    fn crop_entry_preset_resets_a_stale_ratio_choice() {
        let mut session = EditorSession::new();
        session.create_document(800, 600, false);
        session.crop_ratio_choice = "4:3".to_string();

        crop_entry_preset(&mut session, NavigationTool::Crop);
        assert_eq!(session.crop_ratio_choice, "Free");
        assert_eq!(session.crop_rect, Some(CGRect::new(0.0, 0.0, 800.0, 600.0)));

        // An existing frame is never replaced, and its ratio choice stands.
        let framed = CGRect::new(10.0, 10.0, 100.0, 100.0);
        session.crop_rect = Some(framed);
        session.crop_ratio_choice = "1:1".to_string();
        crop_entry_preset(&mut session, NavigationTool::Crop);
        assert_eq!(session.crop_rect, Some(framed));
        assert_eq!(session.crop_ratio_choice, "1:1");
    }
}
