//! Port of the brush family's option bar (`BrushControls.swift`): the four
//! sliders every brush-family tool shares, plus each tool's mode dropdown
//! or sample checkboxes. The bar shell itself stays in
//! ui/navigation_tool_header.rs.

use gpui_kit::{Context, IntoElement, px};

use compositor_core::document::NavigationTool;
use compositor_core::session::{BlurToolMode, BrushToolMode, SpotHealingMode};

use crate::state::AppState;
use crate::ui::navigation_tool_header::menu_entries;
use crate::ui::{
    Dropdown, LabeledCheckbox, LabeledSlider, app_dispatch, app_on_change, text_label,
};

impl AppState {
    /// `BrushControls.swift`'s bar controls, in its order: Size, Hardness,
    /// Opacity, Smoothing, then the per-tool mode dropdown (Brush /
    /// Spot Healing / Smear) or the Clone Stamp checkboxes.
    pub(crate) fn brush_controls(&mut self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let app = cx.entity().downgrade();
        let brush = self.tab().session.read(cx).brush_settings;
        let (tool, erasing) = {
            let session = self.tab().session.read(cx);
            (session.tool, session.brush_mode == BrushToolMode::Erase)
        };

        let mut controls = vec![
            LabeledSlider::new(
                "Size",
                self.brush_sliders[0].clone(),
                format!("{:.0}", brush.diameter),
                px(90.0),
            )
            .into_any_element(),
            LabeledSlider::new(
                "Hardness",
                self.brush_sliders[1].clone(),
                format!("{:.0}%", brush.hardness * 100.0),
                px(70.0),
            )
            .into_any_element(),
            LabeledSlider::new(
                "Opacity",
                self.brush_sliders[2].clone(),
                format!("{:.0}%", brush.opacity * 100.0),
                px(70.0),
            )
            .into_any_element(),
            LabeledSlider::new(
                "Smoothing",
                self.brush_sliders[3].clone(),
                format!("{:.0}", brush.smoothing),
                px(60.0),
            )
            .into_any_element(),
        ];
        if tool == NavigationTool::Brush {
            let mode = self.tab().session.read(cx).brush_mode;
            controls.push(
                Dropdown::new("brush-mode", if erasing { "Erase" } else { "Paint" })
                    .with_width(px(80.0))
                    .with_entries(menu_entries(
                        [
                            (BrushToolMode::Paint, "Paint"),
                            (BrushToolMode::Erase, "Erase"),
                        ],
                        mode,
                        |session, value| session.brush_mode = value,
                    ))
                    .with_dispatch(app_dispatch(&app))
                    .into_any_element(),
            );
        }
        if tool == NavigationTool::SpotHealing {
            let mode = self.tab().session.read(cx).spot_healing_mode;
            let label = |mode: SpotHealingMode| match mode {
                SpotHealingMode::ContentAware => "Content Aware",
                SpotHealingMode::CreateTexture => "Create Texture",
                SpotHealingMode::ProximityMatch => "Proximity Match",
            };
            controls.push(
                Dropdown::new("heal-mode", label(mode))
                    .with_width(px(150.0))
                    .with_entries(menu_entries(
                        [
                            (SpotHealingMode::ContentAware, "Content Aware"),
                            (SpotHealingMode::CreateTexture, "Create Texture"),
                            (SpotHealingMode::ProximityMatch, "Proximity Match"),
                        ],
                        mode,
                        |session, value| session.spot_healing_mode = value,
                    ))
                    .with_dispatch(app_dispatch(&app))
                    .into_any_element(),
            );
        }
        if tool == NavigationTool::Blur {
            let mode = self.tab().session.read(cx).blur_mode;
            let label = |mode: BlurToolMode| match mode {
                BlurToolMode::Liquify => "Liquify",
                BlurToolMode::Blur => "Blur",
                BlurToolMode::Smudge => "Smudge",
            };
            controls.push(
                Dropdown::new("blur-mode", label(mode))
                    .with_width(px(96.0))
                    .with_entries(menu_entries(
                        [
                            (BlurToolMode::Liquify, "Liquify"),
                            (BlurToolMode::Blur, "Blur"),
                            (BlurToolMode::Smudge, "Smudge"),
                        ],
                        mode,
                        |session, value| session.blur_mode = value,
                    ))
                    .with_dispatch(app_dispatch(&app))
                    .into_any_element(),
            );
        }
        if tool == NavigationTool::CloneStamp {
            let (aligned, all_layers) = {
                let clone = self.tab().session.read(cx).clone_settings;
                (clone.aligned, clone.sample_all_layers)
            };
            controls.push(
                LabeledCheckbox::new("clone-aligned", "Aligned")
                    .checked(aligned)
                    .with_on_change(app_on_change(&app, |state, checked, _, cx| {
                        state
                            .tab()
                            .session
                            .update(cx, |session, _| session.clone_settings.aligned = checked);
                        state.touch(cx);
                    }))
                    .into_any_element(),
            );
            controls.push(
                LabeledCheckbox::new("clone-all-layers", "Sample All Layers")
                    .checked(all_layers)
                    .with_on_change(app_on_change(&app, |state, checked, _, cx| {
                        state.tab().session.update(cx, |session, _| {
                            session.clone_settings.sample_all_layers = checked
                        });
                        state.touch(cx);
                    }))
                    .into_any_element(),
            );
            controls.push(text_label("Option-click sets the source", true).into_any_element());
        }
        controls
    }
}
