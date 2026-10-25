//! The root view `ProjectWorkspaceView` (the former `AppState`, renamed in
//! ticket 43 to follow Swift's view): the window shell around the per-tab
//! `EditorSession` — the frame cache, the modal sheet, the panel host and
//! the preview pipelines. The per-tab content view lives in
//! `content_view.rs` (Swift `ContentView`, ticket 43); the child view
//! entities live with their ports (canvas `canvas::editor_canvas`, layers
//! panel `ui::native_layer_list`, panels `ui::floating_panel`, tool headers
//! / welcome / status bar with their ticket-42 files).

use std::sync::Arc;

use gpui_kit::component::GlobalState;
use gpui_kit::component::WindowExt;
use gpui_kit::component::input::InputState;
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::component::slider::SliderState;
use gpui_kit::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, Menu, Pixels, Point, RenderImage,
    Subscription, Window, px,
};

use image::Frame;

use compositor_core::document::CanvasDocument;
use compositor_core::document::ImageLayer;
use compositor_core::filters::{FilterJob, FilterKind, FilterSettings};
use compositor_core::geometry::{CGPoint, CGSize};
use compositor_core::raster::{Bitmap, ImportedImage};
use compositor_core::render;
use compositor_core::session::EditorSession;
use compositor_core::tools::brush::RasterEdit;
use compositor_core::transform::LayerTransform;

use crate::canvas::editor_canvas::{CanvasDrag, CanvasGesture};
use crate::content_view::ContentView;

pub const RULER_SIZE: f32 = 20.0;

/// Which modal sheet is open. The floating panels that replaced the tool
/// dialogs (`Sheet::Levels`, `HueSaturation`, `Effects`, `Filter`,
/// `Amount`, `Shortcuts` — see `ui/floating_panel.rs`) are gone from here; what remains
/// are the modal dialogs Swift presents as sheets/NSAlerts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sheet {
    ImageSize,
    CanvasSize,
    ExportPng,
    ExportJpeg,
    SaveComp,
    Trim,
    About,
    /// The unsaved-changes alert (`Pending::Confirm`): Save / Cancel /
    /// Don't Save, resolved through the workspace's flow.
    Confirm,
    /// A warning alert (`Pending::Error`): message + OK.
    Alert,
}

/// The modal sheet: which one plus all of its widget states. Fields a given
/// sheet doesn't use are simply never read.
pub struct SheetState {
    pub kind: Sheet,
    pub width: Entity<InputState>,
    pub height: Entity<InputState>,
    pub path: Entity<InputState>,
    pub resolution: Entity<InputState>,
    pub tolerance: Entity<InputState>,
    pub percent: bool,
    pub link: bool,
    pub relative: bool,
    pub anchor: usize,
    pub preset: usize,
    pub sliders: Vec<Entity<SliderState>>,
    pub based_on: usize,
    pub sides: [bool; 4],
    /// The JPEG export's live encoded preview (`JPEGResult.preview`),
    /// fitted into the sheet's frame.
    pub jpeg_preview: Option<Arc<RenderImage>>,
    /// The (quality, matte) the ready preview was encoded from; the OK
    /// button stays disabled while this lags the current options
    /// (`readyOptions != options`).
    pub jpeg_ready: Option<(f64, usize)>,
    /// The encoded bytes the ready preview produced — Export writes them
    /// instead of encoding a second time (`result.data`).
    pub jpeg_data: Option<Vec<u8>>,
    /// The encoded size for the status line (`ByteCountFormatter`).
    pub jpeg_size_label: String,
    /// The Confirm/Alert sheet's message text (`Pending::Confirm.message` /
    /// `Pending::Error.{title,message}`).
    pub message: String,
    /// Which flow the Confirm sheet's answer resumes.
    pub confirm_step: Option<crate::project::ConfirmStep>,
}

/// The Curves filter sheet's editor state (`CurvesControls.swift`'s
/// `settings`/`selected`/`dragging`). It lives outside `SheetState` because
/// the curve canvas needs element bounds; it is created when the Curves
/// sheet body first renders and dropped with the sheet.
pub struct CurvesEditor {
    /// The curve being edited: four channels of control points plus the
    /// visible channel.
    pub settings: compositor_core::adjust::CurvesSettings,
    /// The selected control point (drawn in the accent color, the Remove
    /// point target).
    pub selected: Option<usize>,
    /// The point under an active drag, if any.
    pub dragging: Option<usize>,
}

impl CurvesEditor {
    pub(crate) fn new() -> Self {
        CurvesEditor {
            settings: compositor_core::adjust::CurvesSettings::default(),
            selected: None,
            dragging: None,
        }
    }
}

/// Which Type-header numeric field changed (`TypeControls.swift`'s key
/// paths on `LayerTextStyle`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TextStyleField {
    Size,
    Tracking,
    Leading,
}

/// Which Transform-inspector numeric field changed
/// (`TransformInspector.swift`'s value fields).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TransformField {
    X,
    Y,
    Width,
    Height,
    Scale,
    Rotation,
}

/// `TransformValueField.formatted`: no trailing zeros on a whole number, two
/// decimals otherwise.
pub fn format_number(value: f64) -> String {
    if (value - value.round()).abs() < 0.005 {
        format!("{}", value.round() as i64)
    } else {
        format!("{:.2}", value)
    }
}

/// The Type size field's clamp: 1…2000 px (TypeControls.swift's arrowSteps
/// change, which the typed path clamps to as well).
pub fn clamp_font_size(value: f64) -> f64 {
    value.clamp(1.0, 2000.0)
}

/// The Leading field shows nothing while Auto (leading 0) —
/// TypeControls.swift's `leading > 0 ? String(...) : ""`.
pub fn leading_to_text(leading: f64) -> String {
    if leading > 0.0 {
        format!("{}", leading.round() as i64)
    } else {
        String::new()
    }
}

/// Typing into the Leading field: unparseable or empty means Auto (0); the
/// result is clamped to 0…5000.
pub fn leading_from_text(text: &str) -> f64 {
    let value = text.trim().parse::<f64>().unwrap_or(0.0);
    if value.is_finite() {
        value.clamp(0.0, 5000.0)
    } else {
        0.0
    }
}

/// `TransformInspector.resize`: typing a width or height resizes about the
/// origin box, scaling the other side first when the ratio is locked.
/// Numbers under 1 are refused, as in Swift.
pub fn resize_transform(
    mut value: LayerTransform,
    number: f64,
    width: bool,
    lock: bool,
) -> LayerTransform {
    if number < 1.0 || !number.is_finite() {
        return value;
    }
    if width {
        if lock && value.size.width > 0.0 {
            value.size.height *= number / value.size.width;
        }
        value.size.width = number;
    } else {
        if lock && value.size.height > 0.0 {
            value.size.width *= number / value.size.height;
        }
        value.size.height = number;
    }
    value
}

/// The rotation field wraps through 360 (`$0.rotation =
/// $1.truncatingRemainder(dividingBy: 360)`); Rust's `%` is the same
/// truncating remainder.
pub fn normalize_rotation(degrees: f64) -> f64 {
    degrees % 360.0
}

/// Parses a numeric field's text; non-finite or unparseable input is
/// refused (Swift's `Double(text)` guard).
pub fn parse_field(text: &str) -> Option<f64> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

/// What each inspector field shows for a transform (`TransformValueField`'s
/// `sync`), as pure data: the Move drag writes these into the field
/// entities so they follow the handles live.
pub fn transform_field_texts(
    value: LayerTransform,
    pixel_size: CGSize,
) -> Vec<(TransformField, String)> {
    vec![
        (TransformField::X, format_number(value.origin.x)),
        (TransformField::Y, format_number(value.origin.y)),
        (TransformField::Width, format_number(value.size.width)),
        (TransformField::Height, format_number(value.size.height)),
        (
            TransformField::Scale,
            format_number(value.scale_percent(pixel_size)),
        ),
        (TransformField::Rotation, format_number(value.rotation)),
    ]
}

/// The window's root view (Swift `ProjectWorkspaceView`, ProjectTabs.swift
/// :6-19): it observes the workspace and shows the active tab's content. The
/// document lives in the workspace's tabs
/// (`workspace.read(cx).current().session`): every tab carries its own
/// session entity and controller, exactly like Swift's `ProjectWorkspace`
/// (design doc §1, plan A; ADR 0001; the busy flags moved onto the session
/// in stage-D ticket 34). The workspace itself is a pure state entity too
/// (ADR 0003 step F, ticket 43) — the mirror of Swift's `@Observable
/// ProjectWorkspace`; tab operations go through its `update`/`read`. The
/// type was named `AppState` until this ticket renamed it to follow the
/// Swift view (the state.rs filename is kept — declared in the ticket's
/// landing record).
pub struct ProjectWorkspaceView {
    /// The tab collection (`ProjectWorkspace`): a pure state entity (no
    /// Render) whose struct and methods are unchanged — the entity wrap is
    /// the only difference from the former plain field.
    pub workspace: Entity<crate::tabs::ProjectWorkspace>,
    pub focus: FocusHandle,
    /// Bumped on every session mutation; invalidates the frame image. Zoom
    /// and pan live on the session's `CanvasViewport` (per tab, ticket 38)
    /// and redraw through the paint transform without touching this.
    pub revision: u64,
    frame_image: Option<Arc<RenderImage>>,
    frame_image_rev: u64,
    /// The tab the cached frame was rendered for — the cache key is
    /// `(tab_id, revision)`, so switching tabs never shows a stale frame.
    frame_image_tab: crate::project::TabId,
    /// The active tab's content (Swift `ContentView`, ADR 0003 step F /
    /// ticket 43): a fresh entity per tab, rebuilt by [`Self::sync_content`]
    /// whenever the workspace's selected tab changed — the `.id(workspace
    /// .current.id)` mirror. `None` until the first render pass.
    pub(crate) content: Option<Entity<crate::content_view::ContentView>>,
    pub sheet: Option<SheetState>,
    /// The floating panels (design doc §5): the registry of open panels plus
    /// their remembered positions, and the panels' content view entities
    /// (ticket 41) — each entity exists exactly while its panel is open.
    /// Window-level like the modal sheets (the Swift panels are NSPanels, not
    /// per-tab views), so they stay on the root across tab rebuilds.
    pub panels: crate::ui::floating_panel::FloatingPanels,
    pub panel_views: crate::ui::floating_panel::PanelContents,
    /// The active layer's pixels when an adjustment sheet opened, and the
    /// live preview result shown on canvas.
    pub preview_base: Option<Arc<Bitmap>>,
    /// The Camera Raw defringe eyedropper is armed: the next canvas click
    /// centers a defringe range on the clicked fringe color.
    pub camera_raw_samples_defringe: bool,
    /// RGB under the pointer while Camera Raw is open; cleared outside.
    pub camera_raw_readout: Option<[u8; 3]>,
    pub preview: Option<Arc<Bitmap>>,
    pub preview_base_transform: Option<LayerTransform>,
    /// The filter's grown grid (`FilterEdit.grownImage`/`grownTransform`/
    /// `grownMargin`): the blur's room to spread, grown from the preview base
    /// as the margin rises. `None` for the filters that never grow.
    pub preview_grown: Option<Arc<Bitmap>>,
    pub preview_grown_transform: Option<LayerTransform>,
    pub preview_grown_margin: f64,
    /// The kind the preview pipeline last ran (`FilterEdit.kind`): the commit
    /// reads whether the pixels spread (`gaussianBlur`/`motionBlur`).
    pub preview_kind: Option<FilterKind>,
    pub subscriptions: Vec<Subscription>,
    /// Monotonic epoch for the opacity-digit pair window (0.6 s).
    key_epoch: std::time::Instant,
    /// A save panel raised mid-flow (a confirmation's Save button): the tab
    /// whose controller owns the pending `SaveAs` and what to resume after
    /// `finish_save`.
    pub pending_save: Option<(crate::project::TabId, SaveAfter)>,
    /// The live-mask delete ask (`deleteWithLiveMaskChoice`,
    /// LiveLayerMask.swift:106-125): the layers waiting for the
    /// Bake / Remove Links / Cancel answer, while the three-button Confirm
    /// sheet is up.
    pub delete_ask_ids: Option<Vec<uuid::Uuid>>,
    /// Guards against stacking drain pumps (`DRAIN_PUMP_RUNNING`, moved off
    /// the file-level static in ticket 43 / ADR 0003 ruling 4): set when a
    /// pump is spawned, cleared when it runs out of work (or its window went
    /// away, dropping the flag with this entity). Renders that find work
    /// pending while a pump is already polling are no-ops.
    pub(crate) drain_pump_running: bool,
    /// The gpui-kit `AppMenuBar` the title bar hosts (the in-window menu
    /// bar): its menus are a snapshot of the active tab's session, rebuilt
    /// when [`Self::menu_signature`] drifts.
    pub(crate) app_menu_bar: Entity<AppMenuBar>,
    /// The snapshot the menus were last registered from; `None` until the
    /// first registration.
    menu_signature: Option<crate::menu::MenuSignature>,
}

/// What continues after a mid-flow save panel completes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveAfter {
    /// A plain Save/Save As — nothing follows.
    Nothing,
    /// The close-tab flow's Save button (`workspace.close_after_save`).
    Close,
    /// The quit / window-close flow's Save button (`workspace.quit_after_save`).
    Quit,
}

/// Creates a slider with the given range and initial value.
pub(crate) fn slider(min: f32, max: f32, step: f32, value: f32) -> SliderState {
    SliderState::new()
        .min(min)
        .max(max)
        .step(step)
        .default_value(value)
}

/// Creates a numeric field seeded with `format_number(value)` (`InputState`s
/// need the window, so the tool-header and welcome views build theirs in
/// their constructors — the same seeds `ProjectWorkspaceView::new` used when the fields
/// lived there, ticket 42).
pub(crate) fn number_input<V: 'static>(
    value: f64,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).default_value(format_number(value)))
}

/// A fresh pseudo-random seed for noise patterns (`UInt32.random`, via the
/// same time source the core session uses for its own reseeds).
pub(crate) fn time_seed() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u32)
        .unwrap_or(0)
        ^ 0x9e3779b9
}

/// The top-level ids a delete of the current selection removes
/// (`delete_selected_layers`' id set: the selected layers, in document order).
pub(crate) fn delete_candidate_ids(
    document: &compositor_core::document::CanvasDocument,
    selected: &[uuid::Uuid],
) -> Vec<uuid::Uuid> {
    document
        .layers
        .iter()
        .filter(|l| selected.contains(&l.id))
        .map(|l| l.id)
        .collect()
}

/// `deleteWithLiveMaskChoice`'s targets (LiveLayerMask.swift:110-114): the
/// layers that stay but take their live mask from one of the deleted layers —
/// the ones the Bake / Remove Links question is about.
pub(crate) fn delete_ask_targets(
    document: &compositor_core::document::CanvasDocument,
    ids: &[uuid::Uuid],
) -> Vec<uuid::Uuid> {
    let mut removed = std::collections::HashSet::new();
    for id in ids {
        removed.extend(document.descendant_ids(*id));
        removed.insert(*id);
    }
    document
        .layers
        .iter()
        .filter(|l| {
            !removed.contains(&l.id)
                && l.mask_source_id
                    .map(|s| removed.contains(&s))
                    .unwrap_or(false)
        })
        .map(|l| l.id)
        .collect()
}

impl ProjectWorkspaceView {
    /// Shared view of the current tab, for `&self` readers (menu titles,
    /// scene building, compositing). The workspace is an entity (ticket 43),
    /// so the tab is reachable through its read.
    pub fn current_tab<'a>(&self, cx: &'a App) -> &'a crate::tabs::ProjectTab {
        self.workspace.read(cx).current()
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workspace = cx.new(|cx| crate::tabs::ProjectWorkspace::new(cx));
        // The window opens on one 1280×800 document, as before.
        workspace.update(cx, |ws, cx| {
            ws.current()
                .session
                .update(cx, |session, _| session.create_document(1280, 800, true))
        });
        // Session-side ToolDefaults (Auto Select included) are applied per
        // tab in ProjectTab::new, so the content's header widgets seed from
        // already-persisted settings — every view is built after the
        // defaults were applied and reads its seed itself. The per-tab
        // content itself is built by the first render pass
        // ([`Self::sync_content`]), like Swift's `.id(workspace.current.id)`
        // building `ContentView` for the selected tab.

        let mut state = ProjectWorkspaceView {
            workspace,
            focus: cx.focus_handle(),
            revision: 1,
            frame_image: None,
            frame_image_rev: 0,
            frame_image_tab: crate::project::TabId::nil(),
            content: None,
            sheet: None,
            panels: crate::ui::floating_panel::FloatingPanels::new(),
            panel_views: crate::ui::floating_panel::PanelContents::new(),
            preview_base: None,
            camera_raw_samples_defringe: false,
            camera_raw_readout: None,
            preview: None,
            preview_base_transform: None,
            preview_grown: None,
            preview_grown_transform: None,
            preview_grown_margin: 0.0,
            preview_kind: None,
            subscriptions: Vec::new(),
            key_epoch: std::time::Instant::now(),
            pending_save: None,
            delete_ask_ids: None,
            drain_pump_running: false,
            // The app menu bar starts empty; the first render head registers
            // the menus from the session ([`Self::refresh_menus`]).
            app_menu_bar: AppMenuBar::new(cx),
            menu_signature: None,
        };
        // The root observes the workspace entity (ADR 0003 step F, ticket
        // 43): a workspace notification re-renders the window, mirroring
        // Swift's `ProjectWorkspaceView` observing the `@Observable`
        // workspace. The workspace is a plain entity, so its notifications
        // ride the explicit notify points the flows already make.
        state
            .subscriptions
            .push(cx.observe(&state.workspace, |_, _, cx| cx.notify()));
        // The invalidation counter's root-notify tally (ticket 37): the
        // observer fires once per delivered root invalidation — the count
        // the scenario tests read. Debug-only; release drops it.
        #[cfg(any(test, debug_assertions))]
        state
            .subscriptions
            .push(cx.observe_self(|_, cx| invalidation_counter::root_notify(cx)));
        // `on_window_should_close` → `workspace.close_window` (two-phase):
        // a pending unsaved-changes dialog vetoes the close and presents
        // itself; once the flow completes the window closes for real.
        let view = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            view.update(cx, |state, cx| {
                let step = state.workspace.update(cx, |ws, cx| ws.close_window(cx));
                match step {
                    crate::project::Step::Done(true) => {
                        window.remove_window();
                        true
                    }
                    crate::project::Step::Done(false) => false,
                    crate::project::Step::Pending(pending) => {
                        state.present_pending(pending, window, cx);
                        false
                    }
                }
            })
            .unwrap_or(true)
        });
        state
    }

    /// The `.id(workspace.current.id)` mirror (ProjectTabs.swift:11): when
    /// the workspace's selected tab differs from the tab the content was
    /// built for, the per-tab `ContentView` is rebuilt fresh — its drag,
    /// rename and welcome drafts die with it and the fresh children observe
    /// the new tab's session from their constructors. `can_switch`'s
    /// guards (modal lock, CONTEXT.md) keep this from ever running while an
    /// interaction is in flight. Runs inside the render pass, where the
    /// window is available (the constructors build `InputState`s).
    pub(crate) fn sync_content(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.workspace.read(cx).selected_id();
        if let Some(content) = &self.content
            && content.read(cx).tab_id == selected
        {
            return;
        }
        let session = self.workspace.read(cx).current().session.clone();
        let app = cx.entity().downgrade();
        self.content = Some(cx.new(|cx| ContentView::new(selected, session, app, window, cx)));
    }

    /// Monotonic seconds since the app started (`systemUptime`).
    pub fn uptime(&self) -> f64 {
        self.key_epoch.elapsed().as_secs_f64()
    }

    /// Re-registers the app menus when the active session's menu-affecting
    /// state drifted since the last registration. The `AppMenuBar` reads its
    /// menus from `GlobalState`, so a drift means rebuilding the snapshot in
    /// `menu.rs` and reloading the bar — skipped while unchanged, so
    /// unrelated re-renders never reset an open menu. Runs in the render
    /// head, where `sync_content` keeps the snapshot on the active tab.
    pub(crate) fn refresh_menus(&mut self, cx: &mut Context<Self>) {
        let signature = {
            let session = self.current_tab(cx).session.read(cx);
            crate::menu::MenuSignature::of(session)
        };
        if Some(&signature) == self.menu_signature.as_ref() {
            return;
        }
        self.menu_signature = Some(signature);
        let menus = {
            let session = self.current_tab(cx).session.read(cx);
            crate::menu::build_menus(session)
        };
        GlobalState::global_mut(cx).set_app_menus(menus.into_iter().map(Menu::owned).collect());
        self.app_menu_bar.update(cx, |bar, cx| bar.reload(cx));
    }

    /// Number keys set the active tool's opacity (Photoshop-style).
    pub fn opacity_digit(&mut self, digit: i32, window: &mut Window, cx: &mut Context<Self>) {
        let uptime = self.uptime();
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.type_opacity_digit(digit, uptime))
        });
        // Keep the panel sliders truthful (Swift's bindings follow the same
        // settings).
        use compositor_core::document::NavigationTool;
        let (tool, active_opacity, brush_opacity, gradient_opacity) = {
            let session = self.workspace.read(cx).current().session.read(cx);
            (
                session.tool,
                session.active_layer().map(|l| l.opacity).unwrap_or(1.0),
                session.brush_settings.opacity,
                session.gradient_settings.opacity,
            )
        };
        let sync: Option<(Option<Entity<SliderState>>, f32)> = match tool {
            NavigationTool::Brush
            | NavigationTool::SpotHealing
            | NavigationTool::CloneStamp
            | NavigationTool::Blur => Some((
                self.content
                    .as_ref()
                    .map(|content| content.read(cx).brush_header.read(cx).sliders()[2].clone()),
                (brush_opacity * 100.0) as f32,
            )),
            NavigationTool::Gradient => Some((
                self.content
                    .as_ref()
                    .map(|content| content.read(cx).gradient_header.read(cx).opacity_slider()),
                (gradient_opacity * 100.0) as f32,
            )),
            // The Move tool's digits drive the active layer's opacity — the
            // layers panel header's slider (ticket 40 moved it there).
            NavigationTool::Move => Some((
                self.content
                    .as_ref()
                    .map(|content| content.read(cx).layers_panel.read(cx).opacity_slider()),
                (active_opacity * 100.0) as f32,
            )),
            _ => None,
        };
        if let Some((Some(slider), value)) = sync {
            slider.update(cx, |state, cx| {
                state.set_value(
                    gpui_kit::component::slider::SliderValue::Single(value),
                    window,
                    cx,
                )
            });
        }
        self.touch(cx);
    }

    /// Option-arrows step the edited text's tracking (InlineTextEditor.swift:18-19).
    pub fn adjust_text_tracking(&mut self, delta: f64, cx: &mut Context<Self>) {
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .text_draft
            .is_none()
        {
            return;
        }
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut().session.update(cx, |session, _| {
                session.change_text_style(|style| style.tracking += delta)
            })
        });
        self.touch(cx);
    }

    /// Option-arrows step the edited text's leading, counting from whatever
    /// Auto works out to; up closes the lines up (InlineTextEditor.swift:20-22).
    pub fn adjust_text_leading(&mut self, down: bool, step: f64, cx: &mut Context<Self>) {
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .text_draft
            .is_none()
        {
            return;
        }
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut().session.update(cx, |session, _| {
                session.change_text_style(|style| {
                    style.leading = if down {
                        style.line_height() + step
                    } else {
                        (style.line_height() - step).max(1.0)
                    };
                })
            })
        });
        self.touch(cx);
    }

    /// ⌘Return finishes the text being edited (InlineTextEditor.swift:25-27).
    pub fn finish_text_edit(&mut self, cx: &mut Context<Self>) {
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .text_draft
            .is_some()
        {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.finish_text())
            });
            self.touch(cx);
        }
    }

    /// `[` / `]` shrink and grow the brush, keeping the slider in sync. The
    /// slider entity lives on the brush header (ticket 42) — the one
    /// outside write to it.
    pub fn resize_brush(&mut self, factor: f64, window: &mut Window, cx: &mut Context<Self>) {
        let next = (self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .brush_settings
            .diameter
            * factor)
            .clamp(1.0, 300.0);
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.brush_settings.diameter = next)
        });
        let slider = self
            .content
            .as_ref()
            .map(|content| content.read(cx).brush_header.read(cx).sliders()[0].clone());
        if let Some(slider) = slider {
            slider.update(cx, |state, cx| {
                state.set_value(
                    gpui_kit::component::slider::SliderValue::Single(next as f32),
                    window,
                    cx,
                )
            });
        }
        self.set_status(format!("Brush size {:.0} px", next), cx);
    }

    /// Shift-[ / Shift-]: hardness in Photoshop's 25% steps (0, 25, 50, 75, 100%).
    pub fn change_brush_hardness(
        &mut self,
        increase: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.change_brush_hardness(increase))
        });
        let next = self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .brush_settings
            .hardness;
        let slider = self
            .content
            .as_ref()
            .map(|content| content.read(cx).brush_header.read(cx).sliders()[1].clone());
        if let Some(slider) = slider {
            slider.update(cx, |state, cx| {
                state.set_value(
                    gpui_kit::component::slider::SliderValue::Single((next * 100.0) as f32),
                    window,
                    cx,
                )
            });
        }
        self.set_status(format!("Brush hardness {:.0}%", next * 100.0), cx);
    }

    /// Content-Aware Fill (Shift-Delete): heals the selection from surrounding
    /// pixels and commits it as one undo step, without opening a sheet.
    pub fn run_content_aware_fill(&mut self, cx: &mut Context<Self>) {
        let base = self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .active_layer()
            .and_then(|layer| layer.asset.as_ref().map(|asset| asset.image.clone()));
        let Some(base) = base else {
            return;
        };
        let selection = self.selection_clip(cx);
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .selection()
            .is_none()
        {
            return;
        }
        let job = FilterJob::builder(
            FilterKind::ContentAwareFill,
            &base,
            FilterSettings::default(),
        )
        .with_selection(selection)
        // A one-shot edit takes a fresh seed (Filters.swift:223's
        // `UInt32.random` at FilterEdit creation).
        .with_seed(time_seed())
        .build();
        let result = compositor_core::filters::run_filter(&job);
        let id = self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .active_layer_id;
        if let Some(id) = id {
            self.history_edit(
                "Content-Aware Fill",
                move |session| {
                    if let Some(doc) = session.document.as_mut()
                        && let Some(layer) = doc.layer_mut(id)
                    {
                        layer.asset = Some(ImportedImage::new(result, layer.name.clone()));
                    }
                },
                cx,
            );
        }
        self.touch(cx);
    }

    /// Option-P with the Levels panel open: flips its Preview checkbox. The
    /// checkbox is the panel entity's (ticket 41) — the toggle routes through
    /// the registry, the one outside write to it.
    pub fn toggle_levels_preview(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.levels_view() else {
            return;
        };
        panel.update(cx, |panel, cx| panel.toggle_preview(cx));
    }

    // MARK: Type header (TypeControls.swift)

    /// The Type header's color chip: opens the app color picker on the text
    /// color (`session.openTextColorPicker`). The picker's panel entity is
    /// built here rather than in `color_picker_sheet` because that module
    /// only knows the palette targets (ticket 41: the entity replaces the
    /// former `build_picker_ui` widget half).
    pub fn open_text_color_picker_ui(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .color_picker
            .is_some()
        {
            return;
        }
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.open_text_color_picker())
        });
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .color_picker
            .is_some()
        {
            self.build_picker_ui(window, cx);
        }
        self.touch(cx);
    }

    /// Builds the picker panel for whatever target the session just opened
    /// (`open_color_picker_ui`'s entity half).
    fn build_picker_ui(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((title, color)) = self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .color_picker
            .as_ref()
            .map(|picker| (picker.target.title(), picker.color()))
        else {
            return;
        };
        let session = self.workspace.read(cx).current().session.clone();
        let app = cx.entity().downgrade();
        let panel = cx.new(|cx| {
            crate::ui::color_picker_sheet::ColorPickerPanelView::new(
                session, app, color, window, cx,
            )
        });
        // The text color's picker lives in the same floating panel as the
        // palette one (ColorPickerSheet.swift:170-182).
        self.panel_views.insert(
            crate::ui::floating_panel::panel_names::COLOR_PICKER_PANEL,
            crate::ui::floating_panel::PanelContent::ColorPicker(panel),
        );
        self.panels.show(
            crate::ui::floating_panel::panel_names::COLOR_PICKER_PANEL,
            title,
            crate::ui::floating_panel::PanelPlacement::Automatic,
        );
        self.sync_panel_guard(cx);
    }

    // MARK: Transform inspector (TransformInspector.swift)

    /// Pushes the displayed transform back into the inspector's fields
    /// (`TransformValueField.sync`): the fields and the sync live on the
    /// inspector entity since ticket 42; the canvas's Move drags ask the
    /// shell after a drag ends (ticket 39's seam), so the shell forwards.
    pub fn sync_transform_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let inspector = self
            .content
            .as_ref()
            .map(|content| content.read(cx).transform_header.clone());
        if let Some(inspector) = inspector {
            inspector.update(cx, |inspector, cx| {
                inspector.sync_transform_inputs(window, cx)
            });
        }
    }

    /// Every mutation funnels through here so the frame image rebuilds.
    pub fn touch(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        cx.notify();
    }

    /// The touch-without-notify variant: writes the repaint epoch when the
    /// caller notifies separately (`apply_filter_preview`'s panel handlers
    /// run `touch` right after, and the defringe sample paths notify
    /// upstream) — the notify timing stays exactly as before the ticket-37
    /// unification folded the inline `revision += 1` sites into here.
    pub(crate) fn bump_revision(&mut self) {
        self.revision += 1;
    }

    pub fn notify_only(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    /// The composite for canvas display: mask resamples cap at 2048 pixels
    /// while a transform edit drives the placements (`clipImage`'s
    /// interactive `limit`).
    pub fn composite_bitmap(&self, document: &CanvasDocument, cx: &App) -> Bitmap {
        if self
            .current_tab(cx)
            .session
            .read(cx)
            .transform_edit
            .is_some()
        {
            compositor_core::render::draw_live_composite_interactive(document, true)
        } else {
            compositor_core::render::draw_live_composite(document)
        }
    }

    /// Re-encodes the JPEG export sheet's live preview when the options
    /// changed (`JPEGExportSheet.task(id: options)`): the encoded bytes are
    /// kept so Export writes the preview's own data, and OK stays disabled
    /// until the preview matches (`readyOptions != options`).
    pub fn refresh_jpeg_preview(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.sheet.as_ref() else {
            return;
        };
        if sheet.kind != Sheet::ExportJpeg {
            return;
        }
        let quality = self.sheet_value(cx, 0) as f64;
        let matte = sheet.preset;
        if sheet.jpeg_ready == Some((quality, matte)) {
            return;
        }
        let Some(document) = self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .document
            .clone()
        else {
            return;
        };
        let composite = compositor_core::render::draw_live_composite(&document);
        if compositor_core::export::check_export_size(composite.width, composite.height).is_err() {
            return;
        }
        let options = compositor_core::export::JpegOptions {
            quality,
            red: if matte == 1 { 0.0 } else { 1.0 },
            green: if matte == 1 { 0.0 } else { 1.0 },
            blue: if matte == 1 { 0.0 } else { 1.0 },
        };
        let Some(data) =
            compositor_core::export::jpeg_data_with_dpi(&composite, &options, document.resolution)
        else {
            return;
        };
        let preview = compositor_core::export::preview_thumbnail(&composite);
        let (size_label, preview) = {
            let image = bgra_render_image(&preview);
            (byte_count_label(data.len()), image)
        };
        if let Some(sheet) = self.sheet.as_mut() {
            sheet.jpeg_data = Some(data);
            sheet.jpeg_preview = Some(preview);
            sheet.jpeg_size_label = size_label;
            sheet.jpeg_ready = Some((quality, matte));
        }
        cx.notify();
    }

    // MARK: View transform

    pub fn doc_size(&self, cx: &App) -> (f64, f64) {
        self.current_tab(cx)
            .session
            .read(cx)
            .document
            .as_ref()
            .map(|d| (d.width as f64, d.height as f64))
            .unwrap_or((0.0, 0.0))
    }

    /// The active tab's view transform (`CanvasViewport` is `Copy`); the
    /// mapping methods below mirror Swift's direct `session.viewport` reads.
    pub fn canvas_viewport(&self, cx: &App) -> compositor_core::canvas_viewport::CanvasViewport {
        self.current_tab(cx).session.read(cx).viewport
    }

    pub fn doc_to_screen(&self, cx: &App, x: f64, y: f64) -> (f32, f32) {
        let viewport = self.canvas_viewport(cx);
        let (w, h) = self.doc_size(cx);
        let point = viewport.view_point(CGPoint::new(x, y), CGSize::new(w, h));
        (point.x as f32, point.y as f32)
    }

    pub fn screen_to_doc(&self, cx: &App, x: f32, y: f32) -> CGPoint {
        let viewport = self.canvas_viewport(cx);
        let (w, h) = self.doc_size(cx);
        viewport.document_point(CGPoint::new(x as f64, y as f64), CGSize::new(w, h))
    }

    /// The document rect in canvas pixels.
    pub fn image_rect(&self, cx: &App) -> Bounds<Pixels> {
        let viewport = self.canvas_viewport(cx);
        let (w, h) = self.doc_size(cx);
        let per_pixel = viewport.points_per_pixel();
        let origin = viewport.view_point(CGPoint::ZERO, CGSize::new(w, h));
        Bounds::<Pixels>::new(
            Point::new(px(origin.x as f32), px(origin.y as f32)),
            gpui_kit::size(px((w * per_pixel) as f32), px((h * per_pixel) as f32)),
        )
    }

    /// Zoom by a factor with the document point under `screen` pinned
    /// (`session.zoom(to:anchor:)`, EditorSession.swift:913-916) moved with
    /// the canvas gestures (ticket 39) — only the canvas called it.
    /// `session.fit()` — View > Fit Canvas (EditorSession.swift:908-911).
    pub fn zoom_fit(&mut self, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.fit())
        });
    }

    // MARK: Frame image

    fn set_preview_layer(layer: &mut ImageLayer, pixels: &Bitmap, transform: LayerTransform) {
        layer.asset = Some(ImportedImage::with_thumbnail(
            pixels.clone(),
            compositor_core::raster::thumbnail_of(pixels),
            layer.name.clone(),
        ));
        layer.transform = transform;
    }

    /// The composited canvas, with any in-progress stroke or adjustment
    /// preview standing in for the active layer's pixels. The gesture state
    /// it previews (stroke/warp/shape/gradient) lives on the canvas view
    /// since ticket 39, so the caller snapshots it into `gesture`; the
    /// preview branches and the branch order are unchanged.
    pub(crate) fn composite(&self, gesture: CanvasGesture, cx: &App) -> Option<Bitmap> {
        invalidation_counter::composite(cx);
        let session = self.current_tab(cx).session.read(cx);
        let document = session.document.as_ref()?;
        // While the inline text editor is open, its draft's layer is held
        // out of the composite (`EditorCanvas`'s draw loop skips
        // `layer.id == session.textDraft?.layerID`): the overlay input paints
        // the live text, and the layer's stale raster would double it from
        // underneath. The layer returns as soon as the draft is committed or
        // cancelled — no draft, no skip — and every other layer, plus the
        // preview branches below, compose as usual against the rest.
        let held = session
            .text_draft
            .as_ref()
            .and_then(|draft| draft.layer_id)
            .map(|id| {
                let mut doc = document.clone();
                doc.layers.retain(|layer| layer.id != id);
                doc
            });
        let document = held.as_ref().unwrap_or(document);
        let active_id = session.active_layer_id;
        let override_pixels =
            |doc: &mut CanvasDocument, pixels: &Bitmap, transform: LayerTransform| {
                if let Some(id) = active_id
                    && let Some(layer) = doc.layer_mut(id)
                {
                    Self::set_preview_layer(layer, pixels, transform);
                }
            };

        // Preview pixels override the active layer while a sheet is open.
        // A blur's grown preview places by its grown grid's transform; every
        // other preview places by the layer's own.
        let preview_placement = self.preview_grown_transform.or(self.preview_base_transform);
        if let (Some(preview), Some(base_transform)) = (&self.preview, preview_placement) {
            let mut doc = document.clone();
            override_pixels(&mut doc, preview, base_transform);
            return Some(self.composite_bitmap(&doc, cx));
        }
        // A pixel move shows the lifted pixels rebuilt at the offset, placed
        // by the layer's own transform (the rebuilt grid is the layer's).
        if let Some(move_state) = &session.pixel_move
            && let Some(pixels) = move_state.moved_pixels()
            && let Some(layer) = session.active_layer()
        {
            let mut doc = document.clone();
            override_pixels(&mut doc, &pixels, layer.transform);
            return Some(self.composite_bitmap(&doc, cx));
        }
        // A stroke in progress overrides the active layer too.
        if let Some(CanvasDrag::Stroke(edit, _)) = gesture.drag {
            let result = edit.result();
            let mut doc = document.clone();
            override_pixels(&mut doc, &result, edit.paint_transform);
            return Some(self.composite_bitmap(&doc, cx));
        }
        // A Smudge / Liquify stroke shows its document-size working copy in
        // place of the layer, exactly as the Swift canvas does.
        if let Some(CanvasDrag::Warp(stroke)) = gesture.drag {
            let mut doc = document.clone();
            override_pixels(
                &mut doc,
                stroke.image(),
                compositor_core::transform::LayerTransform::new(
                    compositor_core::geometry::CGPoint::ZERO,
                    document.size(),
                ),
            );
            return Some(self.composite_bitmap(&doc, cx));
        }
        // The Shape tool previews its layer live while dragging.
        if let Some(CanvasDrag::Shape { .. }) = gesture.drag
            && let (Some(shape), Some(rect)) = (gesture.shape_shape, gesture.shape_rect)
        {
            let mut doc = document.clone();
            let raster = render::render_shape_image(
                shape,
                rect.width().round() as usize,
                rect.height().round() as usize,
            );
            let mut layer = ImageLayer::with_asset(
                compositor_core::raster::ImportedImage::new(raster, String::new()),
                rect.origin,
            );
            layer.transform =
                compositor_core::transform::LayerTransform::new(rect.origin, rect.size);
            doc.layers.push(layer);
            return Some(self.composite_bitmap(&doc, cx));
        }
        // A gradient drag previews the fill.
        if let Some(CanvasDrag::Gradient(edit, start, _)) = gesture.drag {
            let result = self.gradient_result(edit, *start, gesture.gradient_cursor, cx);
            let mut doc = document.clone();
            override_pixels(&mut doc, &result, edit.paint_transform);
            return Some(self.composite_bitmap(&doc, cx));
        }
        Some(render::draw_live_composite(document))
    }

    pub(crate) fn gradient_result(
        &self,
        edit: &RasterEdit,
        start: CGPoint,
        cursor: Option<CGPoint>,
        cx: &App,
    ) -> Bitmap {
        let session = self.current_tab(cx).session.read(cx);
        let settings = session.gradient_settings;
        let fg = straight_of(session.palette_color(false));
        let bg = straight_of(session.palette_color(true));
        // "Foreground to Transparent" runs fg at full alpha down to fg at zero
        // alpha (Gradient.swift:78-80) — repeating the fg color fills solid.
        let mut colors = if settings.foreground_to_background {
            [fg, bg]
        } else {
            let mut transparent = fg;
            transparent[3] = 0;
            [fg, transparent]
        };
        if settings.reversed {
            colors.reverse();
        }
        let end = cursor.unwrap_or(start);
        edit.fill_gradient(settings.shape, start, end, colors, settings.opacity)
    }

    /// The cached BGRA texture for the composite, rebuilt when the tab or the
    /// revision changes (cache key `(tab_id, revision)` — design doc §1).
    pub fn frame_image(&mut self, composite: &Bitmap, cx: &App) -> Arc<RenderImage> {
        let tab_id = self.workspace.read(cx).selected_id();
        if self.frame_image.is_none()
            || self.frame_image_rev != self.revision
            || self.frame_image_tab != tab_id
        {
            invalidation_counter::frame_rebuild(cx);
            self.frame_image = Some(bgra_render_image(composite));
            self.frame_image_rev = self.revision;
            self.frame_image_tab = tab_id;
        }
        self.frame_image.clone().unwrap()
    }

    // MARK: Status

    /// The status line's write path: the text lives on the status bar view
    /// entity since ticket 42 (Swift `ContentView.statusBar`'s local
    /// message); the root forwards and keeps the repaint epoch (`touch`)
    /// exactly as when the field lived here.
    pub fn set_status(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        let message = message.into();
        if let Some(content) = self.content.as_ref() {
            let bar = content.read(cx).status_bar.clone();
            bar.update(cx, |bar, _| bar.set_status(message));
        }
        self.touch(cx);
    }

    // MARK: Sheets

    /// Wraps a document mutation in one undo step named `name`
    /// (`session.history.begin/end`; one entity update so the begin, the
    /// edit and the end run on the same session borrow).
    pub(crate) fn history_edit(
        &mut self,
        name: &str,
        edit: impl FnOnce(&mut EditorSession),
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut().session.update(cx, |session, _| {
                session
                    .history
                    .begin(name, &session.document, session.active_layer_id);
                edit(session);
                session
                    .history
                    .end(&session.document, session.active_layer_id);
            })
        });
    }

    pub fn close_sheet(&mut self, cx: &mut Context<Self>) {
        self.sheet = None;
        self.preview = None;
        self.preview_base = None;
        self.preview_base_transform = None;
        self.preview_grown = None;
        self.preview_grown_transform = None;
        self.preview_grown_margin = 0.0;
        self.preview_kind = None;
        // The gradient cursor is canvas gesture state (ticket 39); closing
        // the sheet clears it exactly as it did when the field lived here.
        if let Some(content) = self.content.as_ref() {
            let canvas = content.read(cx).canvas_view.clone();
            canvas.update(cx, |canvas, _| canvas.gradient_cursor = None);
        }
        // A filter panel draws its noise seed fresh at open (each
        // FilterPanelView seeds itself), so nothing to reseed here.
        self.delete_ask_ids = None;
        self.touch(cx);
    }

    /// Commits the preview into the active layer's pixels as one undo step,
    /// then drops the preview (the canvas shows the layer's own pixels — now
    /// the committed ones). The sheet/panel caller closes its own UI after.
    ///
    /// The blur commit follows `commitFilter` (Filters.swift): the pixels
    /// spread on the grown grid, `trimmed` cuts whatever stayed empty away —
    /// the layer keeps the spread — and an owned mask is carried onto the new
    /// grid. A layer changed since the edit opened drops the commit silently.
    pub fn commit_preview_pixels(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Some(preview) = self.preview.clone() {
            let base = self.preview_base.clone();
            let base_transform = self.preview_base_transform.unwrap_or_default();
            let grown_transform = self.preview_grown_transform;
            let spread = matches!(
                self.preview_kind,
                Some(FilterKind::GaussianBlur | FilterKind::MotionBlur)
            );
            // `current.asset?.image === edit.original.image, current.transform
            // == edit.transform` — otherwise the commit drops (the panel still
            // closes, as Swift's defer runs either way).
            let unchanged = base.as_ref().is_some_and(|base| {
                self.active_layer_read(cx).is_some_and(|layer| {
                    layer
                        .asset
                        .as_ref()
                        .is_some_and(|asset| Arc::ptr_eq(&asset.image, base))
                        && layer.transform == base_transform
                })
            });
            let mut commit: Option<(
                Bitmap,
                LayerTransform,
                Option<compositor_core::document::LayerMask>,
            )> = None;
            if unchanged {
                let (pixels, transform) = if spread && let Some(grown) = grown_transform {
                    let (image, placed) = compositor_core::filters::trimmed(&preview, grown);
                    (image, placed)
                } else {
                    (preview.as_ref().clone(), base_transform)
                };
                let mask = if spread && let Some(grown) = grown_transform {
                    self.carried_mask(grown, &pixels, base_transform, cx)
                } else {
                    Ok(None)
                };
                match mask {
                    Ok(mask) => commit = Some((pixels, transform, mask)),
                    // The carry threw in Swift: brushError (ExportError
                    // .render's description), no undo step.
                    Err(()) => {
                        self.workspace.update(cx, |ws, cx| {
                            ws.current_mut().session.update(cx, |session, _| {
                                session.brush_error = Some(
                                    "The canvas could not be rendered. Try a smaller canvas."
                                        .into(),
                                );
                            })
                        });
                    }
                }
            }
            if let Some((pixels, transform, mask)) = commit {
                self.history_edit(
                    name,
                    move |session| {
                        if let Some(id) = session.active_layer_id
                            && let Some(doc) = session.document.as_mut()
                            && let Some(layer) = doc.layer_mut(id)
                        {
                            layer.asset = Some(ImportedImage::new(pixels, layer.name.clone()));
                            layer.transform = transform;
                            if let Some(mask) = mask {
                                layer.mask = Some(mask);
                            }
                        }
                    },
                    cx,
                );
            }
        }
        self.preview = None;
        self.preview_base = None;
        self.preview_base_transform = None;
        self.preview_grown = None;
        self.preview_grown_transform = None;
        self.preview_grown_margin = 0.0;
        self.preview_kind = None;
        self.touch(cx);
    }

    /// The active layer, read straight off the workspace (the commit's
    /// unchanged-guard and the mask carry read it outside the history edit).
    fn active_layer_read(&self, cx: &App) -> Option<compositor_core::document::ImageLayer> {
        let session = self.current_tab(cx).session.read(cx);
        session.active_layer().cloned()
    }

    /// `commitFilter`'s mask carry: an owned mask (covering the old grid,
    /// past a 1×1 solid) is resampled onto the grown grid — `enabled
    /// .clipImage(placement: current.transform, over: grown, width:…
    /// height:…)`, its edge tone past the old edge — and replaces the mask's
    /// asset. A carry failure throws in Swift, dropping the whole commit; the
    /// caller surfaces it (`Err`) as brushError.
    fn carried_mask(
        &self,
        grown: LayerTransform,
        pixels: &Bitmap,
        base_transform: LayerTransform,
        cx: &App,
    ) -> Result<Option<compositor_core::document::LayerMask>, ()> {
        let Some(owned) = self.active_layer_read(cx).and_then(|layer| layer.mask) else {
            return Ok(None);
        };
        if owned.placement.is_some()
            || (owned.asset.image.width <= 1 && owned.asset.image.height <= 1)
        {
            return Ok(None);
        }
        let mut enabled = owned.clone();
        enabled.is_enabled = true;
        enabled.placement = Some(base_transform);
        let carried =
            compositor_core::render::mask_clip_image(&enabled, &grown, pixels.width, pixels.height)
                .ok_or(())?;
        Ok(Some(owned.replacing(ImportedImage::new(
            carried,
            owned.asset.name.clone(),
        ))))
    }

    /// The active selection clipped to the canvas (`selection?.clip(canvas:)`).
    pub(crate) fn selection_clip(
        &self,
        cx: &App,
    ) -> Option<compositor_core::selection::SelectionClip> {
        let session = self.current_tab(cx).session.read(cx);
        session.selection().and_then(|selection| {
            session
                .document
                .as_ref()
                .map(|document| selection.clip(document.size()))
        })
    }

    /// Whether deleting the current selection must ask first: the deleted
    /// layers supply live masks to layers that stay (`deleteWithLiveMaskChoice`'s
    /// guard, LiveLayerMask.swift:110-114). Plain deletes answer false and go
    /// through `delete_selected_layers` as before.
    pub fn delete_needs_ask(&self, cx: &App) -> bool {
        let session = self.current_tab(cx).session.read(cx);
        if !session.can_edit_layers() {
            return false;
        }
        let Some(document) = session.document.as_ref() else {
            return false;
        };
        let mut ids = delete_candidate_ids(document, &session.selected_layer_ids);
        if ids.len() <= 1 {
            // delete_selected_layers defers to delete_active_layer for a
            // single selection.
            ids = session.active_layer_id.into_iter().collect();
        }
        !delete_ask_targets(document, &ids).is_empty()
    }

    /// The layers a delete of the selection removes — the id set
    /// `delete_selected_layers` computes (the selected layers, top level;
    /// folders take their contents through `descendant_ids`).
    pub(crate) fn delete_candidates(&self, cx: &App) -> Vec<uuid::Uuid> {
        let session = self.current_tab(cx).session.read(cx);
        match session.document.as_ref() {
            Some(document) => delete_candidate_ids(document, &session.selected_layer_ids),
            None => Vec::new(),
        }
    }

    /// The open filter panel's current settings (`FilterSheet`'s `FilterEdit`
    /// reads): `None` when no filter panel is open.
    pub(crate) fn filter_settings_from_panel(&self, cx: &App) -> Option<FilterSettings> {
        match self
            .panel_views
            .get(crate::ui::floating_panel::panel_names::FILTER_PANEL)
        {
            Some(crate::ui::floating_panel::PanelContent::Filter(view)) => {
                Some(view.read(cx).filter_settings(cx))
            }
            _ => None,
        }
    }

    /// The open filter panel's noise seed (`FilterEdit.seed`, drawn at
    /// panel open); a fresh seed when no panel is open. **Only call from a
    /// context where the panel entity is not being updated** — the shell's
    /// own flows (the canvas defringe sample, Auto white balance). The
    /// panel's own sliders and curve drags pass `FilterPanelView::seed`
    /// through the seam instead (reading the panel from inside its update
    /// double-leases it).
    pub(crate) fn filter_seed_from_panel(&self, cx: &App) -> u32 {
        match self
            .panel_views
            .get(crate::ui::floating_panel::panel_names::FILTER_PANEL)
        {
            Some(crate::ui::floating_panel::PanelContent::Filter(view)) => view.read(cx).seed(),
            _ => time_seed(),
        }
    }

    /// Runs `filter` over the preview base and shows it live on canvas.
    /// `seed` is the panel's open-time noise seed (`FilterEdit.seed`,
    /// Filters.swift:223) — passed in, never read back from the panel here:
    /// this pipeline runs inside the filter panel's own entity update when
    /// its sliders and curve drags call it, and reading the panel entity
    /// then would double-lease it (the panel hands its values over; the
    /// shell reads the panel only from root-originated contexts).
    pub fn apply_filter_preview(
        &mut self,
        kind: FilterKind,
        settings: FilterSettings,
        seed: u32,
        cx: &mut Context<Self>,
    ) {
        // Editing an adjustment layer: the settings go to the session's
        // filter editor and the layer previews through the live composite.
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
                ws.current_mut().session.update(cx, |session, _| {
                    session.filter_edit = Some(settings);
                    session.preview_adjustment_editing(true);
                })
            });
            return;
        }
        let Some(base) = self.preview_base.clone() else {
            return;
        };
        let Some(base_transform) = self.preview_base_transform else {
            return;
        };
        self.preview_kind = Some(kind);
        // A bigger blur needs more room around the layer than it was given
        // (Filters.swift updateFilter → growForBlur): the grid only ever
        // grows, regrowing from the original pixels when the margin rises.
        match compositor_core::filters::grow_for_blur(
            &base,
            &base_transform,
            kind,
            &settings.normalized(),
            self.preview_grown_margin,
        ) {
            Ok(Some(grown)) => {
                self.preview_grown = Some(Arc::new(grown.image));
                self.preview_grown_transform = Some(grown.transform);
                self.preview_grown_margin = grown.margin;
            }
            Ok(None) => {}
            Err(_) => {
                // grow(to:)'s 30k/100MP guards throw ProjectError.tooLarge,
                // surfaced as brushError like every begin/updateFilter catch.
                self.workspace.update(cx, |ws, cx| {
                    ws.current_mut().session.update(cx, |session, _| {
                        session.brush_error = Some(
                            "This project exceeds the supported canvas, layer, file-size, or 100-megapixel image limit."
                                .into(),
                        );
                    })
                });
                return;
            }
        }
        let selection = self.selection_clip(cx);
        // The filter runs on the grown grid when one is up
        // (`edit.grownImage ?? edit.original.image`).
        let grown = self.preview_grown.clone().unwrap_or_else(|| base.clone());
        let job = FilterJob::builder(kind, &grown, settings)
            .with_selection(selection)
            .with_seed(seed)
            .build();
        let result = compositor_core::filters::run_filter(&job);
        self.preview = Some(Arc::new(result));
        self.bump_revision();
    }

    /// `sampleCameraRawDefringe` — centers the purple or green defringe range
    /// on the clicked fringe color and refreshes the preview.
    pub fn sample_camera_raw_defringe(&mut self, at: CGPoint, cx: &mut Context<Self>) {
        let Some(base) = self.preview_base.clone() else {
            return;
        };
        let px = at.x.floor();
        let py = at.y.floor();
        if px < 0.0 || py < 0.0 || px >= base.width as f64 || py >= base.height as f64 {
            return;
        }
        let Some(hue) = compositor_core::adjust::camera_raw::straight_pixel_hue(
            base.pixel(px as usize, py as usize),
        ) else {
            return;
        };
        let Some(mut settings) = self.filter_settings_from_panel(cx) else {
            return;
        };
        compositor_core::adjust::camera_raw::apply_defringe_sample(
            &mut settings.camera_raw.optics,
            hue,
        );
        self.camera_raw_samples_defringe = false;
        // Root context (the canvas click): reading the panel here is legal.
        let seed = self.filter_seed_from_panel(cx);
        self.apply_filter_preview(FilterKind::CameraRaw, settings, seed, cx);
    }

    /// `updateCameraRawReadout` — the RGB under the pointer; outside the
    /// layer the readout clears.
    pub fn update_camera_raw_readout(&mut self, at: CGPoint) {
        let Some(base) = self.preview_base.clone() else {
            self.camera_raw_readout = None;
            return;
        };
        let px = at.x.floor();
        let py = at.y.floor();
        if px < 0.0 || py < 0.0 || px >= base.width as f64 || py >= base.height as f64 {
            self.camera_raw_readout = None;
            return;
        }
        let pixel = base.pixel(px as usize, py as usize);
        let alpha = pixel[3] as f64;
        if alpha <= 0.0 {
            self.camera_raw_readout = None;
            return;
        }
        self.camera_raw_readout = Some([
            ((pixel[0] as f64 / alpha * 255.0).round() as u8),
            ((pixel[1] as f64 / alpha * 255.0).round() as u8),
            ((pixel[2] as f64 / alpha * 255.0).round() as u8),
        ]);
    }

    /// `applyCameraRawAutoWhiteBalance` — selects Auto and solves the
    /// temperature/tint from the preview base.
    pub fn apply_camera_raw_auto_white_balance(&mut self, cx: &mut Context<Self>) {
        use compositor_core::adjust::camera_raw::CameraRawWhiteBalance;
        let Some(base) = self.preview_base.clone() else {
            return;
        };
        let Some(mut settings) = self.filter_settings_from_panel(cx) else {
            return;
        };
        settings.camera_raw.white_balance = CameraRawWhiteBalance::Auto;
        if let Some((temperature, tint)) =
            compositor_core::adjust::camera_raw::CameraRawSettings::auto_balance(&base)
        {
            settings.camera_raw.temperature = temperature;
            settings.camera_raw.tint = tint;
        }
        let seed = self.filter_seed_from_panel(cx);
        self.apply_filter_preview(FilterKind::CameraRaw, settings, seed, cx);
    }

    // MARK: Keys

    /// Single-letter shortcuts and nudges are ignored while a text input or
    /// an open sheet owns the keyboard.
    pub fn canvas_keys_active(&self, window: &mut Window, cx: &mut App) -> bool {
        if self.sheet.is_some() {
            return false;
        }
        !window.has_focused_input(cx)
    }
}

/// Straight [r, g, b, a] bytes for a palette color (fully opaque).
fn straight_of(color: compositor_core::document::PaletteColor) -> [u8; 4] {
    [
        (color.red * 255.0).round() as u8,
        (color.green * 255.0).round() as u8,
        (color.blue * 255.0).round() as u8,
        255,
    ]
}

/// The invalidation counter (ticket 37, ADR 0003 step G): a lightweight tally
/// of the invalidation pipeline's recurring costs — delivered root
/// invalidations, canvas composites, frame-image rebuilds and thumbnail
/// regenerations. The scenario tests read it as the measure-first harness
/// (baseline before the content key, after-numbers after); a debug overlay
/// may read the same numbers. Release builds compile the empty stub so the
/// call sites stay unconditional and the optimizer drops them.
#[cfg(any(test, debug_assertions))]
pub(crate) mod invalidation_counter {
    use std::cell::Cell;

    use gpui_kit::{App, Global};

    /// The tallies as a gpui Global: every increment site runs against
    /// `&App`, so the fields are `Cell`s. The global is absent outside the
    /// scenario tests (`try_global`), so debug runs pay nothing.
    #[derive(Default)]
    pub(crate) struct InvalidationCounters {
        /// Delivered root invalidations: one per update cycle that notifies
        /// [`ProjectWorkspaceView`] (gpui coalesces same-cycle notifies).
        pub(crate) root_notify: Cell<u64>,
        /// Canvas composites ([`ProjectWorkspaceView::composite`], one per
        /// canvas render).
        pub(crate) composite: Cell<u64>,
        /// Frame-image rebuilds (the `(tab_id, revision)` cache missing).
        pub(crate) frame_rebuild: Cell<u64>,
        /// Layer-row thumbnail regenerations (`LayersPanelView`).
        pub(crate) thumbnail_regen: Cell<u64>,
        /// Mask-thumbnail regenerations (`LayersPanelView`).
        pub(crate) mask_thumbnail_regen: Cell<u64>,
    }

    impl Global for InvalidationCounters {}

    impl InvalidationCounters {
        /// A `(root_notify, composite, frame_rebuild, thumbnail_regen,
        /// mask_thumbnail_regen)` snapshot.
        #[cfg(test)]
        pub(crate) fn snapshot(&self) -> [u64; 5] {
            [
                self.root_notify.get(),
                self.composite.get(),
                self.frame_rebuild.get(),
                self.thumbnail_regen.get(),
                self.mask_thumbnail_regen.get(),
            ]
        }

        #[cfg(test)]
        fn reset(&mut self) {
            self.root_notify.set(0);
            self.composite.set(0);
            self.frame_rebuild.set(0);
            self.thumbnail_regen.set(0);
            self.mask_thumbnail_regen.set(0);
        }
    }

    /// Installs a fresh counter (the scenario tests' first call); absent
    /// counters make every increment a no-op, so debug runs pay nothing.
    #[cfg(test)]
    pub(crate) fn install(cx: &mut App) {
        cx.set_global(InvalidationCounters::default());
    }

    #[cfg(test)]
    pub(crate) fn reset(cx: &mut App) {
        cx.global_mut::<InvalidationCounters>().reset();
    }

    #[cfg(test)]
    pub(crate) fn snapshot(cx: &App) -> [u64; 5] {
        cx.global::<InvalidationCounters>().snapshot()
    }

    pub(crate) fn root_notify(cx: &App) {
        if let Some(counters) = cx.try_global::<InvalidationCounters>() {
            counters.root_notify.set(counters.root_notify.get() + 1);
        }
    }

    pub(crate) fn composite(cx: &App) {
        if let Some(counters) = cx.try_global::<InvalidationCounters>() {
            counters.composite.set(counters.composite.get() + 1);
        }
    }

    pub(crate) fn frame_rebuild(cx: &App) {
        if let Some(counters) = cx.try_global::<InvalidationCounters>() {
            counters.frame_rebuild.set(counters.frame_rebuild.get() + 1);
        }
    }

    pub(crate) fn thumbnail_regen(cx: &App) {
        if let Some(counters) = cx.try_global::<InvalidationCounters>() {
            counters
                .thumbnail_regen
                .set(counters.thumbnail_regen.get() + 1);
        }
    }

    pub(crate) fn mask_thumbnail_regen(cx: &App) {
        if let Some(counters) = cx.try_global::<InvalidationCounters>() {
            counters
                .mask_thumbnail_regen
                .set(counters.mask_thumbnail_regen.get() + 1);
        }
    }
}

/// The release stub: no counters, no global — the increment sites compile
/// unchanged and optimize away. (`root_notify` has no release call site —
/// its observer hook is debug-gated — so the stub omits it.)
#[cfg(not(any(test, debug_assertions)))]
pub(crate) mod invalidation_counter {
    use gpui_kit::App;

    pub(crate) fn composite(_: &App) {}
    pub(crate) fn frame_rebuild(_: &App) {}
    pub(crate) fn thumbnail_regen(_: &App) {}
    pub(crate) fn mask_thumbnail_regen(_: &App) {}
}

/// Converts a premultiplied-RGBA bitmap into the BGRA texture GPUI expects.
pub fn bgra_render_image(bitmap: &Bitmap) -> Arc<RenderImage> {
    let mut data = bitmap.data.clone();
    for px in data.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let buffer = image::ImageBuffer::from_raw(bitmap.width as u32, bitmap.height as u32, data)
        .expect("bitmap size matches data");
    Arc::new(RenderImage::new(vec![Frame::new(buffer)]))
}

/// `ByteCountFormatter.string(fromByteCount:countStyle: .file)`: decimal
/// units, one decimal below each unit's next step.
pub fn byte_count_label(bytes: usize) -> String {
    const UNITS: [(&str, f64); 4] = [
        ("GB", 1_000_000_000.0),
        ("MB", 1_000_000.0),
        ("kB", 1_000.0),
        ("bytes", 1.0),
    ];
    let value = bytes as f64;
    for (unit, step) in UNITS {
        if value >= step {
            let scaled = value / step;
            if unit == "bytes" {
                return format!("{bytes} bytes");
            }
            let digits = if scaled < 10.0 { 1 } else { 0 };
            return format!("{scaled:.digits$} {unit}");
        }
    }
    "0 bytes".to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        TransformField, byte_count_label, clamp_font_size, format_number, leading_from_text,
        leading_to_text, normalize_rotation, parse_field, resize_transform, transform_field_texts,
    };
    use compositor_core::geometry::{CGPoint, CGSize};
    use compositor_core::transform::LayerTransform;

    // The doc↔screen mapping and zoom-clamp coverage moved with the viewport
    // onto the core session (crates/core/src/canvas_viewport.rs, ticket 38).

    /// ByteCountFormatter's file style: decimal units, one decimal while
    /// under ten of the unit.
    #[test]
    fn byte_counts_format_like_the_swift_formatter() {
        assert_eq!(byte_count_label(0), "0 bytes");
        assert_eq!(byte_count_label(999), "999 bytes");
        assert_eq!(byte_count_label(345_000), "345 kB");
        assert_eq!(byte_count_label(1_234_000), "1.2 MB");
        assert_eq!(byte_count_label(12_400_000), "12 MB");
        assert_eq!(byte_count_label(2_500_000_000), "2.5 GB");
    }

    #[test]
    fn format_number_drops_trailing_zeros() {
        // TransformValueField.formatted.
        assert_eq!(format_number(34.0), "34");
        assert_eq!(format_number(34.004), "34");
        assert_eq!(format_number(-0.004), "0");
        assert_eq!(format_number(12.25), "12.25");
        assert_eq!(format_number(1.0 / 3.0), "0.33");
    }

    #[test]
    fn font_size_clamps_to_the_type_bar_range() {
        assert_eq!(clamp_font_size(40.0), 40.0);
        assert_eq!(clamp_font_size(0.2), 1.0);
        assert_eq!(clamp_font_size(5000.0), 2000.0);
    }

    #[test]
    fn leading_shows_blank_for_auto() {
        assert_eq!(leading_to_text(0.0), "");
        assert_eq!(leading_to_text(86.4), "86");
        assert_eq!(leading_from_text(""), 0.0);
        assert_eq!(leading_from_text(" 48 "), 48.0);
        assert_eq!(leading_from_text("auto"), 0.0);
        assert_eq!(leading_from_text("9000"), 5000.0);
        assert_eq!(leading_from_text("-4"), 0.0);
        assert_eq!(leading_from_text("NaN"), 0.0);
    }

    #[test]
    fn resize_transform_scales_the_other_side_when_locked() {
        let value = LayerTransform::new(CGPoint::ZERO, CGSize::new(100.0, 50.0));
        let width = resize_transform(value, 200.0, true, true);
        assert_eq!(width.size, CGSize::new(200.0, 100.0));
        // Unlocked keeps the other side.
        let plain = resize_transform(value, 200.0, true, false);
        assert_eq!(plain.size, CGSize::new(200.0, 50.0));
        let height = resize_transform(value, 100.0, false, true);
        assert_eq!(height.size, CGSize::new(200.0, 100.0));
        // Swift's `guard number >= 1`: small and non-finite numbers change nothing.
        assert_eq!(resize_transform(value, 0.5, true, true).size, value.size);
        assert_eq!(
            resize_transform(value, f64::NAN, false, false).size,
            value.size
        );
    }

    #[test]
    fn rotation_wraps_through_360() {
        assert_eq!(normalize_rotation(450.0), 90.0);
        assert_eq!(normalize_rotation(-90.0), -90.0);
        assert_eq!(normalize_rotation(720.0), 0.0);
    }

    #[test]
    fn parse_field_refuses_garbage() {
        assert_eq!(parse_field(" 12.5 "), Some(12.5));
        assert_eq!(parse_field("12px"), None);
        assert_eq!(parse_field(""), None);
        assert_eq!(parse_field("inf"), None);
        assert_eq!(parse_field("NaN"), None);
    }

    #[test]
    fn transform_field_names_are_distinct() {
        // The subscription roles must not collapse when copied.
        assert!(TransformField::X != TransformField::Y);
        assert!(TransformField::Width != TransformField::Height);
    }

    #[test]
    fn dragged_transform_reaches_the_inspector_fields_verbatim() {
        // The Move drag previews start + delta; the fields must show the
        // previewed draft, not the drag's start.
        let start = LayerTransform::new(CGPoint::new(40.0, 20.0), CGSize::new(100.0, 50.0));
        let mut dragged = start;
        dragged.origin.x += 12.5;
        dragged.origin.y -= 7.0;
        let pixel_size = CGSize::new(100.0, 50.0);
        let fields = transform_field_texts(dragged, pixel_size);
        let text_of = |field: TransformField| {
            fields
                .iter()
                .find(|(f, _)| *f == field)
                .map(|(_, text)| text.clone())
                .unwrap()
        };
        assert_eq!(text_of(TransformField::X), "52.50");
        assert_eq!(text_of(TransformField::Y), "13");
        assert_eq!(text_of(TransformField::Width), "100");
        assert_eq!(text_of(TransformField::Height), "50");
        assert_eq!(text_of(TransformField::Scale), "100");
        assert_eq!(text_of(TransformField::Rotation), "0");
        // A resize through a handle moves the scale and the sizes together.
        dragged.size.width *= 1.5;
        dragged.size.height *= 1.5;
        let fields = transform_field_texts(dragged, pixel_size);
        assert_eq!(fields[2].1, "150");
        assert_eq!(fields[3].1, "75");
        assert_eq!(fields[4].1, "150");
    }

    mod delete_ask {
        use super::super::{delete_ask_targets, delete_candidate_ids, time_seed};
        use compositor_core::document::{CanvasDocument, ImageLayer};
        use compositor_core::geometry::CGPoint;
        use compositor_core::raster::{Bitmap, ImportedImage};

        fn empty_layer(name: &str) -> ImageLayer {
            ImageLayer::with_asset(ImportedImage::new(Bitmap::new(4, 4), name), CGPoint::ZERO)
        }

        #[test]
        fn candidates_are_the_selected_layers_in_document_order() {
            let mut document = CanvasDocument::new(10, 10);
            document.layers = vec![empty_layer("a"), empty_layer("b"), empty_layer("c")];
            let ids: Vec<_> = document.layers.iter().map(|l| l.id).collect();
            // Only the selected ids pass, in document order.
            assert_eq!(
                delete_candidate_ids(&document, &[ids[2], ids[0]]),
                vec![ids[0], ids[2]]
            );
            assert!(delete_candidate_ids(&document, &[]).is_empty());
        }

        #[test]
        fn targets_are_the_surviving_layers_masked_by_a_deleted_one() {
            // base <- dependent takes its live mask from base. Deleting base
            // must ask; deleting dependent must not.
            let mut document = CanvasDocument::new(10, 10);
            let base = empty_layer("base");
            let mut dependent = empty_layer("dependent");
            let isolated = empty_layer("isolated");
            let base_id = base.id;
            dependent.mask_source_id = Some(base_id);
            let (dependent_id, isolated_id) = (dependent.id, isolated.id);
            document.layers = vec![base, dependent, isolated];

            let targets = delete_ask_targets(&document, &[base_id]);
            assert_eq!(targets, vec![dependent_id]);
            assert!(delete_ask_targets(&document, &[dependent_id]).is_empty());
            assert!(delete_ask_targets(&document, &[isolated_id]).is_empty());
        }

        #[test]
        fn a_folder_delete_counts_its_contents_suppliers() {
            // Deleting the folder removes the child too, so a layer masked by
            // the child is a target even though the child wasn't named.
            let mut document = CanvasDocument::new(10, 10);
            let mut folder = empty_layer("folder");
            folder.is_group = true;
            let mut child = empty_layer("child");
            let folder_id = folder.id;
            child.parent_id = Some(folder_id);
            let child_id = child.id;
            let mut dependent = empty_layer("dependent");
            dependent.mask_source_id = Some(child_id);
            let dependent_id = dependent.id;
            document.layers = vec![folder, child, dependent];

            assert_eq!(
                delete_ask_targets(&document, &[folder_id]),
                vec![dependent_id]
            );
        }

        #[test]
        fn time_seed_varies_across_calls() {
            // Time-derived seeds (Filters.swift:223's UInt32.random): a run of
            // calls must not all collapse onto one value.
            let seeds: std::collections::HashSet<u32> = (0..64).map(|_| time_seed()).collect();
            assert!(seeds.len() > 1, "seeds never varied: {seeds:?}");
        }
    }

    /// The ticket-37 invalidation scenarios (ADR 0003 step G): the
    /// measure-first harness. Each test drives one of the two notification
    /// hot spots through the real entity tree (root → content → canvas +
    /// layers panel), reads the invalidation counter, and pins the numbers
    /// the ticket's landing record and the diff report carry.
    mod invalidation_scenarios {
        use super::super::{ProjectWorkspaceView, invalidation_counter};
        use crate::ui::floating_panel::{PanelContent, panel_names};
        use compositor_core::document::LayerMask;
        use compositor_core::raster::{Bitmap, ImportedImage};
        use compositor_core::session::EditorSession;
        use gpui_kit::component::slider::{SliderEvent, SliderValue};
        use gpui_kit::test::TestWindowExt as _;
        use gpui_kit::{
            AppContext as _, Bounds, Entity, TestAppContext, VisualTestContext, WindowBounds,
            WindowOptions,
        };

        /// Slider-change events per drag scenario.
        const EVENTS: usize = 5;
        /// Asset layers in every scenario document.
        const LAYERS: usize = 3;

        /// The production window (gpui-kit `Root` around the root view) the
        /// scenarios run in; returns it plus the root view entity.
        fn open_scenario_window(
            cx: &mut TestAppContext,
        ) -> (Entity<ProjectWorkspaceView>, VisualTestContext) {
            let (window, root) = cx.update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::maximized(None, cx))),
                        ..Default::default()
                    },
                    cx,
                    |window, cx| cx.new(|cx| ProjectWorkspaceView::new(window, cx)),
                )
                .expect("open the scenario window")
            });
            (root, VisualTestContext::from_window(window, cx))
        }

        fn colored(color: [u8; 4]) -> ImportedImage {
            ImportedImage::new(Bitmap::new_clearing(8, 8, color), "L")
        }

        /// The opening document with `LAYERS` asset layers (create_document
        /// seeds "Layer 1"; one more `add_blank_layer` tops it up); the
        /// bottom `masks` layers carry a revealing mask.
        fn seed_layers(
            cx: &mut gpui_kit::VisualTestContext,
            session: &Entity<EditorSession>,
            masks: usize,
        ) {
            session.update(cx, |session, _| {
                session.create_document(64, 64, true);
                for _ in 1..LAYERS {
                    session.add_blank_layer();
                }
                let document = session.document.as_mut().unwrap();
                for (index, layer) in document.layers.iter_mut().enumerate() {
                    layer.asset = Some(colored([40, 40, (index * 70 + 40) as u8, 255]));
                    if index < masks {
                        layer.mask = Some(LayerMask::solid(true));
                    }
                }
            });
        }

        /// The active tab's session entity, cloned out of the root.
        fn active_session(
            root: &Entity<ProjectWorkspaceView>,
            cx: &impl gpui_kit::AppContext,
        ) -> Entity<EditorSession> {
            root.read_with(cx, |root, cx| {
                root.workspace.read(cx).current().session.clone()
            })
        }

        /// One full frame: invalidation delivered, every view re-rendered,
        /// the caches refilled — the unit the counters tally.
        fn render_frame(cx: &mut gpui_kit::VisualTestContext) {
            cx.update(|window, cx| window.render_frame(cx));
        }

        /// Fires one real slider Change event at the Levels panel's first
        /// slider (the path a drag produces: the panel's subscription runs
        /// the shell's preview recompute).
        fn fire_slider_change(
            root: &Entity<ProjectWorkspaceView>,
            cx: &mut gpui_kit::VisualTestContext,
            value: f32,
        ) {
            let slider = root.read_with(cx, |root, cx| {
                match root.panel_views.get(panel_names::LEVELS_PANEL) {
                    Some(PanelContent::Levels(panel)) => panel.read(cx).sliders()[0].clone(),
                    _ => panic!("the levels panel is not open"),
                }
            });
            slider.update(cx, |_, cx| {
                cx.emit(SliderEvent::Change(SliderValue::Single(value)));
            });
        }

        /// Slider-drag scenario: N change events on the Levels panel over a
        /// three-layer document. Each event delivers one root invalidation
        /// and one frame (the same shape a real drag produces — one frame
        /// per event). The slider changes preview state only — no layer
        /// asset, transform, effect or canvas size moves — so the thumbnail
        /// regenerations it forces are pure waste.
        #[gpui_kit::test]
        fn slider_drag_invalidation(cx: &mut TestAppContext) {
            cx.update(gpui_kit::init);
            cx.update(invalidation_counter::install);
            let (root, mut cx) = open_scenario_window(cx);
            let session = active_session(&root, &cx);
            seed_layers(&mut cx, &session, 0);
            root.update(&mut cx, |root, cx| root.open_levels_panel(cx));
            render_frame(&mut cx); // warm the caches; the tallies start clean

            cx.update(|_, cx| invalidation_counter::reset(cx));
            for event in 0..EVENTS {
                fire_slider_change(&root, &mut cx, 100.0 + event as f32);
            }
            let counts = cx.update(|_, cx| invalidation_counter::snapshot(cx));
            // Content-keyed thumbnails (ticket 37): a slider change touches
            // none of the drawn inputs, so zero rows regenerate — the frame
            // rebuild (the preview-driven, necessary cost) is unchanged.
            // Baseline before the key: [5, 5, 5, 15, 0] — every event
            // regenerated all three rows.
            assert_eq!(
                counts,
                [EVENTS as u64, EVENTS as u64, EVENTS as u64, 0, 0],
                "slider drag: (root_notify, composite, frame_rebuild, thumbnail_regen, mask)"
            );
        }

        /// Undo scenario: one pixel edit and its undo over a three-layer
        /// document whose bottom layer carries a mask. The frame rebuild is
        /// the necessary cost; the untouched rows' and the mask's thumbnail
        /// regenerations are the waste the content key removes.
        #[gpui_kit::test]
        fn pixel_edit_and_undo_invalidation(cx: &mut TestAppContext) {
            cx.update(gpui_kit::init);
            cx.update(invalidation_counter::install);
            let (root, mut cx) = open_scenario_window(cx);
            let session = active_session(&root, &cx);
            seed_layers(&mut cx, &session, 1);
            render_frame(&mut cx); // warm the caches; the tallies start clean

            cx.update(|_, cx| invalidation_counter::reset(cx));
            // The pixel edit: one undo step replacing the active layer's
            // pixels (the canonical `history_edit` + `touch` shape). One
            // root invalidation, one frame.
            root.update(&mut cx, |root, cx| {
                root.history_edit(
                    "Fill",
                    |session| {
                        if let Some(id) = session.active_layer_id
                            && let Some(document) = session.document.as_mut()
                            && let Some(layer) = document.layer_mut(id)
                        {
                            layer.asset = Some(colored([200, 60, 60, 255]));
                        }
                    },
                    cx,
                );
                root.touch(cx);
            });
            let edit = cx.update(|_, cx| invalidation_counter::snapshot(cx));
            // Content-keyed (ticket 37): only the edited row's asset pointer
            // moved, so one row regenerates; the untouched rows and the mask
            // thumbnail hit. Baseline before the key: [1, 1, 1, 3, 1].
            assert_eq!(
                edit,
                [1, 1, 1, 1, 0],
                "pixel edit: (root_notify, composite, frame_rebuild, thumbnail_regen, mask)"
            );

            // The undo restores the pre-edit pixels; same per-frame shape.
            cx.update(|_, cx| invalidation_counter::reset(cx));
            root.update(&mut cx, |root, cx| {
                root.workspace.update(cx, |workspace, cx| {
                    workspace
                        .current_mut()
                        .session
                        .update(cx, |session, _| session.undo())
                });
                root.touch(cx);
            });
            let undo = cx.update(|_, cx| invalidation_counter::snapshot(cx));
            // The undo restores the pre-edit snapshot — the untouched layers'
            // `Arc`s survive the round trip, so only the edited row's pixels
            // really changed. Baseline before the key: [1, 1, 1, 3, 1].
            assert_eq!(
                undo,
                [1, 1, 1, 1, 0],
                "undo: (root_notify, composite, frame_rebuild, thumbnail_regen, mask)"
            );
        }
    }

    /// The panel double-lease rule (the crash a curves drag exposed): a
    /// panel-entity callback may not re-enter the shell for a read of the
    /// same panel — the shell reads a panel only where no lease on it is
    /// held (root-originated flows), and the panel's own update hands its
    /// values over through the seam.
    #[cfg(test)]
    mod panel_reentrancy_tests {
        use super::super::ProjectWorkspaceView;
        use crate::ui::floating_panel::{PanelContent, panel_names};
        use compositor_core::filters::FilterKind;
        use compositor_core::raster::ImportedImage;
        use gpui_kit::{
            AppContext as _, Bounds, Entity, TestAppContext, VisualTestContext, WindowBounds,
            WindowOptions,
        };

        /// The production window the re-entrancy scenarios run in.
        fn open_scenario_window(
            cx: &mut TestAppContext,
        ) -> (Entity<ProjectWorkspaceView>, VisualTestContext) {
            cx.update(gpui_kit::init);
            let (window, root) = cx.update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::maximized(None, cx))),
                        ..Default::default()
                    },
                    cx,
                    |window, cx| cx.new(|cx| ProjectWorkspaceView::new(window, cx)),
                )
                .expect("open the scenario window")
            });
            (root, VisualTestContext::from_window(window, cx))
        }

        /// A document with one image layer, so `begin_preview` has pixels to
        /// capture when the filter panel opens.
        fn open_filter_panel(
            root: &Entity<ProjectWorkspaceView>,
            kind: FilterKind,
            cx: &mut VisualTestContext,
        ) {
            root.update(cx, |root, cx| {
                root.workspace.update(cx, |workspace, cx| {
                    workspace.current_mut().session.update(cx, |session, _| {
                        session.create_document(64, 64, true);
                        session.insert_image(
                            ImportedImage::new(
                                compositor_core::raster::Bitmap::new_clearing(
                                    64,
                                    64,
                                    [40, 60, 200, 255],
                                ),
                                "L",
                            ),
                            None,
                        );
                    })
                });
                root.open_filter_panel(kind, cx);
            });
        }

        fn filter_panel(
            root: &Entity<ProjectWorkspaceView>,
            cx: &VisualTestContext,
        ) -> Entity<crate::ui::filter_sheet::FilterPanelView> {
            root.read_with(cx, |root, _| {
                match root.panel_views.get(panel_names::FILTER_PANEL) {
                    Some(PanelContent::Filter(panel)) => panel.clone(),
                    _ => panic!("the filter panel is not open"),
                }
            })
        }

        /// The curves drag's re-entrant path (the reported crash): the panel
        /// computes its settings and hands them — with its seed — across the
        /// seam, so the shell's preview pipeline never reads the panel from
        /// inside the panel's own update.
        #[gpui_kit::test]
        fn curves_preview_reentry_from_the_panel_does_not_double_lease(cx: &mut TestAppContext) {
            let (root, mut cx) = open_scenario_window(cx);
            open_filter_panel(&root, FilterKind::Curves, &mut cx);
            let panel = filter_panel(&root, &cx);
            panel.update(&mut cx, |panel, cx| panel.apply_curves_preview(cx));
            cx.update(|_, cx| {
                let root = root.read(cx);
                assert!(
                    root.preview.is_some(),
                    "the curves preview ran through the seam"
                );
            });
        }

        /// A filter slider's re-entrant path, same seam shape as the drag.
        #[gpui_kit::test]
        fn filter_slider_reentry_from_the_panel_does_not_double_lease(cx: &mut TestAppContext) {
            let (root, mut cx) = open_scenario_window(cx);
            open_filter_panel(&root, FilterKind::GaussianBlur, &mut cx);
            let panel = filter_panel(&root, &cx);
            panel.update(&mut cx, |panel, cx| panel.slider_changed(12.0, cx));
            cx.update(|_, cx| {
                let root = root.read(cx);
                assert!(root.preview.is_some(), "the slider preview ran");
            });
        }

        /// The footer's OK dispatches through the root (no panel lease), so
        /// the shell's confirm reads the panel's live kind label legally.
        #[gpui_kit::test]
        fn filter_confirm_from_the_root_context_reads_the_panel(cx: &mut TestAppContext) {
            let (root, mut cx) = open_scenario_window(cx);
            open_filter_panel(&root, FilterKind::GaussianBlur, &mut cx);
            root.update(&mut cx, |root, cx| root.confirm_filter_panel(cx));
            cx.update(|_, cx| {
                let root = root.read(cx);
                assert!(
                    !root.panel_views.contains_key(panel_names::FILTER_PANEL),
                    "the confirm closed the panel"
                );
            });
        }

        /// `gaussianBlurSoftensAHardEdgeAndSpreadsPastTheLayerEdgeAsOneUndoStep`
        /// (FilterTests.swift): the blur is not clamped at the layer's edge —
        /// the layer is given room, the blur spreads into it and whatever
        /// stays empty is cut away again (Filters.swift `growForBlur` /
        /// `PixelFilter.trimmed`). The 40x20 layer, opaque across its full
        /// height for its left half, could not have grown vertically unless
        /// the blur went past the edge.
        #[gpui_kit::test]
        fn gaussian_blur_softens_a_hard_edge_and_spreads_past_the_layer_edge_as_one_undo_step(
            cx: &mut TestAppContext,
        ) {
            let (root, mut cx) = open_scenario_window(cx);
            root.update(&mut cx, |root, cx| {
                root.workspace.update(cx, |workspace, cx| {
                    workspace.current_mut().session.update(cx, |session, _| {
                        session.create_document(40, 20, true);
                        // Left half opaque white, right half transparent.
                        let mut image = compositor_core::raster::Bitmap::new(40, 20);
                        for y in 0..20 {
                            for x in 0..20 {
                                image.set_pixel(x, y, [255, 255, 255, 255]);
                            }
                        }
                        session.insert_image(ImportedImage::new(image, "Half"), None);
                    });
                });
                root.open_filter_panel(FilterKind::GaussianBlur, cx);
                // session.updateFilter(FilterSettings(radius: 3), preview: true).
                root.apply_filter_preview(
                    FilterKind::GaussianBlur,
                    compositor_core::filters::FilterSettings {
                        radius: 3.0,
                        ..Default::default()
                    },
                    0,
                    cx,
                );
            });
            let undo_count = root.read_with(&cx, |root, cx| {
                root.workspace
                    .read(cx)
                    .current()
                    .session
                    .read(cx)
                    .history
                    .undo_count()
            });
            root.update(&mut cx, |root, cx| root.confirm_filter_panel(cx));
            root.read_with(&cx, |root, cx| {
                let session = root.workspace.read(cx).current().session.read(cx);
                assert_eq!(
                    session.history.undo_count(),
                    undo_count + 1,
                    "the commit is one undo step"
                );
                let layer = session.active_layer().expect("the layer survives");
                let result = layer.asset.as_ref().expect("pixels").image.as_ref();
                let origin = layer.transform.origin;
                assert!(
                    origin.x < 0.0 && origin.y < 0.0,
                    "the layer grew on every side: origin {origin:?}"
                );
                assert!(
                    result.height > 20,
                    "the blur spread past the layer's edge: height {}",
                    result.height
                );
                assert!(
                    result.width < 40,
                    "and the half that stayed empty was trimmed away: width {}",
                    result.width
                );
                let middle: Vec<u8> = (0..result.width)
                    .map(|x| result.data[(result.height / 2 * result.width + x) * 4 + 3])
                    .collect();
                // 250 rather than 255: the block's centre is 10 px from its
                // edges, which at this radius leaves it a fraction of a level
                // below full opacity. What would break here is the inside
                // fading, not rounding.
                let max = middle.iter().max().copied().unwrap();
                assert!(max >= 250, "the block's inside is untouched: {max}");
                assert!(
                    middle.iter().any(|a| *a > 20 && *a < 235),
                    "the hard edge is now soft"
                );
                let last = *middle.last().unwrap();
                assert!(last < 20, "and it fades out on the far side: {last}");
            });
        }
    }
}
