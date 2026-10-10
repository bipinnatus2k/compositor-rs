//! Port of the brush family's option bar (`BrushControls.swift`): the four
//! sliders every brush-family tool shares, plus each tool's mode dropdown
//! or sample checkboxes. The bar is a view entity (ADR 0003 step E, ticket
//! 42) holding the session and the slider entities; the bar shell itself
//! stays in ui/navigation_tool_header.rs.

use gpui_kit::{AppContext, Context, Entity, IntoElement, Subscription, WeakEntity, px};

use gpui_kit::component::slider::{SliderEvent, SliderState};

use compositor_core::document::NavigationTool;
use compositor_core::session::EditorSession;
use compositor_core::session::{BlurToolMode, BrushToolMode, SpotHealingMode};

use crate::state::{AppState, slider};
use crate::ui::navigation_tool_header::menu_entries;
use crate::ui::{Dropdown, LabeledCheckbox, LabeledSlider, app_dispatch, app_on, text_label};

/// The brush family's option bar (`BrushControls.swift`): a view entity
/// since ticket 42 — it owns the four slider entities and their write
/// subscriptions (`subscribe_sliders`' brush slice). Only in the render
/// tree while a brush-family tool is active (Swift's
/// `if session.tool.isBrushTool`), so it keeps no session observer; its
/// session field is re-pointed on tab changes by
/// [`AppState::sync_session_observer`].
pub struct BrushControlsView {
    /// The active tab's session — the readouts and the slider writes speak
    /// to it directly (`BrushControls`' `@Bindable session`).
    session: Entity<EditorSession>,
    /// The window shell (the transitional root): the repaint epoch and the
    /// shared header row stay there. No AppState dependencies beyond what
    /// the moved handlers already used.
    app: WeakEntity<AppState>,
    /// The Size/Hardness/Opacity/Smoothing sliders (`BrushControls`' four
    /// slider bindings): seeded from the persisted tool defaults at
    /// construction, exactly as `AppState::new` seeded them when the fields
    /// lived there.
    brush_sliders: [Entity<SliderState>; 4],
    /// Keep-alive for the slider subscriptions: dropping a Subscription
    /// unsubscribes, so the bundle is held, never read.
    #[allow(dead_code)]
    subscriptions: Vec<Subscription>,
}

impl BrushControlsView {
    pub(crate) fn new(
        session: Entity<EditorSession>,
        app: WeakEntity<AppState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let brush = session.read(cx).brush_settings;
        let brush_sliders = [
            cx.new(|_| slider(1.0, 300.0, 1.0, brush.diameter as f32)),
            cx.new(|_| slider(0.0, 100.0, 1.0, (brush.hardness * 100.0) as f32)),
            cx.new(|_| slider(1.0, 100.0, 1.0, (brush.opacity * 100.0) as f32)),
            cx.new(|_| slider(0.0, 100.0, 1.0, brush.smoothing as f32)),
        ];
        let mut view = BrushControlsView {
            session,
            app,
            brush_sliders,
            subscriptions: Vec::new(),
        };
        view.subscribe_sliders(cx);
        view
    }

    /// Re-points the session on tab changes (`AppState::sync_session_observer`,
    /// same as the canvas view and the layers panel). The slider
    /// subscriptions read this field at event time, so the re-point keeps
    /// them writing to the active tab.
    pub(crate) fn set_session(&mut self, session: Entity<EditorSession>) {
        self.session = session;
    }

    /// The slider entities, for the shell's brush flows (`resize_brush`,
    /// `change_brush_hardness`, `opacity_digit` — the writes from outside
    /// this view).
    pub(crate) fn sliders(&self) -> &[Entity<SliderState>; 4] {
        &self.brush_sliders
    }

    /// Keeps the session's brush settings in sync with the sliders
    /// (`subscribe_sliders`' brush slice, moved with the sliders): a Change
    /// or Release writes the value and bumps the repaint epoch.
    fn subscribe_sliders(&mut self, cx: &mut Context<Self>) {
        for index in [0usize, 1, 2, 3] {
            let slider = self.brush_sliders[index].clone();
            let subscription =
                cx.subscribe(&slider, move |this, _slider, event: &SliderEvent, cx| {
                    let value = match event {
                        SliderEvent::Change(value) | SliderEvent::Release(value) => value.start(),
                    };
                    this.session.update(cx, |session, _| {
                        let brush = &mut session.brush_settings;
                        match index {
                            0 => brush.diameter = value as f64,
                            1 => brush.hardness = (value / 100.0) as f64,
                            2 => brush.opacity = (value / 100.0) as f64,
                            _ => brush.smoothing = value as f64,
                        }
                    });
                    this.touch(cx);
                });
            self.subscriptions.push(subscription);
        }
    }

    /// The view's route to the app-side repaint epoch (`touch`): the frame
    /// cache and `revision` stay on AppState (ticket 39's field audit), so
    /// an interaction that changed the session asks the shell through the
    /// seam.
    fn touch(&mut self, cx: &mut Context<Self>) {
        self.app.update(cx, |app, cx| app.touch(cx)).ok();
    }

    /// `BrushControls.swift`'s bar controls, in its order: Size, Hardness,
    /// Opacity, Smoothing, then the per-tool mode dropdown (Brush /
    /// Spot Healing / Smear) or the Clone Stamp checkboxes.
    pub(crate) fn brush_controls(&mut self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let app = self.app.clone();
        let brush = self.session.read(cx).brush_settings;
        let (tool, erasing) = {
            let session = self.session.read(cx);
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
            let mode = self.session.read(cx).brush_mode;
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
            let mode = self.session.read(cx).spot_healing_mode;
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
            let mode = self.session.read(cx).blur_mode;
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
                let clone = self.session.read(cx).clone_settings;
                (clone.aligned, clone.sample_all_layers)
            };
            controls.push(
                LabeledCheckbox::new("clone-aligned", "Aligned")
                    .checked(aligned)
                    .with_on_change(app_on(&app, |state, checked, _, cx| {
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
                    .with_on_change(app_on(&app, |state, checked, _, cx| {
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
