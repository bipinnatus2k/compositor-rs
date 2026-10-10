//! Port of the Move tool's option bar (`TransformInspector.swift`): the
//! toggles, the labeled numeric fields, flip, the sampling dropdown and the
//! Cancel/Apply pair. The bar is a view entity (ADR 0003 step E, ticket 42)
//! owning the numeric fields; the bar shell itself stays in
//! ui/navigation_tool_header.rs.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    App, Context, Entity, Focusable, FontWeight, IntoElement, ParentElement, Pixels, RenderOnce,
    SharedString, Styled, Subscription, WeakEntity, Window, px,
};

use compositor_core::geometry::CGSize;
use compositor_core::session::EditorSession;
use compositor_core::transform::{LayerSampling, LayerTransform};

use crate::state::{
    ProjectWorkspaceView, TransformField, normalize_rotation, number_input, parse_field,
    resize_transform, transform_field_texts,
};
use crate::ui::{Dropdown, LabeledCheckbox, MenuEntry, app_dispatch, app_on, div_row, text_label};

/// The Move tool's option bar (`TransformInspector.swift`): a view entity
/// since ticket 42 — it owns the six numeric field entities and their write
/// subscriptions (`subscribe_inputs`' transform slice), and carries the
/// displayed-transform read and the draft edit. Only in the render tree
/// while the Move tool is active (Swift's `.id(session.activeLayerID)`
/// aside, which the port does not model), so it keeps no session observer;
/// its session field comes with the per-tab content build (ticket 43).
pub struct TransformInspectorView {
    /// The active tab's session — the toggles, the displayed transform and
    /// the draft edits speak to it directly (`TransformInspector`'
    /// `@Bindable session`; the per-field `@State text`/`stepper`/
    /// `@FocusState`, TransformInspector.swift:82-84, are the input
    /// entities below).
    session: Entity<EditorSession>,
    /// The root view (the window shell): the repaint epoch and the
    /// shared header row stay there. No ProjectWorkspaceView dependencies beyond what
    /// the moved handlers already used.
    app: WeakEntity<ProjectWorkspaceView>,
    /// The numeric fields (X, Y, W, H, Scale %, ° — the TransformValueField
    /// `@State text` stand-ins): seeded flat, exactly as `ProjectWorkspaceView::new`
    /// seeded them when the fields lived there.
    transform_x_input: Entity<InputState>,
    transform_y_input: Entity<InputState>,
    transform_w_input: Entity<InputState>,
    transform_h_input: Entity<InputState>,
    transform_scale_input: Entity<InputState>,
    transform_rotation_input: Entity<InputState>,
    /// Keep-alive for the field subscriptions: dropping a Subscription
    /// unsubscribes, so the bundle is held, never read.
    #[allow(dead_code)]
    subscriptions: Vec<Subscription>,
}

impl TransformInspectorView {
    pub(crate) fn new(
        session: Entity<EditorSession>,
        app: WeakEntity<ProjectWorkspaceView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let transform_x_input = number_input(0.0, window, cx);
        let transform_y_input = number_input(0.0, window, cx);
        let transform_w_input = number_input(0.0, window, cx);
        let transform_h_input = number_input(0.0, window, cx);
        let transform_scale_input = number_input(100.0, window, cx);
        let transform_rotation_input = number_input(0.0, window, cx);
        let mut view = TransformInspectorView {
            session,
            app,
            transform_x_input,
            transform_y_input,
            transform_w_input,
            transform_h_input,
            transform_scale_input,
            transform_rotation_input,
            subscriptions: Vec::new(),
        };
        view.subscribe_inputs(cx);
        view
    }

    /// Wiring for the numeric fields: typing writes straight into the
    /// draft (`subscribe_inputs`' transform slice, moved with the fields —
    /// Swift's live `onChange(of: text)`).
    fn subscribe_inputs(&mut self, cx: &mut Context<Self>) {
        let transform_fields = [
            (self.transform_x_input.clone(), TransformField::X),
            (self.transform_y_input.clone(), TransformField::Y),
            (self.transform_w_input.clone(), TransformField::Width),
            (self.transform_h_input.clone(), TransformField::Height),
            (self.transform_scale_input.clone(), TransformField::Scale),
            (
                self.transform_rotation_input.clone(),
                TransformField::Rotation,
            ),
        ];
        for (input, field) in transform_fields {
            let watched = input.clone();
            let subscription = cx.subscribe(
                &input,
                move |this, _input, event: &InputEvent, cx: &mut Context<Self>| {
                    if let InputEvent::Change = event {
                        let text = watched.read(cx).value().to_string();
                        this.transform_field_changed(field, &text, cx);
                    }
                },
            );
            self.subscriptions.push(subscription);
        }
    }

    /// The view's route to the app-side repaint epoch (`touch`): the frame
    /// cache and `revision` stay on ProjectWorkspaceView (ticket 39's field audit), so
    /// an interaction that changed the session asks the shell through the
    /// seam.
    fn touch(&mut self, cx: &mut Context<Self>) {
        self.app.update(cx, |app, cx| app.touch(cx)).ok();
    }

    /// The transform the fields show: the draft while an edit is open, else
    /// the active layer's (or its unlinked mask's) transform
    /// (`session.editedTransform`).
    fn transform_displayed(&self, cx: &App) -> Option<LayerTransform> {
        let session = self.session.read(cx);
        if let Some(edit) = session.transform_edit.as_ref() {
            return Some(edit.draft);
        }
        let layer = session.active_layer()?;
        let mask_alone = session.is_mask_selected
            && layer
                .mask
                .as_ref()
                .map(|mask| !mask.is_linked)
                .unwrap_or(false);
        Some(if mask_alone {
            layer.mask_transform()
        } else {
            layer.transform
        })
    }

    /// 100% scale: the layer's pixels, else the displayed size
    /// (`transformPixelSize ?? activeLayer?.size ?? value.size`).
    fn transform_pixel_size(&self, cx: &App) -> CGSize {
        if let Some(layer) = self.session.read(cx).active_layer()
            && let Some(asset) = layer.asset.as_ref()
        {
            return CGSize::new(asset.image.width as f64, asset.image.height as f64);
        }
        self.transform_displayed(cx)
            .map(|value| value.size)
            .unwrap_or(CGSize::new(1.0, 1.0))
    }

    /// `TransformInspector.change`: open an edit if none runs, fold the
    /// update into its draft, and show it live.
    fn transform_change(
        &mut self,
        update: impl FnOnce(&mut LayerTransform),
        cx: &mut Context<Self>,
    ) {
        self.session.update(cx, |session, _| {
            if session.transform_edit.is_none() {
                session.begin_transform();
            }
            let Some(edit) = session.transform_edit.as_ref() else {
                return;
            };
            let mut value = edit.draft;
            update(&mut value);
            session.preview_transform(value);
        });
        self.touch(cx);
    }

    /// A typed change in one of the fields (`TransformInspector`'s
    /// `field`/`resize`/scale closures).
    fn transform_field_changed(
        &mut self,
        field: TransformField,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(number) = parse_field(text) else {
            return;
        };
        match field {
            TransformField::X => self.transform_change(|value| value.origin.x = number, cx),
            TransformField::Y => self.transform_change(|value| value.origin.y = number, cx),
            TransformField::Width | TransformField::Height => {
                let lock = self.session.read(cx).locks_transform_ratio;
                let width = field == TransformField::Width;
                self.transform_change(
                    move |value| *value = resize_transform(*value, number, width, lock),
                    cx,
                );
            }
            TransformField::Scale => {
                if number <= 0.0 {
                    return;
                }
                let pixel_size = self.transform_pixel_size(cx);
                self.transform_change(move |value| *value = value.scaled(number, pixel_size), cx);
            }
            TransformField::Rotation => {
                self.transform_change(|value| value.rotation = normalize_rotation(number), cx)
            }
        }
    }

    /// Pushes the displayed transform back into the fields
    /// (`TransformValueField.sync`). Reached from outside this view too —
    /// the canvas's Move drags ask the shell to run it
    /// (`ProjectWorkspaceView::sync_transform_inputs`, ticket 39's seam).
    pub(crate) fn sync_transform_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(value) = self.transform_displayed(cx) else {
            return;
        };
        let pixel_size = self.transform_pixel_size(cx);
        for (field, text) in transform_field_texts(value, pixel_size) {
            let input = match field {
                TransformField::X => &self.transform_x_input,
                TransformField::Y => &self.transform_y_input,
                TransformField::Width => &self.transform_w_input,
                TransformField::Height => &self.transform_h_input,
                TransformField::Scale => &self.transform_scale_input,
                TransformField::Rotation => &self.transform_rotation_input,
            };
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            if !focused && input.read(cx).value() != text {
                input.update(cx, |state, cx| state.set_value(text, window, cx));
            }
        }
    }

    /// Flip H / Flip V (`TransformInspector`'s flip buttons): one transform
    /// change each, then the fields follow.
    fn transform_flip(
        &mut self,
        horizontal: bool,
        window: &mut gpui_kit::Window,
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

    /// `TransformInspector.swift`'s bar controls: toggles, the numeric
    /// fields, flip, sampling, then Cancel/Apply.
    pub(crate) fn transform_inspector(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Vec<gpui_kit::AnyElement> {
        let app = self.app.clone();
        let edit = self.session.read(cx).transform_edit.as_ref();
        let has_edit = edit.is_some();
        let corners = edit.map(|edit| edit.corners.is_some()).unwrap_or(false);
        let mask = edit.map(|edit| edit.mask).unwrap_or_else(|| {
            let session = self.session.read(cx);
            session.is_mask_selected
                && session
                    .active_layer()
                    .and_then(|layer| layer.mask.as_ref().map(|mask| !mask.is_linked))
                    .unwrap_or(false)
        });
        // The fields gray out while nothing can transform, or while a
        // corner distort owns the edit (the handles are the controls).
        let fields_enabled = (self.session.read(cx).can_transform() || has_edit) && !corners;
        // The sampling dropdown: grays out under the same conditions.
        let current = self
            .transform_displayed(cx)
            .map(|value| value.sampling)
            .unwrap_or(LayerSampling::High);
        let sampling_entries: Vec<MenuEntry> = LayerSampling::ALL
            .iter()
            .map(|sampling| {
                let sampling = *sampling;
                MenuEntry::new(sampling.label(), move |state, _, cx| {
                    let inspector = state
                        .content
                        .as_ref()
                        .map(|content| content.read(cx).transform_header.clone());
                    if let Some(inspector) = inspector {
                        inspector.update(cx, |view, cx| {
                            view.transform_change(move |value| value.sampling = sampling, cx)
                        });
                    }
                })
                .checked(current == sampling)
            })
            .collect();

        let controls = vec![
            text_label(if mask { "Transform Mask" } else { "Transform" }, false)
                // Section title: ToolHeaderStyle.swift:6 (13 semibold).
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .flex_shrink_0()
                .into_any_element(),
            LabeledCheckbox::new("transform-auto-select", "Auto-select")
                .checked(self.session.read(cx).transform_auto_select)
                .with_on_change(app_on(&app, |state, checked, _, cx| {
                    // transformAutoSelect (EditorSession.swift:198). The
                    // canvas press reads it when the next Move press lands.
                    state.workspace.update(cx, |ws, cx| {
                        ws.current_mut()
                            .session
                            .update(cx, |session, _| session.transform_auto_select = checked)
                    });
                    state.persist_tool_defaults(cx);
                    state.touch(cx);
                }))
                .into_any_element(),
            LabeledCheckbox::new("transform-show-controls", "Show Controls")
                .checked(self.session.read(cx).shows_transform_controls)
                .with_on_change(app_on(&app, |state, checked, _, cx| {
                    state.workspace.update(cx, |ws, cx| {
                        ws.current_mut()
                            .session
                            .update(cx, |session, _| session.shows_transform_controls = checked)
                    });
                    state.touch(cx);
                }))
                .into_any_element(),
            NumberField::new("transform-x", "X", self.transform_x_input.clone(), px(52.0))
                .into_any_element(),
            NumberField::new("transform-y", "Y", self.transform_y_input.clone(), px(52.0))
                .into_any_element(),
            NumberField::new("transform-w", "W", self.transform_w_input.clone(), px(52.0))
                .into_any_element(),
            NumberField::new("transform-h", "H", self.transform_h_input.clone(), px(52.0))
                .into_any_element(),
            LabeledCheckbox::new("transform-lock-ratio", "Lock")
                .checked(self.session.read(cx).locks_transform_ratio)
                .with_on_change(app_on(&app, |state, checked, _, cx| {
                    state.workspace.update(cx, |ws, cx| {
                        ws.current_mut()
                            .session
                            .update(cx, |session, _| session.locks_transform_ratio = checked)
                    });
                    state.touch(cx);
                }))
                .into_any_element(),
            NumberField::new(
                "transform-scale",
                "Scale %",
                self.transform_scale_input.clone(),
                px(56.0),
            )
            .into_any_element(),
            NumberField::new(
                "transform-rotation",
                "\u{b0}",
                self.transform_rotation_input.clone(),
                px(48.0),
            )
            .into_any_element(),
            Dropdown::new("transform-sampling", current.label())
                .with_width(px(120.0))
                .disabled(!fields_enabled)
                .with_entries(sampling_entries)
                .with_dispatch(app_dispatch(&app))
                .into_any_element(),
            Button::new("transform-flip-h")
                .label("Flip H")
                .small()
                .outline()
                .disabled(!fields_enabled)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.transform_flip(true, window, cx);
                }))
                .into_any_element(),
            Button::new("transform-flip-v")
                .label("Flip V")
                .small()
                .outline()
                .disabled(!fields_enabled)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.transform_flip(false, window, cx);
                }))
                .into_any_element(),
            Button::new("transform-cancel")
                .label("Cancel")
                .small()
                .disabled(!has_edit)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.session
                        .update(cx, |session, _| session.cancel_transform());
                    this.sync_transform_inputs(window, cx);
                    this.touch(cx);
                }))
                .into_any_element(),
            Button::new("transform-apply")
                .label("Apply")
                .small()
                .primary()
                .disabled(!has_edit)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.session
                        .update(cx, |session, _| session.commit_transform());
                    this.sync_transform_inputs(window, cx);
                    this.touch(cx);
                }))
                .into_any_element(),
        ];
        controls
    }
}

/// One of the inspector's labeled numeric fields (TransformValueField's
/// `label + rounded field`). Typing writes through the field's Change
/// subscription; nothing here touches the session.
#[derive(IntoElement)]
pub struct NumberField {
    id: &'static str,
    label: SharedString,
    state: Entity<InputState>,
    width: Pixels,
}

impl NumberField {
    pub fn new(
        id: &'static str,
        label: impl Into<SharedString>,
        state: Entity<InputState>,
        width: Pixels,
    ) -> Self {
        NumberField {
            id,
            label: label.into(),
            state,
            width,
        }
    }
}

impl RenderOnce for NumberField {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div_row()
            .child(text_label(self.label.to_string(), true))
            .child(Input::new(&self.state).id(self.id).small().w(self.width))
    }
}
