//! Port of the brush family's option bar (`BrushControls.swift`): the four
//! sliders every brush-family tool shares, plus each tool's mode dropdown
//! or sample checkboxes. The bar shell itself stays in
//! ui/navigation_tool_header.rs.

use gpui_kit::{Context, IntoElement, px};

use compositor_core::document::NavigationTool;
use compositor_core::session::{BlurToolMode, BrushToolMode, SpotHealingMode};

use crate::state::AppState;
use crate::ui::navigation_tool_header::{checkbox, menu_entries};
use crate::ui::{dropdown, labeled_slider, text_label};

impl AppState {
    /// `BrushControls.swift`'s bar controls, in its order: Size, Hardness,
    /// Opacity, Smoothing, then the per-tool mode dropdown (Brush /
    /// Spot Healing / Smear) or the Clone Stamp checkboxes.
    pub(crate) fn brush_controls(&mut self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let app = cx.entity().downgrade();
        let brush = self.tab().session.brush_settings;
        let tool = self.tab().session.tool;
        let erasing = self.tab().session.brush_mode == BrushToolMode::Erase;

        let mut controls = vec![
            labeled_slider(
                "Size",
                &self.brush_sliders[0],
                format!("{:.0}", brush.diameter),
                px(90.0),
            )
            .into_any_element(),
            labeled_slider(
                "Hardness",
                &self.brush_sliders[1],
                format!("{:.0}%", brush.hardness * 100.0),
                px(70.0),
            )
            .into_any_element(),
            labeled_slider(
                "Opacity",
                &self.brush_sliders[2],
                format!("{:.0}%", brush.opacity * 100.0),
                px(70.0),
            )
            .into_any_element(),
            labeled_slider(
                "Smoothing",
                &self.brush_sliders[3],
                format!("{:.0}", brush.smoothing),
                px(60.0),
            )
            .into_any_element(),
        ];
        if tool == NavigationTool::Brush {
            let mode = self.tab().session.brush_mode;
            controls.push(
                dropdown(
                    &app,
                    "brush-mode",
                    if erasing { "Erase" } else { "Paint" },
                    px(80.0),
                    menu_entries(
                        [
                            (BrushToolMode::Paint, "Paint"),
                            (BrushToolMode::Erase, "Erase"),
                        ],
                        mode,
                        |session, value| session.brush_mode = value,
                    ),
                )
                .into_any_element(),
            );
        }
        if tool == NavigationTool::SpotHealing {
            let mode = self.tab().session.spot_healing_mode;
            let label = |mode: SpotHealingMode| match mode {
                SpotHealingMode::ContentAware => "Content Aware",
                SpotHealingMode::CreateTexture => "Create Texture",
                SpotHealingMode::ProximityMatch => "Proximity Match",
            };
            controls.push(
                dropdown(
                    &app,
                    "heal-mode",
                    label(mode),
                    px(150.0),
                    menu_entries(
                        [
                            (SpotHealingMode::ContentAware, "Content Aware"),
                            (SpotHealingMode::CreateTexture, "Create Texture"),
                            (SpotHealingMode::ProximityMatch, "Proximity Match"),
                        ],
                        mode,
                        |session, value| session.spot_healing_mode = value,
                    ),
                )
                .into_any_element(),
            );
        }
        if tool == NavigationTool::Blur {
            let mode = self.tab().session.blur_mode;
            let label = |mode: BlurToolMode| match mode {
                BlurToolMode::Liquify => "Liquify",
                BlurToolMode::Blur => "Blur",
                BlurToolMode::Smudge => "Smudge",
            };
            controls.push(
                dropdown(
                    &app,
                    "blur-mode",
                    label(mode),
                    px(96.0),
                    menu_entries(
                        [
                            (BlurToolMode::Liquify, "Liquify"),
                            (BlurToolMode::Blur, "Blur"),
                            (BlurToolMode::Smudge, "Smudge"),
                        ],
                        mode,
                        |session, value| session.blur_mode = value,
                    ),
                )
                .into_any_element(),
            );
        }
        if tool == NavigationTool::CloneStamp {
            let aligned = self.tab().session.clone_settings.aligned;
            let all_layers = self.tab().session.clone_settings.sample_all_layers;
            controls.push(checkbox(
                &app,
                "clone-aligned",
                "Aligned",
                aligned,
                |state, checked, _, cx| {
                    state.tab().session.clone_settings.aligned = checked;
                    state.touch(cx);
                },
            ));
            controls.push(checkbox(
                &app,
                "clone-all-layers",
                "Sample All Layers",
                all_layers,
                |state, checked, _, cx| {
                    state.tab().session.clone_settings.sample_all_layers = checked;
                    state.touch(cx);
                },
            ));
            controls.push(text_label("Option-click sets the source", true).into_any_element());
        }
        controls
    }
}
