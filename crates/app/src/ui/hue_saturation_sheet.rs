//! Port of the Hue/Saturation sheet (`HueSaturationSheet.swift`) as its own
//! view entity (ADR 0003 step D, ticket 41): the panel owns the three
//! sliders and the Colorize toggle — the sheet view's `@State` — and renders
//! its body and footer; the shell keeps the preview recompute and the commit
//! flows, reached through the panel's seam.
//!
//! (`HueSaturationSheet.swift:132`'s `@State dragging` belongs to
//! SpectrumEditor, the spectrum-bars sub-view; that surface has no
//! counterpart in this port, so the audit's dragging ruling is honored
//! vacuously — nothing to re-host.)

use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription,
    WeakEntity, Window,
};

use compositor_core::adjust::{ColorRange, HueSaturationSettings, RangeAdjustment};
use compositor_core::session::EditorSession;

use crate::sheets_ui::SheetRow;
use crate::state::ProjectWorkspaceView;
use crate::ui::LabeledCheckbox;
use crate::ui::floating_panel::{
    PanelPlacement, panel_body_frame, panel_footer, panel_names, panel_on,
};
use crate::ui::sheets::panel_value;

/// The Hue/Saturation panel as its own view entity (Swift
/// `HueSaturationSheet`'s edit state; ticket 41). Created by
/// `ProjectWorkspaceView::open_hue_saturation_panel`, dropped on close, so its session
/// can never go stale (tab switches are blocked while any panel is open).
pub struct HueSaturationPanelView {
    /// The session this panel edits. Unread today — every interaction
    /// funnels through the shell seam, and the panel's lifetime sits inside
    /// one modal session (tab switches blocked) so it can never go stale.
    /// Carried for the house view-entity convention; removal is a ticket-44
    /// cleanup candidate.
    #[allow(dead_code)]
    session: Entity<EditorSession>,
    /// The root view (the window shell): the preview recompute and
    /// the commit/cancel flows stay there.
    app: WeakEntity<ProjectWorkspaceView>,
    /// The three sliders: hue (-180…180), saturation and lightness
    /// (-100…100).
    sliders: Vec<Entity<SliderState>>,
    /// The Colorize toggle.
    colorize: bool,
    subscriptions: Vec<Subscription>,
}

impl HueSaturationPanelView {
    pub(crate) fn new(
        session: Entity<EditorSession>,
        app: WeakEntity<ProjectWorkspaceView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut panel = HueSaturationPanelView {
            session,
            app,
            sliders: Vec::new(),
            colorize: false,
            subscriptions: Vec::new(),
        };
        panel.sliders = vec![
            panel.make_slider(-180.0, 180.0, 0.0, cx),
            panel.make_slider(-100.0, 100.0, 0.0, cx),
            panel.make_slider(-100.0, 100.0, 0.0, cx),
        ];
        panel
    }

    /// Creates one of the panel's sliders. Every slider funnels its changes
    /// into [`Self::slider_changed`] — the write-back reads all three
    /// sliders, so no per-slider role is needed.
    fn make_slider(
        &mut self,
        min: f32,
        max: f32,
        value: f32,
        cx: &mut Context<Self>,
    ) -> Entity<SliderState> {
        let state = cx.new(|_| {
            crate::state::slider(min, max, if max - min > 12.0 { 1.0 } else { 0.1 }, value)
        });
        let subscription = cx.subscribe(&state, |this, _slider, event: &SliderEvent, cx| {
            let value = match event {
                SliderEvent::Change(value) | SliderEvent::Release(value) => value.start(),
            };
            this.slider_changed(value, cx);
        });
        self.subscriptions.push(subscription);
        state
    }

    /// The settings the panel's sliders and Colorize toggle currently spell
    /// (`apply_slider`'s Hue branch head, moved with the widgets).
    fn settings(&self, cx: &gpui_kit::App) -> HueSaturationSettings {
        let hue = panel_value(&self.sliders, cx, 0) as f64;
        let saturation = panel_value(&self.sliders, cx, 1) as f64;
        let lightness = panel_value(&self.sliders, cx, 2) as f64;
        let mut settings = HueSaturationSettings::new(hue, saturation, lightness, self.colorize);
        if self.colorize {
            settings.adjustments.insert(
                ColorRange::Master,
                RangeAdjustment {
                    hue,
                    // Colorize's saturation is an HSL amount 0…1
                    // (HueSaturation.swift:301); zero must stay zero.
                    saturation: (saturation / 100.0).clamp(0.0, 1.0),
                    lightness,
                },
            );
        }
        settings
    }

    /// A slider moved (or Colorize flipped — the old checkbox handler routed
    /// through `apply_slider(Hue, …)` too): the shell recomputes the live
    /// preview from this panel's settings.
    fn slider_changed(&mut self, _value: f32, cx: &mut Context<Self>) {
        let settings = self.settings(cx);
        self.app
            .update(cx, |app, cx| app.apply_hue_saturation_preview(settings, cx))
            .ok();
    }
}

impl Render for HueSaturationPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let colorize = self.colorize;
        let sliders = self.sliders.clone();

        let body =
            gpui_kit::div()
                .flex()
                .flex_col()
                .gap_1()
                .child(SheetRow::new("Hue").with_optional_control(
                    sliders.first().map(|slider| Slider::new(slider).flex_1()),
                ))
                .child(SheetRow::new("Saturation").with_optional_control(
                    sliders.get(1).map(|slider| Slider::new(slider).flex_1()),
                ))
                .child(SheetRow::new("Lightness").with_optional_control(
                    sliders.get(2).map(|slider| Slider::new(slider).flex_1()),
                ))
                .child(
                    SheetRow::new("Colorize").with_control(
                        LabeledCheckbox::new("hue-colorize", "Colorize")
                            .checked(colorize)
                            .with_on_change(panel_on(&this, |panel, checked, _, cx| {
                                panel.colorize = checked;
                                panel.slider_changed(0.0, cx);
                            })),
                    ),
                )
                .into_any_element();
        let footer = panel_footer(
            panel_names::ADJUSTMENT_PANEL,
            &self.app,
            false,
            |root, _, cx| root.cancel_hue_saturation_panel(cx),
            |root, _, cx| root.confirm_hue_saturation_panel(cx),
        );
        panel_body_frame(body, footer)
    }
}

impl ProjectWorkspaceView {
    /// `session.hueSaturation != nil` → adjustmentPanel, title
    /// "Hue/Saturation" (`ContentView.swift:208-215`).
    pub fn open_hue_saturation_panel(&mut self, cx: &mut Context<Self>) {
        self.begin_preview(cx);
        let session = self.workspace.read(cx).current().session.clone();
        let app = cx.entity().downgrade();
        let panel = cx.new(|cx| HueSaturationPanelView::new(session, app, cx));
        self.panel_views.insert(
            panel_names::ADJUSTMENT_PANEL,
            crate::ui::floating_panel::PanelContent::HueSaturation(panel),
        );
        self.panels.show(
            panel_names::ADJUSTMENT_PANEL,
            "Hue/Saturation",
            PanelPlacement::Automatic,
        );
        self.sync_panel_guard(cx);
        self.touch(cx);
    }

    /// `cancelHueSaturation` (`ContentView.swift:210`).
    pub fn cancel_hue_saturation_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.close(panel_names::ADJUSTMENT_PANEL) {
            return;
        }
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .adjustment_editing_id
            .is_some()
        {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.finish_adjustment_editing(false))
            });
        }
        self.panel_views.remove(panel_names::ADJUSTMENT_PANEL);
        self.after_panel_editing_closed(cx);
    }

    pub fn confirm_hue_saturation_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.is_visible(panel_names::ADJUSTMENT_PANEL) {
            return;
        }
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .adjustment_editing_id
            .is_some()
        {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.finish_adjustment_editing(true))
            });
        } else {
            self.commit_preview_pixels("Hue/Saturation", cx);
        }
        self.panel_views.remove(panel_names::ADJUSTMENT_PANEL);
        self.panels.close(panel_names::ADJUSTMENT_PANEL);
        self.after_panel_editing_closed(cx);
    }
}
