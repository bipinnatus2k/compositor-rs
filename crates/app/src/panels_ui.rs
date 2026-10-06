//! The floating panels' content (`ContentView.swift`'s `onChange` wiring plus
//! the panel views): per-panel widget state, the open/OK/Cancel actions and
//! the bodies rendered inside [`render_panel`]'s shell.
//!
//! Each migrated modal sheet keeps its semantics: OK runs the sheet's commit
//! branch, Cancel — including the title-bar ✕, which Swift routes through
//! `onClose` — runs the cancel branch, and the sliders feed the same live
//! preview the sheet did. The panel registry lives on
//! [`AppState::panels`](crate::state::AppState); the widgets on
//! [`PanelWidgets`]. Both are window-level: a panel survives a canvas pan,
//! and while any panel is open the workspace's `can_switch` guard blocks tab
//! switches (Swift's `canStartProjectOperation` counts the same edit states;
//! the Rust core only carries them for adjustment editing, so the app adds
//! `ui_panels_open`).
//!
//! Panels keep their Swift placements: Camera Raw's filter panel opens
//! `DockedToMainWindowRight`, everything else `Automatic` (centered on the
//! canvas the first time, then wherever it was left).

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::component::{Disableable, Selectable, Sizable};
use gpui_kit::{
    AnyElement, AppContext, Context, InteractiveElement, IntoElement, ParentElement, Pixels, Point,
    SharedString, Styled, Window, px,
};

use compositor_core::adjust::LevelsChannel;
use compositor_core::adjust::levels_auto::LevelsAuto;
use compositor_core::effects::LayerEffectKind;
use compositor_core::filters::FilterKind;
use compositor_core::session::SelectionAmountOperation;

use crate::panels::{DOCKED_WIDTH, PanelPlacement, docked_top_left, panel_names, render_panel};
use crate::sheets::SliderRole;
use crate::sheets_ui::{
    histogram_view, labeled_slider, levels_channel_label, parse_input_usize, sheet_row,
};
use crate::state::AppState;
use crate::ui::{MenuEntry, dropdown, labeled_checkbox, text_label};

/// The per-panel widget bundles. A panel's bundle exists exactly while the
/// panel is open — the same lifetime the sheet's `SheetState` had.
#[derive(Default)]
pub struct PanelWidgets {
    pub levels: Option<LevelsPanelState>,
    pub hue_saturation: Option<HueSaturationPanelState>,
    pub amount: Option<AmountPanelState>,
    pub filter: Option<FilterPanelState>,
    pub effects: Option<EffectsPanelState>,
}

/// The Levels panel's widgets (`LevelsSheet`'s edit state): the five sliders,
/// the visible channel, the Preview toggle and the histogram, plus the full
/// settings the Auto modes write wholesale.
pub struct LevelsPanelState {
    pub sliders: Vec<gpui_kit::Entity<SliderState>>,
    pub channel: usize,
    pub preview: bool,
    pub histogram: [f64; 1024],
    pub levels: compositor_core::adjust::LevelsSettings,
}

/// The Hue/Saturation panel's widgets (`HueSaturationSheet`).
pub struct HueSaturationPanelState {
    pub sliders: Vec<gpui_kit::Entity<SliderState>>,
    pub colorize: bool,
}

/// The selection-amount panel's widgets (`SelectionAmountSheet`): the amount
/// field, its 1…maximum companion slider (`LassoControls.swift:224-228`) and
/// which operation is being sized.
pub struct AmountPanelState {
    pub operation: SelectionAmountOperation,
    pub tolerance: gpui_kit::Entity<InputState>,
    pub slider: gpui_kit::Entity<SliderState>,
}

/// The filter panel's widgets (`FilterSheet`): which filter is open plus its
/// sliders and Add Noise's two checkboxes. Curves keeps its editor on
/// `AppState.curves_editor`, exactly as the sheet did.
pub struct FilterPanelState {
    pub kind: FilterKind,
    pub sliders: Vec<gpui_kit::Entity<SliderState>>,
    /// Add Noise's Distribution (Swift's Uniform/Gaussian picker).
    pub gaussian: bool,
    /// Add Noise's Monochromatic toggle.
    pub monochromatic: bool,
}

/// The Effects panel's widgets (`EffectsSheet`): which effect is being edited
/// (an index into `LayerEffectKind::ALL`), its sliders, and Stroke's
/// Outside/Inside position.
pub struct EffectsPanelState {
    pub kind_index: usize,
    pub sliders: Vec<gpui_kit::Entity<SliderState>>,
    pub inside: bool,
}

impl AppState {
    // MARK: Opening

    /// Keeps the workspace's tab-switch guard in step with the panel stack.
    pub(crate) fn sync_panel_guard(&mut self) {
        self.workspace.ui_panels_open = !self.panels.open_panels().is_empty();
    }

    /// `session.levels != nil` → levelsPanel (`ContentView.swift:198-206`),
    /// title "Levels", `Automatic` placement. Opening captures the active
    /// layer for the live preview and computes the histogram, exactly as the
    /// sheet's `build_sheet` did.
    pub fn open_levels_panel(&mut self, cx: &mut Context<Self>) {
        self.begin_preview();
        let mut panel = LevelsPanelState {
            sliders: vec![
                self.make_slider(0.0, 254.0, 0.0, SliderRole::LevelsInputBlack, cx),
                self.make_slider(0.1, 9.99, 1.0, SliderRole::LevelsGamma, cx),
                self.make_slider(1.0, 255.0, 255.0, SliderRole::LevelsWhite, cx),
                self.make_slider(0.0, 255.0, 0.0, SliderRole::LevelsOutBlack, cx),
                self.make_slider(0.0, 255.0, 255.0, SliderRole::LevelsOutWhite, cx),
            ],
            channel: 0,
            // Swift's LevelsEdit.preview (LevelsSheet's Preview toggle): on when opened.
            preview: true,
            histogram: [0.0; 1024],
            levels: compositor_core::adjust::LevelsSettings::default(),
        };
        if let Some(pixels) = self.preview_base.clone() {
            // With a selection, bins are weighted by the selection's coverage
            // on the layer's pixel grid (LevelsFilter.histogram,
            // Levels.swift:90-96).
            let coverage = crate::sheets::histogram_coverage(
                self.preview_base_transform,
                self.tab().session.selection().cloned(),
                pixels.width,
                pixels.height,
            );
            compositor_core::adjust::levels_histogram(
                pixels.as_ref(),
                coverage.as_ref(),
                &mut panel.histogram,
            );
        }
        self.panel_widgets.levels = Some(panel);
        self.panels.show(
            panel_names::LEVELS_PANEL,
            "Levels",
            PanelPlacement::Automatic,
        );
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// `session.hueSaturation != nil` → adjustmentPanel, title
    /// "Hue/Saturation" (`ContentView.swift:208-215`).
    pub fn open_hue_saturation_panel(&mut self, cx: &mut Context<Self>) {
        self.begin_preview();
        let panel = HueSaturationPanelState {
            sliders: vec![
                self.make_slider(-180.0, 180.0, 0.0, SliderRole::Hue, cx),
                self.make_slider(-100.0, 100.0, 0.0, SliderRole::Saturation, cx),
                self.make_slider(-100.0, 100.0, 0.0, SliderRole::Lightness, cx),
            ],
            colorize: false,
        };
        self.panel_widgets.hue_saturation = Some(panel);
        self.panels.show(
            panel_names::ADJUSTMENT_PANEL,
            "Hue/Saturation",
            PanelPlacement::Automatic,
        );
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// `session.selectionAmountOperation` → selectionAmountPanel, title
    /// "{Operation} Selection" (`ContentView.swift:227-234`). The operation
    /// rides on the core session, which is what blocks project operations
    /// while the panel is open.
    pub fn open_amount_panel(
        &mut self,
        operation: SelectionAmountOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tab().session.selection_amount_operation.is_some() {
            return;
        }
        self.tab().session.selection_amount_operation = Some(operation);
        let session = &self.tab().session;
        let amount = match operation {
            SelectionAmountOperation::Expand => session.selection_expand_amount,
            SelectionAmountOperation::Contract => session.selection_contract_amount,
            SelectionAmountOperation::Feather => session.selection_feather_amount,
        };
        let tolerance = cx.new(|cx| InputState::new(window, cx).default_value(amount.to_string()));
        let maximum = amount_maximum(operation);
        let slider = cx.new(|_| {
            SliderState::new()
                .min(1.0)
                .max(maximum as f32)
                .step(1.0)
                .default_value(amount as f32)
        });
        // Slider → field: the slider writes the input's text
        // (`SelectionAmountSheet`: `set: { input = String(Int($0.rounded())) }`).
        let watched = tolerance.clone();
        let slider_subscription = cx.subscribe_in(
            &slider,
            window,
            move |_this, _slider, event: &SliderEvent, window, cx| {
                let value = match event {
                    SliderEvent::Change(value) | SliderEvent::Release(value) => value.end(),
                };
                let text = amount_text_for_slider(value, maximum);
                watched.update(cx, |state, cx| {
                    if state.value() != text {
                        state.set_value(text, window, cx);
                    }
                });
            },
        );
        self.subscriptions.push(slider_subscription);
        // Field → slider: the thumb re-derives from the parsed amount and
        // jumps to 1 while the text is invalid (`get: { Double(amount ?? 1) }`).
        // `InputState::set_value` and `SliderState::set_value` both suppress
        // their events, so the two-way sync cannot ping-pong.
        let watched = slider.clone();
        let input_subscription = cx.subscribe_in(
            &tolerance,
            window,
            move |_this, input, event: &InputEvent, window, cx| {
                if let InputEvent::Change = event {
                    // Read through the same entity the event came from.
                    let text = input.read(cx).value().to_string();
                    let value = amount_slider_value(&text, maximum);
                    watched.update(cx, |state, cx| {
                        state.set_value(SliderValue::Single(value), window, cx);
                    });
                }
            },
        );
        self.subscriptions.push(input_subscription);
        self.panel_widgets.amount = Some(AmountPanelState {
            operation,
            tolerance,
            slider,
        });
        let title = match operation {
            SelectionAmountOperation::Feather => "Feather Selection",
            SelectionAmountOperation::Expand => "Expand Selection",
            SelectionAmountOperation::Contract => "Contract Selection",
        };
        self.panels.show(
            panel_names::SELECTION_AMOUNT_PANEL,
            title,
            PanelPlacement::Automatic,
        );
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// `session.filterEdit != nil` → filterPanel (`ContentView.swift:236-246`):
    /// title = the filter's label; Camera Raw opens
    /// `DockedToMainWindowRight`, every other filter `Automatic`.
    pub fn open_filter_panel(&mut self, kind: FilterKind, cx: &mut Context<Self>) {
        self.begin_preview();
        let mut panel = FilterPanelState {
            kind,
            sliders: Vec::new(),
            gaussian: false,
            monochromatic: false,
        };
        let defaults = compositor_core::filters::FilterSettings::default();
        match kind {
            FilterKind::GaussianBlur => {
                panel.sliders = vec![self.make_slider(
                    0.1,
                    250.0,
                    defaults.radius as f32,
                    SliderRole::Radius,
                    cx,
                )];
            }
            FilterKind::MotionBlur => {
                panel.sliders = vec![
                    self.make_slider(-90.0, 90.0, defaults.angle as f32, SliderRole::Angle, cx),
                    self.make_slider(
                        1.0,
                        2000.0,
                        defaults.distance as f32,
                        SliderRole::Distance,
                        cx,
                    ),
                ];
            }
            FilterKind::AddNoise => {
                panel.sliders = vec![self.make_slider(
                    0.1,
                    400.0,
                    defaults.amount as f32,
                    SliderRole::Amount,
                    cx,
                )];
            }
            FilterKind::LensCorrection => {
                panel.sliders =
                    vec![self.make_slider(-100.0, 100.0, 0.0, SliderRole::Distortion, cx)];
            }
            FilterKind::CameraRaw => {
                let group: [(f64, f64, f64); 14] = [
                    (-100.0, 100.0, 0.0), // Temperature
                    (-100.0, 100.0, 0.0), // Tint
                    (-5.0, 5.0, 0.0),     // Exposure
                    (-100.0, 100.0, 0.0), // Contrast
                    (-100.0, 100.0, 0.0), // Highlights
                    (-100.0, 100.0, 0.0), // Shadows
                    (-100.0, 100.0, 0.0), // Whites
                    (-100.0, 100.0, 0.0), // Blacks
                    (-100.0, 100.0, 0.0), // Vibrance
                    (-100.0, 100.0, 0.0), // Saturation
                    (-100.0, 100.0, 0.0), // Texture
                    (-100.0, 100.0, 0.0), // Clarity
                    (-100.0, 100.0, 0.0), // Dehaze
                    (-100.0, 100.0, 0.0), // Vignette Amount
                ];
                panel.sliders = group
                    .iter()
                    .enumerate()
                    .map(|(i, (min, max, value))| {
                        self.make_slider(
                            *min as f32,
                            *max as f32,
                            *value as f32,
                            SliderRole::CameraRaw(i),
                            cx,
                        )
                    })
                    .collect();
            }
            // Curves edits through `curves_editor`; the automatic filters
            // (Black & White, Color Balance, Exposure, Gradient Map, Grain)
            // run with their default settings and show no sliders.
            _ => {}
        }
        // A fresh filter replaces any previous editor state (the sheet dropped
        // `curves_editor` in `close_sheet`).
        self.curves_editor = if kind == FilterKind::Curves {
            Some(crate::state::CurvesEditor::new())
        } else {
            None
        };
        self.panel_widgets.filter = Some(panel);
        let placement = if kind == FilterKind::CameraRaw {
            PanelPlacement::DockedToMainWindowRight
        } else {
            PanelPlacement::Automatic
        };
        self.panels
            .show(panel_names::FILTER_PANEL, kind.label(), placement);
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// `session.effectsEditing` → effectsPanel, title = the effect's name
    /// (`ContentView.swift:217-224`). The caller (`add_effect`) has already
    /// started the editing session.
    pub fn open_effects_panel(&mut self, cx: &mut Context<Self>) {
        // No preview capture: every slider change is committed to the layer's
        // effects right away (one undo step per change, as Swift's
        // changeEffects does).
        let editing = self.tab().session.editing_effects();
        let kind = self
            .tab()
            .session
            .effects_editing
            .map(|editing| editing.kind)
            .unwrap_or(LayerEffectKind::Stroke);
        let mut slider = |min: f64, max: f64, value: f64, index: usize| {
            self.make_slider(
                min as f32,
                max as f32,
                value as f32,
                SliderRole::Effect(index),
                cx,
            )
        };
        // Slider thumb ranges are the EffectsSheet's visible ranges; the
        // core `isValid` guard bounds every stored value.
        let sliders = match kind {
            LayerEffectKind::Stroke => vec![
                slider(0.0, 20.0, editing.stroke.map(|s| s.size).unwrap_or(4.0), 0),
                slider(
                    0.0,
                    100.0,
                    editing.stroke.map(|s| s.opacity * 100.0).unwrap_or(100.0),
                    1,
                ),
            ],
            LayerEffectKind::Shadow => vec![
                slider(
                    0.0,
                    100.0,
                    editing.shadow.map(|s| s.opacity * 100.0).unwrap_or(50.0),
                    0,
                ),
                slider(
                    -180.0,
                    180.0,
                    editing.shadow.map(|s| s.angle).unwrap_or(90.0),
                    1,
                ),
                slider(
                    0.0,
                    100.0,
                    editing.shadow.map(|s| s.distance).unwrap_or(20.0),
                    2,
                ),
                slider(
                    0.0,
                    100.0,
                    editing.shadow.map(|s| s.blur).unwrap_or(20.0),
                    3,
                ),
            ],
            LayerEffectKind::ColorOverlay => vec![slider(
                0.0,
                100.0,
                editing
                    .color_overlay
                    .map(|s| s.opacity * 100.0)
                    .unwrap_or(100.0),
                0,
            )],
            LayerEffectKind::InnerShadow => vec![
                slider(
                    0.0,
                    100.0,
                    editing
                        .inner_shadow
                        .map(|s| s.opacity * 100.0)
                        .unwrap_or(50.0),
                    0,
                ),
                slider(
                    -180.0,
                    180.0,
                    editing.inner_shadow.map(|s| s.angle).unwrap_or(90.0),
                    1,
                ),
                slider(
                    0.0,
                    50.0,
                    editing.inner_shadow.map(|s| s.distance).unwrap_or(10.0),
                    2,
                ),
                slider(
                    0.0,
                    100.0,
                    editing.inner_shadow.map(|s| s.blur).unwrap_or(10.0),
                    3,
                ),
            ],
            LayerEffectKind::OuterGlow => vec![
                slider(
                    0.0,
                    100.0,
                    editing.outer_glow.map(|s| s.size).unwrap_or(20.0),
                    0,
                ),
                slider(
                    0.0,
                    100.0,
                    editing
                        .outer_glow
                        .map(|s| s.opacity * 100.0)
                        .unwrap_or(75.0),
                    1,
                ),
            ],
            LayerEffectKind::InnerGlow => vec![
                slider(
                    0.0,
                    100.0,
                    editing.inner_glow.map(|s| s.size).unwrap_or(10.0),
                    0,
                ),
                slider(
                    0.0,
                    100.0,
                    editing
                        .inner_glow
                        .map(|s| s.opacity * 100.0)
                        .unwrap_or(75.0),
                    1,
                ),
            ],
        };
        let kind_index = LayerEffectKind::ALL
            .iter()
            .position(|candidate| *candidate == kind)
            .unwrap_or(0);
        // Stroke's Outside/Inside position rides in `inside`.
        let inside = editing.stroke.map(|stroke| stroke.inside).unwrap_or(false);
        self.panel_widgets.effects = Some(EffectsPanelState {
            kind_index,
            sliders,
            inside,
        });
        self.panels.show(
            panel_names::EFFECTS_PANEL,
            kind.label(),
            PanelPlacement::Automatic,
        );
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// `ShortcutSettings.show()` (`KeyboardShortcuts.swift:155-157`): title
    /// "Keyboard Shortcuts". The editor state (baseline draft, recording row,
    /// keystroke interceptor) is created by the body's first render, as the
    /// sheet did.
    pub fn open_shortcuts_panel(&mut self, cx: &mut Context<Self>) {
        self.panels.show(
            panel_names::KEYBOARD_SHORTCUTS,
            "Keyboard Shortcuts",
            PanelPlacement::Automatic,
        );
        self.sync_panel_guard();
        self.touch(cx);
    }

    // MARK: Closing, Cancel, OK

    /// The title-bar ✕ and Esc route here (`onClose` semantics = Cancel).
    pub fn cancel_panel(&mut self, name: &str, cx: &mut Context<Self>) {
        match name {
            panel_names::LEVELS_PANEL => self.cancel_levels_panel(cx),
            panel_names::ADJUSTMENT_PANEL => self.cancel_hue_saturation_panel(cx),
            panel_names::SELECTION_AMOUNT_PANEL => self.cancel_amount_panel(cx),
            panel_names::FILTER_PANEL => self.cancel_filter_panel(cx),
            panel_names::EFFECTS_PANEL => self.cancel_effects_panel(cx),
            panel_names::COLOR_PICKER_PANEL => self.close_color_picker_ui(false, cx),
            panel_names::KEYBOARD_SHORTCUTS => {
                // The draft is discarded: the registry returns to its state
                // when the panel opened (KeyboardShortcutsSheet's Cancel).
                Self::shortcuts_cancel_draft();
                self.close_shortcuts_panel(cx);
                // Re-sync the live keymap with the restored registry.
                self.rebuild_keymap(cx);
            }
            _ => {}
        }
    }

    /// A panel footer's OK button — the sheet's commit branch.
    pub fn confirm_panel(&mut self, name: &str, cx: &mut Context<Self>) {
        match name {
            panel_names::LEVELS_PANEL => self.confirm_levels_panel(cx),
            panel_names::ADJUSTMENT_PANEL => self.confirm_hue_saturation_panel(cx),
            panel_names::SELECTION_AMOUNT_PANEL => self.confirm_amount_panel(cx),
            panel_names::FILTER_PANEL => self.confirm_filter_panel(cx),
            panel_names::EFFECTS_PANEL => self.confirm_effects_panel(cx),
            _ => {}
        }
    }

    /// `cancelLevels` (`LevelsSheet.swift:79`): an adjustment editing session
    /// restores the opening settings; a pixel-layer session just drops the
    /// preview.
    pub fn cancel_levels_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.close(panel_names::LEVELS_PANEL) {
            return;
        }
        if self.tab().session.adjustment_editing_id.is_some() {
            self.tab().session.finish_adjustment_editing(false);
        }
        self.panel_widgets.levels = None;
        self.after_panel_editing_closed(cx);
    }

    /// `commitLevels` (`LevelsSheet.swift:82`): the settings previewed live
    /// are kept as one undo step.
    pub fn confirm_levels_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.is_visible(panel_names::LEVELS_PANEL) {
            return;
        }
        // Editing an adjustment layer: the previewed settings are the commit.
        if self.tab().session.adjustment_editing_id.is_some() {
            self.tab().session.finish_adjustment_editing(true);
        } else {
            self.commit_preview_pixels("Levels", cx);
        }
        self.panel_widgets.levels = None;
        self.panels.close(panel_names::LEVELS_PANEL);
        self.after_panel_editing_closed(cx);
    }

    /// `cancelHueSaturation` (`ContentView.swift:210`).
    pub fn cancel_hue_saturation_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.close(panel_names::ADJUSTMENT_PANEL) {
            return;
        }
        if self.tab().session.adjustment_editing_id.is_some() {
            self.tab().session.finish_adjustment_editing(false);
        }
        self.panel_widgets.hue_saturation = None;
        self.after_panel_editing_closed(cx);
    }

    pub fn confirm_hue_saturation_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.is_visible(panel_names::ADJUSTMENT_PANEL) {
            return;
        }
        if self.tab().session.adjustment_editing_id.is_some() {
            self.tab().session.finish_adjustment_editing(true);
        } else {
            self.commit_preview_pixels("Hue/Saturation", cx);
        }
        self.panel_widgets.hue_saturation = None;
        self.panels.close(panel_names::ADJUSTMENT_PANEL);
        self.after_panel_editing_closed(cx);
    }

    /// `onClose = { session.selectionAmountOperation = nil }`
    /// (`ContentView.swift:230`): nothing was committed.
    pub fn cancel_amount_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.close(panel_names::SELECTION_AMOUNT_PANEL) {
            return;
        }
        self.tab().session.selection_amount_operation = None;
        self.panel_widgets.amount = None;
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// OK (`SelectionAmountSheet.swift:224`): runs the operation the session
    /// is holding. The footer keeps OK disabled while the amount is outside
    /// the operation's range, so this always runs with a valid amount.
    pub fn confirm_amount_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.is_visible(panel_names::SELECTION_AMOUNT_PANEL) {
            return;
        }
        let amount = {
            let cx_app: &gpui_kit::App = cx;
            self.panel_widgets
                .amount
                .as_ref()
                .map(|panel| parse_input_usize(&panel.tolerance, cx_app).unwrap_or(0))
                .unwrap_or(0) as i64
        };
        self.tab().session.confirm_selection_amount(amount);
        self.panel_widgets.amount = None;
        self.panels.close(panel_names::SELECTION_AMOUNT_PANEL);
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// The amount field's validity, driving the footer's disabled OK:
    /// a whole number within the operation's range
    /// (`SelectionAmountSheet.amount`, maximum 250 for Feather, else 500).
    pub fn amount_input_valid(&self, cx: &gpui_kit::App) -> bool {
        let Some(panel) = self.panel_widgets.amount.as_ref() else {
            return false;
        };
        parse_amount(
            &panel.tolerance.read(cx).value(),
            amount_maximum(panel.operation),
        )
        .is_some()
    }

    /// `cancelFilter` (`FilterSheet.swift`): an adjustment editing session
    /// restores the opening settings; the preview is dropped.
    pub fn cancel_filter_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.close(panel_names::FILTER_PANEL) {
            return;
        }
        if self.tab().session.adjustment_editing_id.is_some() {
            self.tab().session.finish_adjustment_editing(false);
        }
        self.panel_widgets.filter = None;
        self.curves_editor = None;
        self.after_panel_editing_closed(cx);
    }

    /// `commitFilter` (`FilterSheet.swift`): the filter's previewed pixels
    /// become one undo step named after the filter.
    pub fn confirm_filter_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.is_visible(panel_names::FILTER_PANEL) {
            return;
        }
        if self.tab().session.adjustment_editing_id.is_some() {
            self.tab().session.finish_adjustment_editing(true);
        } else {
            let name = self
                .panel_widgets
                .filter
                .as_ref()
                .map(|panel| panel.kind.label())
                .unwrap_or("Filter")
                .to_string();
            self.commit_preview_pixels(&name, cx);
        }
        self.panel_widgets.filter = None;
        self.curves_editor = None;
        self.panels.close(panel_names::FILTER_PANEL);
        self.after_panel_editing_closed(cx);
    }

    /// `finishEffectsEditing(commit: false)` (`EffectsSheet.swift:20`).
    pub fn cancel_effects_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.close(panel_names::EFFECTS_PANEL) {
            return;
        }
        self.tab().session.finish_effects_editing(false);
        self.panel_widgets.effects = None;
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// `finishEffectsEditing(commit: true)` (`EffectsSheet.swift:22`): every
    /// change was already a committed undo step; OK just ends the session.
    pub fn confirm_effects_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.is_visible(panel_names::EFFECTS_PANEL) {
            return;
        }
        self.tab().session.finish_effects_editing(true);
        self.panel_widgets.effects = None;
        self.panels.close(panel_names::EFFECTS_PANEL);
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// `ShortcutSettings.close` after Done saved the draft
    /// (`KeyboardShortcuts.swift:162-166`).
    pub fn close_shortcuts_panel(&mut self, cx: &mut Context<Self>) {
        self.panels.close(panel_names::KEYBOARD_SHORTCUTS);
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// Rebuilds the app keymap from the live shortcut registry
    /// (`keys::rebuild_bindings`): the component baseline is snapshotted and
    /// replayed, the app's own bindings are regenerated, so a saved or
    /// restored chord takes effect on the next keystroke without a restart.
    fn rebuild_keymap(&mut self, cx: &mut Context<Self>) {
        let app: &mut gpui_kit::App = cx;
        crate::keys::rebuild_with(app, &self.shortcuts());
    }

    /// The panel counterpart of `close_sheet`'s preview teardown: the live
    /// adjustment/filter preview is dropped, so the canvas shows the layer's
    /// own pixels again.
    fn after_panel_editing_closed(&mut self, cx: &mut Context<Self>) {
        self.panel_widgets.effects = None;
        self.preview = None;
        self.preview_base = None;
        self.preview_base_transform = None;
        self.sync_panel_guard();
        self.touch(cx);
    }

    // MARK: Rendering

    /// Renders every open panel as an overlay above the workspace. The
    /// stacking order from [`FloatingPanels::open_panels`] is preserved, so
    /// the topmost panel paints last.
    pub fn render_panels_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let names: Vec<&'static str> = self
            .panels
            .open_panels()
            .iter()
            .map(|panel| panel.name)
            .collect();
        names
            .into_iter()
            .filter_map(|name| self.render_one_panel(name, window, cx))
            .collect()
    }

    fn render_one_panel(
        &mut self,
        name: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let app = cx.entity().downgrade();
        let entry = self.panels.open_panels().iter().find(|p| p.name == name)?;
        let title = entry.title.clone();
        let placement = entry.placement;
        let viewport = window.viewport_size();
        let top_left = match placement {
            PanelPlacement::DockedToMainWindowRight => {
                let (x, y) =
                    docked_top_left((f32::from(viewport.width), f32::from(viewport.height)));
                Point::new(px(x), px(y))
            }
            PanelPlacement::Automatic => {
                let (x, y) = self.panels.position(
                    name,
                    self.panel_canvas_center(window),
                    panel_estimated_size(name),
                )?;
                Point::new(px(x), px(y))
            }
        };
        let fixed_size = if placement == PanelPlacement::DockedToMainWindowRight {
            Some((px(DOCKED_WIDTH), viewport.height))
        } else {
            None
        };

        let content = self.panel_content(name, cx)?;

        let app = app.clone();
        let on_close =
            move |state: &mut AppState, _window: &mut Window, cx: &mut Context<AppState>| {
                state.cancel_panel(name, cx);
            };
        let on_moved = move |state: &mut AppState,
                             position: Point<Pixels>,
                             _window: &mut Window,
                             cx: &mut Context<AppState>| {
            state
                .panels
                .remember_position(name, f32::from(position.x), f32::from(position.y));
            state.notify_only(cx);
        };
        Some(render_panel(
            &app, top_left, title, fixed_size, content, on_close, on_moved,
        ))
    }

    /// The canvas area's center (`canvasCenter(in:)`): the panel's center
    /// lands there on the first open. Uses the canvas paint area's bounds;
    /// before the first layout pass it falls back to the window's center.
    fn panel_canvas_center(&self, window: &mut Window) -> (f32, f32) {
        let viewport = self.viewport;
        let (w, h) = (
            f32::from(viewport.size.width),
            f32::from(viewport.size.height),
        );
        if w > 1.0 && h > 1.0 {
            (
                f32::from(viewport.origin.x) + w / 2.0,
                f32::from(viewport.origin.y) + h / 2.0,
            )
        } else {
            let size = window.viewport_size();
            (f32::from(size.width) / 2.0, f32::from(size.height) / 2.0)
        }
    }

    /// One panel's content: the view body plus its footer (Swift's Cancel /
    /// OK row). The color picker carries its own OK/Cancel in its layout and
    /// the Shortcuts panel its Restore/Cancel/Done footer.
    fn panel_content(&mut self, name: &'static str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let body = match name {
            panel_names::LEVELS_PANEL => self.levels_panel_body(cx),
            panel_names::ADJUSTMENT_PANEL => self.hue_saturation_panel_body(cx),
            panel_names::SELECTION_AMOUNT_PANEL => self.amount_panel_body(cx),
            panel_names::FILTER_PANEL => self.filter_panel_body(cx),
            panel_names::EFFECTS_PANEL => self.effects_panel_body(cx),
            panel_names::COLOR_PICKER_PANEL => {
                return crate::color_picker_ui::color_picker_panel_body(self, cx);
            }
            panel_names::KEYBOARD_SHORTCUTS => {
                return Some(self.shortcuts_panel_footer_body(cx));
            }
            _ => return None,
        };
        let ok_disabled = if name == panel_names::SELECTION_AMOUNT_PANEL {
            let cx_app: &gpui_kit::App = cx;
            !self.amount_input_valid(cx_app)
        } else {
            false
        };
        let footer = panel_footer(name, ok_disabled, cx);
        Some(
            gpui_kit::div()
                .flex()
                .flex_col()
                .px_3()
                .py_2()
                .gap_1()
                .child(body)
                .child(footer)
                .into_any_element(),
        )
    }

    /// The Shortcuts panel's body plus its own footer
    /// (`KeyboardShortcutsSheet.swift:270-278`): Restore Defaults, Cancel
    /// (discards the draft) and Done (saves it, disabled while a row records
    /// or the draft conflicts).
    fn shortcuts_panel_footer_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let body = self.shortcuts_sheet_body(cx);
        let done_disabled =
            Self::shortcuts_recording_id().is_some() || self.shortcuts().validate().is_err();
        gpui_kit::div()
            .flex()
            .flex_col()
            .px_3()
            .py_2()
            .gap_1()
            .child(body)
            .child(
                gpui_kit::div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("shortcuts-restore")
                            .label("Restore Defaults")
                            .small()
                            .outline()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.shortcuts().reset_all();
                                // Restore Defaults applies immediately: the
                                // keymap is rebuilt from the reset registry.
                                this.rebuild_keymap(cx);
                                this.notify_only(cx);
                            })),
                    )
                    .child(gpui_kit::div().flex_1())
                    .child(
                        Button::new("panel-cancel")
                            .label("Cancel")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cancel_panel(panel_names::KEYBOARD_SHORTCUTS, cx);
                            })),
                    )
                    .child(
                        Button::new("shortcuts-done")
                            .label("Done")
                            .small()
                            .primary()
                            .disabled(done_disabled)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.shortcuts_done(cx);
                                // Done rebuilt nothing while the draft was
                                // being edited; save closed the panel, and
                                // the saved registry now drives a live
                                // keymap rebuild (no restart needed).
                                this.rebuild_keymap(cx);
                                this.set_status("Shortcuts saved", cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    // MARK: Panel bodies (moved from the sheets)

    /// `LevelsSheet`'s body: Preview toggle, channel picker, histogram, five
    /// sliders, and the Auto trio (`LevelsAutomatic.swift`'s modes).
    fn levels_panel_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.entity().downgrade();
        let Some(panel) = self.panel_widgets.levels.as_ref() else {
            return gpui_kit::div().into_any_element();
        };
        let channel = panel.channel;
        let histogram = panel.histogram;
        let sliders = panel.sliders.clone();
        let preview = panel.preview;

        gpui_kit::div()
            .flex()
            .flex_col()
            .gap_1()
            .child(sheet_row(
                "Preview",
                labeled_checkbox(
                    &app,
                    "levels-preview",
                    "Preview",
                    preview,
                    |state, checked, _, cx| {
                        if let Some(panel) = state.panel_widgets.levels.as_mut() {
                            panel.preview = checked;
                        }
                        state.refresh_levels_preview(cx);
                    },
                )
                .into_any_element(),
            ))
            .child(sheet_row(
                "Channel",
                dropdown(
                    &app,
                    "levels-channel",
                    match channel {
                        1 => "Red",
                        2 => "Green",
                        3 => "Blue",
                        _ => "RGB",
                    },
                    px(90.0),
                    vec![
                        MenuEntry::new("RGB", |state, _, cx| set_levels_channel(state, 0, cx)),
                        MenuEntry::new("Red", |state, _, cx| set_levels_channel(state, 1, cx)),
                        MenuEntry::new("Green", |state, _, cx| set_levels_channel(state, 2, cx)),
                        MenuEntry::new("Blue", |state, _, cx| set_levels_channel(state, 3, cx)),
                    ],
                ),
            ))
            .child(
                gpui_kit::div()
                    .w_full()
                    .h(px(96.0))
                    .rounded_sm()
                    .overflow_hidden()
                    .child(histogram_view(histogram)),
            )
            .child(labeled_slider(&sliders, 0, "Input Black"))
            .child(labeled_slider(&sliders, 1, "Gamma"))
            .child(labeled_slider(&sliders, 2, "Input White"))
            .child(labeled_slider(&sliders, 3, "Output Black"))
            .child(labeled_slider(&sliders, 4, "Output White"))
            .child(sheet_row(
                "Auto",
                gpui_kit::div()
                    .flex()
                    .flex_row()
                    .gap_1()
                    .child(auto_button(
                        "levels-auto-contrast",
                        "Contrast",
                        LevelsAuto::Contrast,
                        cx,
                    ))
                    .child(auto_button(
                        "levels-auto-color",
                        "Color",
                        LevelsAuto::Color,
                        cx,
                    ))
                    .child(auto_button(
                        "levels-auto-neutral",
                        "Neutral",
                        LevelsAuto::Neutral,
                        cx,
                    )),
            ))
            .into_any_element()
    }

    /// `HueSaturationSheet`'s body: three sliders plus the Colorize toggle.
    fn hue_saturation_panel_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.entity().downgrade();
        let Some(panel) = self.panel_widgets.hue_saturation.as_ref() else {
            return gpui_kit::div().into_any_element();
        };
        let colorize = panel.colorize;
        let sliders = panel.sliders.clone();
        gpui_kit::div()
            .flex()
            .flex_col()
            .gap_1()
            .child(labeled_slider(&sliders, 0, "Hue"))
            .child(labeled_slider(&sliders, 1, "Saturation"))
            .child(labeled_slider(&sliders, 2, "Lightness"))
            .child(sheet_row(
                "Colorize",
                labeled_checkbox(
                    &app,
                    "hue-colorize",
                    "Colorize",
                    colorize,
                    |state, checked, _, cx| {
                        if let Some(panel) = state.panel_widgets.hue_saturation.as_mut() {
                            panel.colorize = checked;
                        }
                        state.apply_slider(SliderRole::Hue, 0.0, cx);
                    },
                )
                .into_any_element(),
            ))
            .into_any_element()
    }

    /// `SelectionAmountSheet`'s body: the Amount field with its 1…maximum
    /// companion slider (`LassoControls.swift:220-231`), plus the invalid
    /// hint line. Field and slider stay in two-way sync.
    fn amount_panel_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(panel) = self.panel_widgets.amount.as_ref() else {
            return gpui_kit::div().into_any_element();
        };
        let tolerance = panel.tolerance.clone();
        let slider = panel.slider.clone();
        let maximum = amount_maximum(panel.operation);
        let cx_app: &gpui_kit::App = cx;
        let valid = parse_amount(&tolerance.read(cx_app).value(), maximum).is_some();
        let mut body = gpui_kit::div().flex().flex_col().gap_1().child(sheet_row(
            "Amount",
            gpui_kit::div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .flex_1()
                .child(
                    gpui_kit::div()
                        .flex_1()
                        .min_w_0()
                        .child(Slider::new(&slider)),
                )
                .child(Input::new(&tolerance).id("amount-value").w(px(64.0))),
        ));
        if !valid {
            body = body.child(text_label(
                format!("Enter a whole number from 1 to {maximum} px."),
                true,
            ));
        }
        body.into_any_element()
    }

    /// `FilterSheet`'s body: the open filter's controls.
    fn filter_panel_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.entity().downgrade();
        let Some(panel) = self.panel_widgets.filter.as_ref() else {
            return gpui_kit::div().into_any_element();
        };
        let kind = panel.kind;
        let sliders = panel.sliders.clone();
        let mut body = gpui_kit::div().flex().flex_col().gap_1();
        match kind {
            FilterKind::Curves => {
                // The curve editor (CurvesControls.swift): channel picker, the
                // curve canvas with its click/drag gesture, a selected-point
                // readout with Remove, and Reset. The preview runs through the
                // same filter pipeline as the slider sheets.
                if self.curves_editor.is_none() {
                    self.curves_editor = Some(crate::state::CurvesEditor::new());
                }
                let (curves, selected) = {
                    let editor = self.curves_editor.as_ref().unwrap();
                    (editor.settings.clone(), editor.selected)
                };
                let channel = curves.channel;
                let points = curves.channels[channel.index()].clone();
                let readout = selected
                    .and_then(|index| points.get(index))
                    .map(|point| {
                        format!("Input {} \u{b7} Output {}", point.x as i64, point.y as i64)
                    })
                    .unwrap_or_default();
                let can_remove =
                    selected.is_some_and(|index| index > 0 && index + 1 < points.len());
                body = body
                    .child(sheet_row(
                        "Channel",
                        dropdown(
                            &app,
                            "curves-channel",
                            levels_channel_label(channel),
                            px(90.0),
                            LevelsChannel::ALL
                                .iter()
                                .map(|choice| {
                                    let choice = *choice;
                                    MenuEntry::new(
                                        levels_channel_label(choice),
                                        move |state, _, cx| {
                                            state.curves_set_channel(choice, cx);
                                        },
                                    )
                                    .checked(channel == choice)
                                })
                                .collect(),
                        ),
                    ))
                    .child(
                        gpui_kit::div()
                            .id("curves-canvas")
                            .w_full()
                            .h(px(240.0))
                            .rounded_sm()
                            .overflow_hidden()
                            .border_1()
                            .border_color(gpui_kit::rgb(crate::ui::BORDER_INSET))
                            .on_mouse_down(
                                gpui_kit::MouseButton::Left,
                                cx.listener(|this, event: &gpui_kit::MouseDownEvent, _, cx| {
                                    this.curves_mouse_down(event, cx);
                                }),
                            )
                            .child(crate::sheets_ui::curves_canvas_view(
                                &curves, &points, selected, cx,
                            )),
                    )
                    .child(text_label("Click to add a point. Drag to adjust.", true))
                    .child(
                        gpui_kit::div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(text_label(readout, false))
                            .child(gpui_kit::div().flex_1())
                            .child(
                                Button::new("curves-remove")
                                    .label("Remove point")
                                    .small()
                                    .outline()
                                    .disabled(!can_remove)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.curves_remove_point(cx);
                                    })),
                            )
                            .child(
                                Button::new("curves-reset")
                                    .label("Reset curve")
                                    .small()
                                    .outline()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.curves_reset(cx);
                                    })),
                            ),
                    );
            }
            FilterKind::GaussianBlur => {
                body = body.child(labeled_slider(&sliders, 0, "Radius"));
            }
            FilterKind::MotionBlur => {
                body = body
                    .child(labeled_slider(&sliders, 0, "Angle"))
                    .child(labeled_slider(&sliders, 1, "Distance"));
            }
            FilterKind::AddNoise => {
                let gaussian = panel.gaussian;
                let monochromatic = panel.monochromatic;
                body = body
                    .child(labeled_slider(&sliders, 0, "Amount"))
                    .child(sheet_row(
                        "Distribution",
                        labeled_checkbox(
                            &app,
                            "noise-gaussian",
                            "Gaussian",
                            gaussian,
                            |state, checked, _, cx| {
                                if let Some(panel) = state.panel_widgets.filter.as_mut() {
                                    panel.gaussian = checked;
                                }
                                state.refresh_filter_preview(cx);
                            },
                        )
                        .into_any_element(),
                    ))
                    .child(sheet_row(
                        "",
                        labeled_checkbox(
                            &app,
                            "noise-mono",
                            "Monochromatic",
                            monochromatic,
                            |state, checked, _, cx| {
                                if let Some(panel) = state.panel_widgets.filter.as_mut() {
                                    panel.monochromatic = checked;
                                }
                                state.refresh_filter_preview(cx);
                            },
                        )
                        .into_any_element(),
                    ));
            }
            FilterKind::LensCorrection => {
                body = body.child(labeled_slider(&sliders, 0, "Distortion"));
            }
            FilterKind::CameraRaw => {
                body = body
                    .child(text_label("White Balance", false).mb_1())
                    .child(labeled_slider(&sliders, 0, "Temperature"))
                    .child(labeled_slider(&sliders, 1, "Tint"))
                    .child(text_label("Light", false).mb_1())
                    .child(labeled_slider(&sliders, 2, "Exposure"))
                    .child(labeled_slider(&sliders, 3, "Contrast"))
                    .child(labeled_slider(&sliders, 4, "Highlights"))
                    .child(labeled_slider(&sliders, 5, "Shadows"))
                    .child(labeled_slider(&sliders, 6, "Whites"))
                    .child(labeled_slider(&sliders, 7, "Blacks"))
                    .child(text_label("Color", false).mb_1())
                    .child(labeled_slider(&sliders, 8, "Vibrance"))
                    .child(labeled_slider(&sliders, 9, "Saturation"))
                    .child(text_label("Effects", false).mb_1())
                    .child(labeled_slider(&sliders, 10, "Texture"))
                    .child(labeled_slider(&sliders, 11, "Clarity"))
                    .child(labeled_slider(&sliders, 12, "Dehaze"))
                    .child(labeled_slider(&sliders, 13, "Vignette"));
            }
            _ => {
                body = body.child(text_label(
                    "This filter runs with its default settings.",
                    true,
                ));
            }
        }
        body.into_any_element()
    }

    /// `EffectsSheet`'s body: the edited effect's controls. Stroke's Position
    /// is the segmented Outside/Inside picker (`EffectsSheet.swift:37-42`),
    /// rendered in the title row like Swift's headline row.
    fn effects_panel_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(panel) = self.panel_widgets.effects.as_ref() else {
            return gpui_kit::div().into_any_element();
        };
        let kind = LayerEffectKind::ALL
            .get(panel.kind_index)
            .copied()
            .unwrap_or(LayerEffectKind::Stroke);
        let inside = panel.inside;
        let sliders = panel.sliders.clone();
        let color = self.tab().session.editing_effects().color(kind);
        let title_row = |cx: &mut Context<Self>| {
            let mut row = gpui_kit::div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(text_label(kind.label(), false).text_size(px(13.0)))
                .child(gpui_kit::div().flex_1());
            if kind == LayerEffectKind::Stroke {
                row = row
                    .child(stroke_segment_button(0, inside, cx))
                    .child(stroke_segment_button(1, inside, cx));
            }
            row
        };
        let mut body = gpui_kit::div()
            .flex()
            .flex_col()
            .gap_1()
            .child(title_row(cx));
        match kind {
            LayerEffectKind::Stroke => {
                body = body
                    .child(sheet_row("Color", effect_swatch(color, kind, cx)))
                    .child(labeled_slider(&sliders, 0, "Size (px)"))
                    .child(labeled_slider(&sliders, 1, "Opacity (%)"));
            }
            LayerEffectKind::Shadow => {
                body = body
                    .child(sheet_row("Color", effect_swatch(color, kind, cx)))
                    .child(labeled_slider(&sliders, 0, "Opacity (%)"))
                    .child(labeled_slider(&sliders, 1, "Angle (°)"))
                    .child(labeled_slider(&sliders, 2, "Distance (px)"))
                    .child(labeled_slider(&sliders, 3, "Blur (px)"));
            }
            LayerEffectKind::ColorOverlay => {
                body = body
                    .child(sheet_row("Color", effect_swatch(color, kind, cx)))
                    .child(labeled_slider(&sliders, 0, "Opacity (%)"));
            }
            LayerEffectKind::InnerShadow => {
                body = body
                    .child(sheet_row("Color", effect_swatch(color, kind, cx)))
                    .child(labeled_slider(&sliders, 0, "Opacity (%)"))
                    .child(labeled_slider(&sliders, 1, "Angle (°)"))
                    .child(labeled_slider(&sliders, 2, "Distance (px)"))
                    .child(labeled_slider(&sliders, 3, "Blur (px)"));
            }
            LayerEffectKind::OuterGlow | LayerEffectKind::InnerGlow => {
                body = body
                    .child(sheet_row("Color", effect_swatch(color, kind, cx)))
                    .child(labeled_slider(&sliders, 0, "Size (px)"))
                    .child(labeled_slider(&sliders, 1, "Opacity (%)"));
            }
        }
        body.into_any_element()
    }
}

/// The Stroke position picker's segments in Swift `Picker` order
/// (`EffectsSheet.swift:39-41`: Outside, then Inside).
const STROKE_POSITION_SEGMENTS: [&str; 2] = ["Outside", "Inside"];

/// Whether `segment` (an index into [`STROKE_POSITION_SEGMENTS`]) is the
/// selected one for a stroke at `inside` — the picker's `get` binding
/// (`get: { effect.inside }` with `Text("Outside").tag(false)`,
/// `Text("Inside").tag(true)`).
fn stroke_segment_selected(inside: bool, segment: usize) -> bool {
    (segment == 1) == inside
}

/// The position a click on `segment` writes — the picker's `set` binding.
fn stroke_segment_target(segment: usize) -> bool {
    segment == 1
}

/// One segment of the Stroke position picker: two side-by-side toggle
/// buttons, the selected one on the accent (gpui-component's `selected`
/// button style), standing in for Swift's `NSSegmentedControl`-style
/// `.pickerStyle(.segmented)`.
fn stroke_segment_button(
    segment: usize,
    inside: bool,
    cx: &mut Context<AppState>,
) -> gpui_kit::AnyElement {
    Button::new(SharedString::from(format!("stroke-position-{segment}")))
        .label(STROKE_POSITION_SEGMENTS[segment])
        .small()
        .selected(stroke_segment_selected(inside, segment))
        .on_click(cx.listener(move |state, _, _window, cx| {
            set_stroke_inside(state, stroke_segment_target(segment), cx);
        }))
        .into_any_element()
}

/// Stroke's Outside/Inside change goes through the session like every slider,
/// so the canvas and the undo trail follow (`EffectsSheet.swift`'s picker).
fn set_stroke_inside(state: &mut AppState, inside: bool, cx: &mut Context<AppState>) {
    state.tab().session.change_effects(|effects| {
        if let Some(stroke) = effects.stroke.as_mut() {
            stroke.inside = inside;
        }
    });
    if let Some(panel) = state.panel_widgets.effects.as_mut() {
        panel.inside = inside;
    }
    state.touch(cx);
}

/// The selection-amount slider's upper bound (`SelectionAmountSheet.maximum`:
/// 250 for Feather, 500 otherwise).
pub(crate) fn amount_maximum(operation: SelectionAmountOperation) -> i64 {
    if operation == SelectionAmountOperation::Feather {
        250
    } else {
        500
    }
}

/// Parses the amount field: a whole number in 1…maximum
/// (`SelectionAmountSheet.amount`).
pub(crate) fn parse_amount(text: &str, maximum: i64) -> Option<i64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed
        .parse::<i64>()
        .ok()
        .filter(|value| (1..=maximum).contains(value))
}

/// The slider thumb position for the amount text — the slider binding's
/// getter, `Double(amount ?? 1)` (`LassoControls.swift:225`): an invalid
/// field parks the thumb at 1.
pub(crate) fn amount_slider_value(text: &str, maximum: i64) -> f32 {
    parse_amount(text, maximum).unwrap_or(1) as f32
}

/// The amount text for a slider position — the binding's setter,
/// `String(Int($0.rounded()))`, clamped to the operation's range.
pub(crate) fn amount_text_for_slider(value: f32, maximum: i64) -> String {
    format!("{}", (value.round() as i64).clamp(1, maximum))
}

/// The Levels channel dropdown's write-back: only the visible channel
/// changes, exactly as the sheet's `set_channel` did.
fn set_levels_channel(state: &mut AppState, channel: usize, cx: &mut Context<AppState>) {
    if let Some(panel) = state.panel_widgets.levels.as_mut() {
        panel.channel = channel;
    }
    state.notify_only(cx);
}

/// One Auto button (`LevelsSheet.swift:64`): the mode's settings replace the
/// edit's wholesale, then the sliders and the preview follow.
fn auto_button(
    id: &'static str,
    label: &'static str,
    mode: LevelsAuto,
    cx: &mut Context<AppState>,
) -> AnyElement {
    Button::new(id)
        .label(label)
        .small()
        .outline()
        .on_click(cx.listener(move |this, _, window, cx| {
            this.levels_auto_mode(mode, window, cx);
        }))
        .into_any_element()
}

/// The effect's color swatch, opening the color picker panel on click
/// (EffectsSheet's color rows). Unset components show the effect's default.
fn effect_swatch(
    color: Option<compositor_core::document::PaletteColor>,
    kind: LayerEffectKind,
    cx: &mut Context<AppState>,
) -> gpui_kit::AnyElement {
    use gpui_kit::{StatefulInteractiveElement as _, Styled as _};
    let color = color.unwrap_or_default();
    let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    let rgb = (channel(color.red) << 16) | (channel(color.green) << 8) | channel(color.blue);
    gpui_kit::div()
        .id(SharedString::from(format!(
            "effect-swatch-{}",
            kind.label()
        )))
        .w(px(36.0))
        .h(px(18.0))
        .rounded_xs()
        .border_1()
        .border_color(gpui_kit::rgb(crate::ui::BORDER_INSET))
        .bg(gpui_kit::rgb(rgb))
        .on_click(cx.listener(move |state, _, _, cx| {
            state.open_effect_color_picker(kind, cx);
        }))
        .into_any_element()
}

/// The shared Cancel/OK footer row (Swift's `HStack { Cancel; Spacer; OK }`).
/// OK runs the sheet's commit branch; Cancel — like the ✕ — runs the cancel
/// branch.
fn panel_footer(
    name: &'static str,
    ok_disabled: bool,
    cx: &mut Context<AppState>,
) -> gpui_kit::Div {
    gpui_kit::div()
        .flex()
        .flex_row()
        .justify_end()
        .gap_2()
        .child(
            Button::new(SharedString::from(format!("panel-{name}-cancel")))
                .label("Cancel")
                .small()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.cancel_panel(name, cx);
                })),
        )
        .child(
            Button::new(SharedString::from(format!("panel-{name}-ok")))
                .label("OK")
                .small()
                .primary()
                .disabled(ok_disabled)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.confirm_panel(name, cx);
                })),
        )
}

/// A first-open size estimate per panel (`fittingSize`): only used to center
/// the panel on the canvas the first time; afterwards the remembered position
/// wins and the panel hugs its content.
pub fn panel_estimated_size(name: &str) -> (f32, f32) {
    match name {
        // Histogram (96) + five sliders + rows.
        panel_names::LEVELS_PANEL => (440.0, 390.0),
        // Three sliders + the Colorize row.
        panel_names::ADJUSTMENT_PANEL => (440.0, 220.0),
        panel_names::SELECTION_AMOUNT_PANEL => (340.0, 110.0),
        // Curves is the tallest of the filter bodies.
        panel_names::FILTER_PANEL => (440.0, 420.0),
        panel_names::EFFECTS_PANEL => (440.0, 240.0),
        panel_names::COLOR_PICKER_PANEL => (540.0, 350.0),
        panel_names::KEYBOARD_SHORTCUTS => (560.0, 560.0),
        _ => (440.0, 300.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::panel_names;

    /// Every panel has a positive first-open size estimate; the docked width
    /// is the widest a docked panel assumes.
    #[test]
    fn panel_size_estimates_are_positive() {
        for name in [
            panel_names::LEVELS_PANEL,
            panel_names::ADJUSTMENT_PANEL,
            panel_names::SELECTION_AMOUNT_PANEL,
            panel_names::FILTER_PANEL,
            panel_names::EFFECTS_PANEL,
            panel_names::COLOR_PICKER_PANEL,
            panel_names::KEYBOARD_SHORTCUTS,
        ] {
            let (w, h) = panel_estimated_size(name);
            assert!(w > 0.0 && h > 0.0, "{name}: {w}x{h}");
        }
    }

    /// The Stroke position segments match Swift's Picker tags
    /// (`EffectsSheet.swift:39-41`): Outside first, Inside second, exactly
    /// one selected at a time, and clicking the other segment switches.
    #[test]
    fn stroke_position_segments_match_swift_picker() {
        assert_eq!(STROKE_POSITION_SEGMENTS, ["Outside", "Inside"]);
        for (inside, selected) in [(false, 0), (true, 1)] {
            for segment in 0..STROKE_POSITION_SEGMENTS.len() {
                assert_eq!(
                    stroke_segment_selected(inside, segment),
                    segment == selected,
                    "inside={inside} segment={segment}"
                );
            }
        }
        // The click target mirrors the tag: segment 0 = Outside (false),
        // segment 1 = Inside (true). Clicking the selected segment keeps it.
        assert!(!stroke_segment_target(0));
        assert!(stroke_segment_target(1));
        for segment in 0..STROKE_POSITION_SEGMENTS.len() {
            assert!(
                stroke_segment_selected(stroke_segment_target(segment), segment),
                "target of segment {segment} must select it"
            );
        }
    }

    /// The selection-amount bounds (`SelectionAmountSheet.maximum`).
    #[test]
    fn amount_maximum_follows_the_operation() {
        assert_eq!(amount_maximum(SelectionAmountOperation::Feather), 250);
        assert_eq!(amount_maximum(SelectionAmountOperation::Expand), 500);
        assert_eq!(amount_maximum(SelectionAmountOperation::Contract), 500);
    }

    /// The amount field's parse rule: a whole number in 1…maximum
    /// (`SelectionAmountSheet.amount`).
    #[test]
    fn parse_amount_bounds_and_whole_numbers() {
        assert_eq!(parse_amount("10", 500), Some(10));
        assert_eq!(parse_amount(" 42 ", 500), Some(42));
        assert_eq!(parse_amount("1", 500), Some(1));
        assert_eq!(parse_amount("500", 500), Some(500));
        assert_eq!(parse_amount("250", 250), Some(250));
        // Out of range, zero, negative, fractional, junk.
        assert_eq!(parse_amount("0", 500), None);
        assert_eq!(parse_amount("501", 500), None);
        assert_eq!(parse_amount("-3", 500), None);
        assert_eq!(parse_amount("2.5", 500), None);
        assert_eq!(parse_amount("px", 500), None);
        assert_eq!(parse_amount("", 500), None);
    }

    /// Field ↔ slider sync (`LassoControls.swift:224-228`): the thumb
    /// re-derives from the parsed text (parking at 1 while invalid) and the
    /// text re-derives from the rounded thumb (clamped into range), so a
    /// round trip through both is the identity for valid values.
    #[test]
    fn amount_field_and_slider_stay_in_sync() {
        for maximum in [250, 500] {
            for value in [1, 2, 37, maximum - 1, maximum] {
                let text = value.to_string();
                let thumb = amount_slider_value(&text, maximum);
                assert_eq!(amount_text_for_slider(thumb, maximum), text);
            }
        }
        // Invalid text parks the thumb at 1 (Swift's `amount ?? 1`).
        assert_eq!(amount_slider_value("nonsense", 500), 1.0);
        assert_eq!(amount_slider_value("0", 500), 1.0);
        assert_eq!(amount_slider_value("9999", 500), 1.0);
        // The thumb clamps: dragging past the maximum writes the maximum.
        assert_eq!(amount_text_for_slider(750.0, 500), "500");
        assert_eq!(amount_text_for_slider(0.0, 500), "1");
        // Fractional thumb positions round (`Int($0.rounded())`).
        assert_eq!(amount_text_for_slider(10.4, 500), "10");
        assert_eq!(amount_text_for_slider(10.6, 500), "11");
    }
}
