//! Application state: the `EditorSession` plus the viewport, the active
//! canvas drag, the modal sheet and the rendered-frame cache. Owned by the
//! window's root view (see `workspace.rs`).

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::WindowExt;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::slider::SliderEvent;
use gpui_kit::component::slider::SliderState;
use gpui_kit::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, Focusable, KeyDownEvent, KeyUpEvent,
    Pixels, Point, RenderImage, Subscription, Window, px,
};
use image::Frame;

use compositor_core::document::CanvasDocument;
use compositor_core::document::ImageLayer;
use compositor_core::filters::{FilterJob, FilterKind, FilterSettings};
use compositor_core::geometry::{CGPoint, CGRect, CGSize};
use compositor_core::raster::{Bitmap, ImportedImage};
use compositor_core::render;
use compositor_core::session::EditorSession;
use compositor_core::tools::brush::RasterEdit;
use compositor_core::tools::warp::WarpStroke;
use compositor_core::transform::LayerTransform;

use crate::canvas::inline_text_editor::inline_text_key;

pub const RULER_SIZE: f32 = 20.0;
pub const MIN_ZOOM: f64 = 0.008;
pub const MAX_ZOOM: f64 = 32.0;

/// Document coordinates to viewport pixels under the given zoom and pan.
/// Pure so tests can cover it without a window.
pub fn doc_to_screen_at(zoom: f64, pan: (f64, f64), x: f64, y: f64) -> (f32, f32) {
    ((x * zoom + pan.0) as f32, (y * zoom + pan.1) as f32)
}

/// Viewport pixels back to document coordinates; the inverse of
/// `doc_to_screen_at`.
pub fn screen_to_doc_at(zoom: f64, pan: (f64, f64), x: f32, y: f32) -> CGPoint {
    CGPoint::new((x as f64 - pan.0) / zoom, (y as f64 - pan.1) / zoom)
}

/// Clamps a zoom factor to the editor's supported range.
pub fn clamp_zoom(zoom: f64) -> f64 {
    zoom.clamp(MIN_ZOOM, MAX_ZOOM)
}

/// What the pointer is currently doing on the canvas.
pub enum CanvasDrag {
    /// Hand tool or middle-mouse panning.
    Pan {
        start: Point<Pixels>,
        start_pan: (f64, f64),
    },
    /// A brush-family stroke in progress on the layer captured at
    /// `begin_stroke` (EditorSession+Brush.swift:130-133 commits to the
    /// captured layer, never to whatever is active at mouse-up).
    Stroke(RasterEdit, uuid::Uuid),
    /// Gradient drag: the edit plus the document-space start point, on the
    /// captured layer.
    Gradient(RasterEdit, CGPoint, uuid::Uuid),
    /// Marquee / freehand lasso drag; the draft lives in the session.
    MarqueeLasso,
    /// A Smudge or Liquify stroke working on a document-size copy.
    Warp(WarpStroke),
    /// The Shape tool dragging a shape out; the draft rect grows live.
    Shape { start: CGPoint },
    /// Move tool drag.
    MoveLayer {
        start_origin: compositor_core::transform::LayerTransform,
        start: CGPoint,
    },
    /// Crop drag; the drag lives in the session.
    Crop,
    /// Guide drag (create from a ruler or move an existing line); the drag
    /// lives in the session's `guide_drag`.
    Guide,
    /// Cmd-drag moving the selected pixels; the lifted raster lives in the
    /// session's `pixel_move` (`EditorCanvas.pixelDragStart`).
    PixelMove { start: CGPoint },
    /// Plain drag moving the selection outline only; the origin lives in the
    /// session's `selection_move_origin`.
    SelectionMove { start: CGPoint },
    /// The Type tool dragging a paragraph box out (`textBoxRect`); the rect
    /// lives app-side in `text_box_rect`.
    TextBox { start: CGPoint },
    /// An inline text editor resize-handle drag (`InlineTextEditor.mouseDown`
    /// /`mouseDragged`'s resize branch): which handle, where the drag began,
    /// the transform the box showed when it began and the box size the
    /// resize divides by; the draft itself lives in the session.
    TextResize {
        handle: usize,
        start: CGPoint,
        old: LayerTransform,
        source: CGSize,
        draft: compositor_core::session::TextDraft,
    },
}

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
/// the curve canvas needs element bounds, and because the sheet's sliders
/// have nothing to do with it; it is created when the Curves sheet body
/// first renders and dropped with the sheet.
pub struct CurvesEditor {
    /// The curve being edited: four channels of control points plus the
    /// visible channel.
    pub settings: compositor_core::adjust::CurvesSettings,
    /// The selected control point (drawn in the accent color, the Remove
    /// point target).
    pub selected: Option<usize>,
    /// The point under an active drag, if any.
    pub dragging: Option<usize>,
    /// The curve canvas's last layout bounds (window pixels); pointer events
    /// convert through this into curve units.
    pub canvas_bounds: Bounds<Pixels>,
}

impl CurvesEditor {
    pub(crate) fn new() -> Self {
        CurvesEditor {
            settings: compositor_core::adjust::CurvesSettings::default(),
            selected: None,
            dragging: None,
            canvas_bounds: Bounds::new(Point::new(px(0.), px(0.)), gpui_kit::size(px(0.), px(0.))),
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

/// Everything the window shows, owned as a single GPUI entity. The document
/// lives in the workspace's tabs (`workspace.current().session.read(cx)`):
/// every tab carries its own session entity and controller, exactly like
/// Swift's `ProjectWorkspace` (design doc §1, plan A; ADR 0001; the busy
/// flags moved onto the session in stage-D ticket 34).
pub struct AppState {
    pub workspace: crate::tabs::ProjectWorkspace,
    pub focus: FocusHandle,
    /// Screen pixels per document pixel.
    pub zoom: f64,
    /// Document (0, 0) in viewport pixels.
    pub pan: (f64, f64),
    /// Bumped on every session/viewport mutation; invalidates the frame image.
    pub revision: u64,
    frame_image: Option<Arc<RenderImage>>,
    frame_image_rev: u64,
    /// The tab the cached frame was rendered for — the cache key is
    /// `(tab_id, revision)`, so switching tabs never shows a stale frame.
    frame_image_tab: crate::project::TabId,
    /// The canvas paint area, updated by every frame.
    pub viewport: Bounds<Pixels>,
    pub drag: Option<CanvasDrag>,
    pub sheet: Option<SheetState>,
    /// The floating panels (design doc §5): the registry of open panels plus
    /// their remembered positions, and the per-panel widget bundles.
    pub panels: crate::ui::floating_panel::FloatingPanels,
    pub panel_widgets: crate::ui::floating_panel::PanelWidgets,
    pub status: Option<String>,
    /// The layers-panel rename edit field (`renamingLayerID`'s widget half;
    /// the session flag itself lives in the core session since stage-D
    /// ticket 35).
    pub rename_input: Option<Entity<InputState>>,
    thumbnails: HashMap<(uuid::Uuid, bool), (u64, Arc<RenderImage>)>,
    /// Document dims the viewport was last fit for, and the tab it was fit
    /// for: Swift rebuilds the view per tab (`.id(tab.id)`), so a tab switch
    /// refits the viewport too.
    pub fit_for: FitDims,
    pub fit_tab: crate::project::TabId,
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
    /// The Shape tool's drag: the shape being placed and its current rect.
    pub shape_shape: Option<compositor_core::document::LayerShape>,
    pub shape_rect: Option<CGRect>,
    /// The gradient tool's drag end, in document coordinates.
    pub gradient_cursor: Option<CGPoint>,
    /// Tool option sliders (brush size/hardness/opacity/smoothing,
    /// wand tolerance, gradient opacity).
    pub brush_sliders: [Entity<SliderState>; 4],
    pub wand_slider: Entity<SliderState>,
    pub gradient_slider: Entity<SliderState>,
    /// The active layer's opacity (Layers panel header).
    pub opacity_slider: Entity<SliderState>,
    pub subscriptions: Vec<Subscription>,
    /// The tool saved while Space temporarily turns the canvas into the Hand
    /// tool (`spaceHeld` in EditorCanvas.swift).
    pub space_saved_tool: Option<compositor_core::document::NavigationTool>,
    /// The open color picker's widgets (`color_picker_ui`).
    pub picker_ui: Option<crate::ui::color_picker_sheet::PickerUi>,
    /// The Curves filter sheet's editor; `None` while no Curves sheet is
    /// open (see `CurvesEditor`).
    pub curves_editor: Option<CurvesEditor>,
    /// Move tool: select layers by clicking the canvas
    /// (`transformAutoSelect` in EditorSession.swift, persisted through
    /// ToolDefaults there; the core session has no such field, so the app
    /// carries it until it is wired into canvas picking).
    pub transform_auto_select: bool,
    /// The Type header's numeric fields (size, tracking, leading).
    pub type_size_input: Entity<InputState>,
    pub type_tracking_input: Entity<InputState>,
    pub type_leading_input: Entity<InputState>,
    /// The Transform inspector's numeric fields (X, Y, W, H, Scale %, °).
    pub transform_x_input: Entity<InputState>,
    pub transform_y_input: Entity<InputState>,
    pub transform_w_input: Entity<InputState>,
    pub transform_h_input: Entity<InputState>,
    pub transform_scale_input: Entity<InputState>,
    pub transform_rotation_input: Entity<InputState>,
    /// The empty tab's welcome form's numeric fields (`render_welcome`,
    /// NewCanvasSheet's `@State width`/`height`, shown while the tab has no
    /// document). Built here because an `InputState` needs the window.
    pub welcome_width_input: Entity<InputState>,
    pub welcome_height_input: Entity<InputState>,
    /// The inline canvas text editor (`InlineTextEditor.swift`): one
    /// persistent multi-line input, shown as an overlay while a text draft
    /// is open (see `inline_text`).
    pub inline_text_input: Entity<gpui_kit::component::input::TextareaState>,
    /// Whether the inline editor is bound to the current draft.
    pub inline_text_open: bool,
    /// The last text pushed into the inline input; keeps the two-way
    /// binding from echoing the user's own keystrokes back.
    pub inline_text_pushed: Option<String>,
    /// The inline caret's blink phase anchor: any keystroke or caret move
    /// resets it, so the caret holds solid while the user is working
    /// (Swift's `caretBlinkTimer` invalidation on text change).
    pub inline_text_caret_phase: std::time::Instant,
    /// The Type tool's in-progress box drag (`textBoxRect`).
    pub text_box_rect: Option<CGRect>,
    /// Where an external file drag hovers, in document coordinates — the
    /// import lands there on drop (`ImageFileDrop`'s drop point).
    pub drop_point_doc: Option<CGPoint>,
    /// Monotonic epoch for the opacity-digit pair window (0.6 s).
    key_epoch: std::time::Instant,
    /// A save panel raised mid-flow (a confirmation's Save button): the tab
    /// whose controller owns the pending `SaveAs` and what to resume after
    /// `finish_save`.
    pub pending_save: Option<(crate::project::TabId, SaveAfter)>,
    /// Add Noise / grain's random pattern for the open filter panel: fixed
    /// while the panel is open so changing Amount doesn't reshuffle it, and
    /// shared by the preview and the committed pixels (FilterEdit.seed,
    /// Filters.swift:223).
    pub filter_seed: u32,
    /// The live-mask delete ask (`deleteWithLiveMaskChoice`,
    /// LiveLayerMask.swift:106-125): the layers waiting for the
    /// Bake / Remove Links / Cancel answer, while the three-button Confirm
    /// sheet is up.
    pub delete_ask_ids: Option<Vec<uuid::Uuid>>,
    /// The observe subscription on the active tab's session entity
    /// (ADR 0001): a session-entity notify re-renders the window.
    /// Re-attached whenever the active tab changes
    /// ([`AppState::sync_session_observer`]); non-active tabs are not
    /// observed. Kept in a dedicated field — not `subscriptions` — so a tab
    /// switch drops the outgoing tab's observation.
    session_observer: Option<(Entity<EditorSession>, Subscription)>,
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
fn slider(min: f32, max: f32, step: f32, value: f32) -> SliderState {
    SliderState::new()
        .min(min)
        .max(max)
        .step(step)
        .default_value(value)
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

/// Document pixel dimensions the viewport was last fit for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FitDims {
    pub width: usize,
    pub height: usize,
}

impl FitDims {
    /// The sentinel meaning "refit requested" — no real document is 0×0.
    pub const REFIT: FitDims = FitDims {
        width: 0,
        height: 0,
    };
}

impl AppState {
    /// The current tab (its session entity and controller) — the rewrite of
    /// every former `self.tab().session` call site (design doc §1).
    pub fn tab(&mut self) -> &mut crate::tabs::ProjectTab {
        self.workspace.current_mut()
    }

    /// Shared view of the current tab, for `&self` readers (menu titles,
    /// scene building, compositing).
    pub fn current_tab(&self) -> &crate::tabs::ProjectTab {
        self.workspace.current()
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workspace = crate::tabs::ProjectWorkspace::new(cx);
        // The window opens on one 1280×800 document, as before.
        workspace
            .current()
            .session
            .update(cx, |session, _| session.create_document(1280, 800, true));
        // The persisted auto-select toggle is app-global in this port (Swift
        // keeps it per-session — docs/rust-vs-swift-diff-report.md); the
        // session-side defaults are applied per tab in ProjectTab::new, so
        // the panel sliders seed from already-persisted settings.
        let transform_auto_select = crate::settings::stored_auto_select();
        let (brush, wand_tolerance, gradient_opacity, style) = {
            let session = workspace.current().session.read(cx);
            (
                session.brush_settings,
                session.wand_settings.tolerance,
                session.gradient_settings.opacity,
                session.current_text_style(),
            )
        };
        let number_input = |value: f64, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).default_value(format_number(value)))
        };
        let type_size_input = number_input(style.font_size, window, cx);
        let type_tracking_input = number_input(style.tracking, window, cx);
        let type_leading_input =
            cx.new(|cx| InputState::new(window, cx).default_value(leading_to_text(style.leading)));
        let transform_x_input = number_input(0.0, window, cx);
        let transform_y_input = number_input(0.0, window, cx);
        let transform_w_input = number_input(0.0, window, cx);
        let transform_h_input = number_input(0.0, window, cx);
        let transform_scale_input = number_input(100.0, window, cx);
        let transform_rotation_input = number_input(0.0, window, cx);
        let inline_text_input = cx
            .new(|cx| gpui_kit::component::input::TextareaState::new(window, cx).default_value(""));
        // The welcome form's defaults: NewCanvasSheet's `@State width`/
        // `height` ("1920" / "1080").
        let welcome_width_input = number_input(1920.0, window, cx);
        let welcome_height_input = number_input(1080.0, window, cx);

        let brush_sliders = [
            cx.new(|_| slider(1.0, 300.0, 1.0, brush.diameter as f32)),
            cx.new(|_| slider(0.0, 100.0, 1.0, (brush.hardness * 100.0) as f32)),
            cx.new(|_| slider(1.0, 100.0, 1.0, (brush.opacity * 100.0) as f32)),
            cx.new(|_| slider(0.0, 100.0, 1.0, brush.smoothing as f32)),
        ];
        let wand_slider = cx.new(|_| slider(0.0, 255.0, 1.0, wand_tolerance as f32));
        let gradient_slider =
            cx.new(|_| slider(1.0, 100.0, 1.0, (gradient_opacity * 100.0) as f32));
        let opacity_slider = cx.new(|_| slider(0.0, 100.0, 1.0, 100.0));

        let mut state = AppState {
            workspace,
            focus: cx.focus_handle(),
            zoom: 1.0,
            pan: (0.0, 0.0),
            revision: 1,
            frame_image: None,
            frame_image_rev: 0,
            frame_image_tab: crate::project::TabId::nil(),
            viewport: Bounds::new(Point::new(px(0.), px(0.)), gpui_kit::size(px(0.), px(0.))),
            drag: None,
            sheet: None,
            panels: crate::ui::floating_panel::FloatingPanels::new(),
            panel_widgets: crate::ui::floating_panel::PanelWidgets::default(),
            status: None,
            rename_input: None,
            thumbnails: HashMap::new(),
            fit_for: FitDims::REFIT,
            fit_tab: crate::project::TabId::nil(),
            preview_base: None,
            camera_raw_samples_defringe: false,
            camera_raw_readout: None,
            preview: None,
            preview_base_transform: None,
            gradient_cursor: None,
            shape_shape: None,
            shape_rect: None,
            brush_sliders,
            wand_slider,
            gradient_slider,
            opacity_slider,
            subscriptions: Vec::new(),
            space_saved_tool: None,
            picker_ui: None,
            curves_editor: None,
            transform_auto_select,
            type_size_input,
            type_tracking_input,
            type_leading_input,
            transform_x_input,
            transform_y_input,
            transform_w_input,
            transform_h_input,
            transform_scale_input,
            transform_rotation_input,
            welcome_width_input,
            welcome_height_input,
            inline_text_input,
            inline_text_open: false,
            inline_text_pushed: None,
            inline_text_caret_phase: std::time::Instant::now(),
            text_box_rect: None,
            drop_point_doc: None,
            key_epoch: std::time::Instant::now(),
            pending_save: None,
            filter_seed: time_seed(),
            delete_ask_ids: None,
            session_observer: None,
        };
        state.subscribe_sliders(cx);
        state.subscribe_inputs(cx);
        state.sync_session_observer(cx);
        // `on_window_should_close` → `workspace.close_window` (two-phase):
        // a pending unsaved-changes dialog vetoes the close and presents
        // itself; once the flow completes the window closes for real.
        let view = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            view.update(cx, |state, cx| {
                let step = state.workspace.close_window(cx);
                state.sync_session_observer(cx);
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

    /// Monotonic seconds since the app started (`systemUptime`).
    pub fn uptime(&self) -> f64 {
        self.key_epoch.elapsed().as_secs_f64()
    }

    /// Space keyDown: the canvas pans until the key comes back up
    /// (EditorCanvas.swift:1825).
    pub fn space_down(&mut self, cx: &mut Context<Self>) {
        if self.space_saved_tool.is_some() {
            return;
        }
        let (has_document, has_draft, tool) = {
            let session = self.tab().session.read(cx);
            (
                session.document.is_some(),
                session.text_draft.is_some(),
                session.tool,
            )
        };
        if !has_document || has_draft {
            return;
        }
        self.space_saved_tool = Some(tool);
        self.tab().session.update(cx, |session, _| {
            session.tool = compositor_core::document::NavigationTool::Hand
        });
        self.touch(cx);
    }

    /// Space keyUp: the saved tool comes back (EditorCanvas.swift:1866).
    pub fn space_up(&mut self, cx: &mut Context<Self>) {
        if let Some(tool) = self.space_saved_tool.take() {
            self.tab()
                .session
                .update(cx, |session, _| session.tool = tool);
            self.touch(cx);
        }
    }

    /// Number keys set the active tool's opacity (Photoshop-style).
    pub fn opacity_digit(&mut self, digit: i32, window: &mut Window, cx: &mut Context<Self>) {
        let uptime = self.uptime();
        self.tab()
            .session
            .update(cx, |session, _| session.type_opacity_digit(digit, uptime));
        // Keep the panel sliders truthful (Swift's bindings follow the same
        // settings).
        use compositor_core::document::NavigationTool;
        let (tool, active_opacity, brush_opacity, gradient_opacity) = {
            let session = self.tab().session.read(cx);
            (
                session.tool,
                session.active_layer().map(|l| l.opacity).unwrap_or(1.0),
                session.brush_settings.opacity,
                session.gradient_settings.opacity,
            )
        };
        let sync: Option<(Entity<SliderState>, f32)> = match tool {
            NavigationTool::Brush
            | NavigationTool::SpotHealing
            | NavigationTool::CloneStamp
            | NavigationTool::Blur => Some((
                self.brush_sliders[2].clone(),
                (brush_opacity * 100.0) as f32,
            )),
            NavigationTool::Gradient => Some((
                self.gradient_slider.clone(),
                (gradient_opacity * 100.0) as f32,
            )),
            NavigationTool::Move => {
                Some((self.opacity_slider.clone(), (active_opacity * 100.0) as f32))
            }
            _ => None,
        };
        if let Some((slider, value)) = sync {
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
        if self.tab().session.read(cx).text_draft.is_none() {
            return;
        }
        self.tab().session.update(cx, |session, _| {
            session.change_text_style(|style| style.tracking += delta)
        });
        self.touch(cx);
    }

    /// Option-arrows step the edited text's leading, counting from whatever
    /// Auto works out to; up closes the lines up (InlineTextEditor.swift:20-22).
    pub fn adjust_text_leading(&mut self, down: bool, step: f64, cx: &mut Context<Self>) {
        if self.tab().session.read(cx).text_draft.is_none() {
            return;
        }
        self.tab().session.update(cx, |session, _| {
            session.change_text_style(|style| {
                style.leading = if down {
                    style.line_height() + step
                } else {
                    (style.line_height() - step).max(1.0)
                };
            })
        });
        self.touch(cx);
    }

    /// ⌘Return finishes the text being edited (InlineTextEditor.swift:25-27).
    pub fn finish_text_edit(&mut self, cx: &mut Context<Self>) {
        if self.tab().session.read(cx).text_draft.is_some() {
            self.tab()
                .session
                .update(cx, |session, _| session.finish_text());
            self.touch(cx);
        }
    }

    /// The raw canvas keys: Space hold-to-pan, opacity digits, Option-arrow
    /// text spacing and ⌘Return (`EditorCanvas.keyDown`). Runs on the bubble
    /// phase; keys that mean something here don't reach the action system.
    pub fn canvas_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let modifiers = &event.keystroke.modifiers;
        let bare = !(modifiers.platform || modifiers.control || modifiers.alt || modifiers.shift);
        // While the inline canvas text editor owns the keyboard, its keys
        // route first (`CanvasTextView.keyDown`): ⌘Return finishes, Escape
        // cancels, Option-arrows step the spacing, everything else types.
        // The Option-arrows normally arrive as actions instead —
        // `keys::inline_text_bindings` binds them above the text system's
        // own alt-arrow bindings and the overlay div's handlers consume
        // them — so this gate's spacing arms are a fallback that only fires
        // when no binding matched (they can never double-step: a consumed
        // keystroke never reaches this listener).
        if self.inline_text_focused(window, cx) {
            use crate::canvas::inline_text_editor::InlineTextKey;
            if let Some(action) = inline_text_key(
                key,
                modifiers.control,
                modifiers.alt,
                modifiers.platform,
                modifiers.shift,
            ) {
                match action {
                    InlineTextKey::Finish => self.commit_inline_text(cx),
                    InlineTextKey::Cancel => self.cancel_inline_text(cx),
                    InlineTextKey::Tracking(delta) => self.adjust_text_tracking(delta, cx),
                    InlineTextKey::Leading(down, step) => self.adjust_text_leading(down, step, cx),
                }
            }
            cx.stop_propagation();
            return;
        }
        if !self.canvas_keys_active(window, cx) {
            return;
        }
        // ⌘Return finishes the text draft (⌘ maps to Ctrl on Windows).
        if modifiers.control && !modifiers.alt && key == "enter" {
            self.finish_text_edit(cx);
            cx.stop_propagation();
            return;
        }
        // Space holds the Hand tool; a text draft keeps typing into the box.
        if key == "space" && bare && self.tab().session.read(cx).text_draft.is_none() {
            self.space_down(cx);
            cx.stop_propagation();
            return;
        }
        // Option with the arrows sets the text's spacing: left and right the
        // tracking, up and down the leading; Shift makes each step ten.
        if modifiers.alt && !modifiers.control && !modifiers.platform {
            let step = if modifiers.shift { 10.0 } else { 1.0 };
            match key {
                "left" => {
                    self.adjust_text_tracking(-step, cx);
                    cx.stop_propagation();
                    return;
                }
                "right" => {
                    self.adjust_text_tracking(step, cx);
                    cx.stop_propagation();
                    return;
                }
                "up" => {
                    self.adjust_text_leading(false, step, cx);
                    cx.stop_propagation();
                    return;
                }
                "down" => {
                    self.adjust_text_leading(true, step, cx);
                    cx.stop_propagation();
                    return;
                }
                _ => {}
            }
        }
        // Number keys set opacity when the tool uses it (usesOpacityKeys).
        if bare
            && let Ok(digit) = key.parse::<i32>()
            && (0..=9).contains(&digit)
        {
            self.opacity_digit(digit, window, cx);
            cx.stop_propagation();
        }
    }

    /// The raw canvas key releases: Space gives the saved tool back.
    pub fn canvas_key_up(
        &mut self,
        event: &KeyUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "space" {
            self.space_up(cx);
        }
    }

    /// Keeps the session's tool settings in sync with the sliders.
    fn subscribe_sliders(&mut self, cx: &mut Context<Self>) {
        let brush_indices = [0usize, 1, 2, 3];
        for index in brush_indices {
            let slider = self.brush_sliders[index].clone();
            let subscription =
                cx.subscribe(&slider, move |state, _slider, event: &SliderEvent, cx| {
                    let value = match event {
                        SliderEvent::Change(value) | SliderEvent::Release(value) => value.start(),
                    };
                    state.tab().session.update(cx, |session, _| {
                        let brush = &mut session.brush_settings;
                        match index {
                            0 => brush.diameter = value as f64,
                            1 => brush.hardness = (value / 100.0) as f64,
                            2 => brush.opacity = (value / 100.0) as f64,
                            _ => brush.smoothing = value as f64,
                        }
                    });
                    state.revision += 1;
                    cx.notify();
                });
            self.subscriptions.push(subscription);
        }
        let wand_slider = self.wand_slider.clone();
        let subscription = cx.subscribe(&wand_slider, |state, _slider, event: &SliderEvent, cx| {
            let value = match event {
                SliderEvent::Change(value) | SliderEvent::Release(value) => value.start(),
            };
            state.tab().session.update(cx, |session, _| {
                session.wand_settings.tolerance = value as i32
            });
            state.revision += 1;
            cx.notify();
        });
        self.subscriptions.push(subscription);
        let gradient_slider = self.gradient_slider.clone();
        let subscription = cx.subscribe(
            &gradient_slider,
            |state, _slider, event: &SliderEvent, cx| {
                let value = match event {
                    SliderEvent::Change(value) | SliderEvent::Release(value) => value.start(),
                };
                state.tab().session.update(cx, |session, _| {
                    session.gradient_settings.opacity = (value / 100.0) as f64
                });
                state.revision += 1;
                cx.notify();
            },
        );
        self.subscriptions.push(subscription);
        let opacity_slider = self.opacity_slider.clone();
        let subscription = cx.subscribe(
            &opacity_slider,
            |state, _slider, event: &SliderEvent, cx| {
                match event {
                    SliderEvent::Change(value) => {
                        state.tab().session.update(cx, |session, _| {
                            session.begin_opacity_edit();
                            session.set_layer_opacity((value.start() / 100.0) as f64);
                        });
                    }
                    SliderEvent::Release(value) => {
                        state.tab().session.update(cx, |session, _| {
                            session.set_layer_opacity((value.start() / 100.0) as f64);
                            session.finish_opacity_edit();
                        });
                    }
                }
                state.revision += 1;
                cx.notify();
            },
        );
        self.subscriptions.push(subscription);
    }

    /// `[` / `]` shrink and grow the brush, keeping the slider in sync.
    pub fn resize_brush(&mut self, factor: f64, window: &mut Window, cx: &mut Context<Self>) {
        let next = (self.tab().session.read(cx).brush_settings.diameter * factor).clamp(1.0, 300.0);
        self.tab()
            .session
            .update(cx, |session, _| session.brush_settings.diameter = next);
        let slider = self.brush_sliders[0].clone();
        slider.update(cx, |state, cx| {
            state.set_value(
                gpui_kit::component::slider::SliderValue::Single(next as f32),
                window,
                cx,
            )
        });
        self.set_status(format!("Brush size {:.0} px", next), cx);
    }

    /// Shift-[ / Shift-]: hardness in Photoshop's 25% steps (0, 25, 50, 75, 100%).
    pub fn change_brush_hardness(
        &mut self,
        increase: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tab()
            .session
            .update(cx, |session, _| session.change_brush_hardness(increase));
        let next = self.tab().session.read(cx).brush_settings.hardness;
        let slider = self.brush_sliders[1].clone();
        slider.update(cx, |state, cx| {
            state.set_value(
                gpui_kit::component::slider::SliderValue::Single((next * 100.0) as f32),
                window,
                cx,
            )
        });
        self.set_status(format!("Brush hardness {:.0}%", next * 100.0), cx);
    }

    /// Content-Aware Fill (Shift-Delete): heals the selection from surrounding
    /// pixels and commits it as one undo step, without opening a sheet.
    pub fn run_content_aware_fill(&mut self, cx: &mut Context<Self>) {
        let base = self
            .tab()
            .session
            .read(cx)
            .active_layer()
            .and_then(|layer| layer.asset.as_ref().map(|asset| asset.image.clone()));
        let Some(base) = base else {
            return;
        };
        let selection = self.selection_clip(cx);
        if self.tab().session.read(cx).selection().is_none() {
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
        let id = self.tab().session.read(cx).active_layer_id;
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

    /// Option-P with the Levels panel open: flips its Preview checkbox.
    pub fn toggle_levels_preview(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.panel_widgets.levels.as_mut() else {
            return;
        };
        panel.preview = !panel.preview;
        self.refresh_levels_preview(cx);
    }

    // MARK: Curves sheet

    /// Runs the curve editor's settings as the Curves filter preview, the
    /// same path the other filter sheets' sliders use.
    pub fn apply_curves_preview(&mut self, cx: &mut Context<Self>) {
        let mut settings = FilterSettings::default();
        if let Some(editor) = self.curves_editor.as_ref() {
            settings.curves = editor.settings.clone();
        }
        self.apply_filter_preview(FilterKind::Curves, settings, cx);
        self.touch(cx);
    }

    // MARK: Type header (TypeControls.swift)

    /// A typed change in one of the Type header's fields writes straight
    /// into the style — the draft while text is being edited, the defaults
    /// otherwise (`session.changeTextStyle`).
    pub fn text_style_field_changed(
        &mut self,
        field: TextStyleField,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        match field {
            TextStyleField::Size => {
                if let Some(value) = parse_field(text) {
                    let value = clamp_font_size(value);
                    self.tab().session.update(cx, |session, _| {
                        session.change_text_style(move |style| style.font_size = value)
                    });
                }
            }
            TextStyleField::Tracking => {
                if let Some(value) = parse_field(text) {
                    self.tab().session.update(cx, |session, _| {
                        session.change_text_style(move |style| style.tracking = value)
                    });
                }
            }
            TextStyleField::Leading => {
                let value = leading_from_text(text);
                self.tab().session.update(cx, |session, _| {
                    session.change_text_style(move |style| style.leading = value)
                });
            }
        }
        self.touch(cx);
    }

    /// Pushes the current style back into the Type header's fields (Swift's
    /// `sync`, run when a value may have moved under an unfocused field).
    pub fn sync_text_style_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let style = self.tab().session.read(cx).current_text_style();
        let fields = [
            (&self.type_size_input, format_number(style.font_size)),
            (&self.type_tracking_input, format_number(style.tracking)),
            (&self.type_leading_input, leading_to_text(style.leading)),
        ];
        for (input, text) in fields {
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            if !focused && input.read(cx).value() != text {
                input.update(cx, |state, cx| state.set_value(text, window, cx));
            }
        }
    }

    /// The Type header's color chip: opens the app color picker on the text
    /// color (`session.openTextColorPicker`). The picker is built here rather
    /// than in `color_picker_ui` because that module only knows the palette
    /// targets.
    pub fn open_text_color_picker_ui(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab().session.read(cx).color_picker.is_some() {
            return;
        }
        self.tab()
            .session
            .update(cx, |session, _| session.open_text_color_picker());
        if self.tab().session.read(cx).color_picker.is_some() {
            self.build_picker_ui(window, cx);
        }
        self.touch(cx);
    }

    /// Builds the picker widgets for whatever target the session just
    /// opened (`open_color_picker_ui`'s widget half).
    fn build_picker_ui(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((title, hex)) = self
            .tab()
            .session
            .read(cx)
            .color_picker
            .as_ref()
            .map(|picker| (picker.target.title(), picker.color().hex()))
        else {
            return;
        };
        // The text color's picker lives in the same floating panel as the
        // palette one (ColorPickerSheet.swift:170-182).
        let hex = cx.new(|cx| InputState::new(window, cx).default_value(hex));
        self.picker_ui = Some(crate::ui::color_picker_sheet::PickerUi { hex });
        self.panels.show(
            crate::ui::floating_panel::panel_names::COLOR_PICKER_PANEL,
            title,
            crate::ui::floating_panel::PanelPlacement::Automatic,
        );
        self.sync_panel_guard();
    }

    // MARK: Transform inspector (TransformInspector.swift)

    /// The transform the inspector's fields show: the draft while an edit is
    /// open, else the active layer's (or its unlinked mask's) transform
    /// (`session.editedTransform`).
    pub fn transform_displayed(&self, cx: &App) -> Option<LayerTransform> {
        let session = self.current_tab().session.read(cx);
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
    pub fn transform_pixel_size(&self, cx: &App) -> CGSize {
        if let Some(layer) = self.current_tab().session.read(cx).active_layer()
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
    pub fn transform_change(
        &mut self,
        update: impl FnOnce(&mut LayerTransform),
        cx: &mut Context<Self>,
    ) {
        self.tab().session.update(cx, |session, _| {
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

    /// A typed change in one of the inspector's fields
    /// (`TransformInspector`'s `field`/`resize`/scale closures).
    pub fn transform_field_changed(
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
                let lock = self.tab().session.read(cx).locks_transform_ratio;
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

    /// Pushes the displayed transform back into the inspector's fields
    /// (`TransformValueField.sync`).
    pub fn sync_transform_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    /// Wiring for the header's numeric fields: typing writes straight into
    /// the session (Swift's live `onChange(of: text)`).
    fn subscribe_inputs(&mut self, cx: &mut Context<Self>) {
        let text_fields = [
            (self.type_size_input.clone(), TextStyleField::Size),
            (self.type_tracking_input.clone(), TextStyleField::Tracking),
            (self.type_leading_input.clone(), TextStyleField::Leading),
        ];
        for (input, field) in text_fields {
            let watched = input.clone();
            let subscription = cx.subscribe(
                &input,
                move |state: &mut Self, _input, event: &InputEvent, cx: &mut Context<Self>| {
                    if let InputEvent::Change = event {
                        let text = watched.read(cx).value().to_string();
                        state.text_style_field_changed(field, &text, cx);
                    }
                },
            );
            self.subscriptions.push(subscription);
        }
        // The inline canvas text editor's two-way binding: typing writes
        // into the draft (`textDidChange`).
        let inline = self.inline_text_input.clone();
        let subscription = cx.subscribe(
            &inline,
            move |state: &mut Self, _input, event: &InputEvent, cx: &mut Context<Self>| {
                if let InputEvent::Change = event {
                    let text = state.inline_text_input.read(cx).value().to_string();
                    state.inline_text_changed(&text, cx);
                }
            },
        );
        self.subscriptions.push(subscription);
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
                move |state: &mut Self, _input, event: &InputEvent, cx: &mut Context<Self>| {
                    if let InputEvent::Change = event {
                        let text = watched.read(cx).value().to_string();
                        state.transform_field_changed(field, &text, cx);
                    }
                },
            );
            self.subscriptions.push(subscription);
        }
    }

    /// Every mutation funnels through here so the frame image rebuilds.
    pub fn touch(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        cx.notify();
    }

    /// (Re)attaches the window's observer to the active tab's session
    /// entity (ADR 0001): while a session entity itself notifies, the
    /// window re-renders. Idempotent — call after any workspace flow that
    /// may have changed the active tab; the outgoing tab's observation is
    /// dropped by the replacement, so non-active tabs are not observed
    /// (notification-scope refinement beyond the active tab is ticket 37).
    pub(crate) fn sync_session_observer(&mut self, cx: &mut Context<Self>) {
        let session = self.workspace.current().session.clone();
        if self
            .session_observer
            .as_ref()
            .is_some_and(|(observed, _)| observed.entity_id() == session.entity_id())
        {
            return;
        }
        let subscription = cx.observe(&session, |_, _, cx| cx.notify());
        self.session_observer = Some((session, subscription));
    }

    pub fn notify_only(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    /// The composite for canvas display: mask resamples cap at 2048 pixels
    /// while a transform edit drives the placements (`clipImage`'s
    /// interactive `limit`).
    pub fn composite_bitmap(&self, document: &CanvasDocument, cx: &App) -> Bitmap {
        if self.current_tab().session.read(cx).transform_edit.is_some() {
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
        let Some(document) = self.tab().session.read(cx).document.clone() else {
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
        self.current_tab()
            .session
            .read(cx)
            .document
            .as_ref()
            .map(|d| (d.width as f64, d.height as f64))
            .unwrap_or((0.0, 0.0))
    }

    pub fn doc_to_screen(&self, x: f64, y: f64) -> (f32, f32) {
        doc_to_screen_at(self.zoom, self.pan, x, y)
    }

    pub fn screen_to_doc(&self, x: f32, y: f32) -> CGPoint {
        screen_to_doc_at(self.zoom, self.pan, x, y)
    }

    /// The document rect in viewport pixels.
    pub fn image_rect(&self, cx: &App) -> Bounds<Pixels> {
        let (w, h) = self.doc_size(cx);
        let (x, y) = self.doc_to_screen(0.0, 0.0);
        Bounds::<Pixels>::new(
            Point::new(px(x), px(y)),
            gpui_kit::size(px((w * self.zoom) as f32), px((h * self.zoom) as f32)),
        )
    }

    pub fn zoom_at(&mut self, screen: (f32, f32), factor: f64) {
        let doc = self.screen_to_doc(screen.0, screen.1);
        self.zoom = clamp_zoom(self.zoom * factor);
        self.pan = (
            screen.0 as f64 - doc.x * self.zoom,
            screen.1 as f64 - doc.y * self.zoom,
        );
        self.revision += 1;
    }

    pub fn zoom_fit(&mut self, viewport: (f32, f32), cx: &App) {
        let (w, h) = self.doc_size(cx);
        if w <= 0.0 || h <= 0.0 || viewport.0 <= 0.0 || viewport.1 <= 0.0 {
            return;
        }
        let margin = 48.0f64;
        let zoom = ((viewport.0 as f64 - margin) / w)
            .min((viewport.1 as f64 - margin) / h)
            .clamp(MIN_ZOOM, 1.0);
        self.zoom = zoom;
        self.pan = (
            (viewport.0 as f64 - w * zoom) / 2.0,
            (viewport.1 as f64 - h * zoom) / 2.0,
        );
        self.fit_for = FitDims {
            width: w as usize,
            height: h as usize,
        };
        self.fit_tab = self.workspace.selected_id();
        self.revision += 1;
    }

    /// Requests a viewport refit on the next canvas layout pass (Swift's
    /// view rebuild on a tab switch or a document size change).
    pub fn request_refit(&mut self) {
        self.fit_for = FitDims::REFIT;
        self.fit_tab = crate::project::TabId::nil();
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
    /// preview standing in for the active layer's pixels.
    pub fn composite(&self, cx: &App) -> Option<Bitmap> {
        let session = self.current_tab().session.read(cx);
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
        if let (Some(preview), Some(base_transform)) = (&self.preview, &self.preview_base_transform)
        {
            let mut doc = document.clone();
            override_pixels(&mut doc, preview, *base_transform);
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
        if let Some(CanvasDrag::Stroke(edit, _)) = &self.drag {
            let result = edit.result();
            let mut doc = document.clone();
            override_pixels(&mut doc, &result, edit.paint_transform);
            return Some(self.composite_bitmap(&doc, cx));
        }
        // A Smudge / Liquify stroke shows its document-size working copy in
        // place of the layer, exactly as the Swift canvas does.
        if let Some(CanvasDrag::Warp(stroke)) = &self.drag {
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
        if let Some(CanvasDrag::Shape { .. }) = &self.drag
            && let (Some(shape), Some(rect)) = (&self.shape_shape, &self.shape_rect)
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
        if let Some(CanvasDrag::Gradient(edit, start, _)) = &self.drag {
            let result = self.gradient_result(edit, *start, cx);
            let mut doc = document.clone();
            override_pixels(&mut doc, &result, edit.paint_transform);
            return Some(self.composite_bitmap(&doc, cx));
        }
        Some(render::draw_live_composite(document))
    }

    pub(crate) fn gradient_result(&self, edit: &RasterEdit, start: CGPoint, cx: &App) -> Bitmap {
        let session = self.current_tab().session.read(cx);
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
        let end = self.gradient_cursor.unwrap_or(start);
        edit.fill_gradient(settings.shape, start, end, colors, settings.opacity)
    }

    /// The cached BGRA texture for the composite, rebuilt when the tab or the
    /// revision changes (cache key `(tab_id, revision)` — design doc §1).
    pub fn frame_image(&mut self, composite: &Bitmap) -> Arc<RenderImage> {
        let tab_id = self.workspace.selected_id();
        if self.frame_image.is_none()
            || self.frame_image_rev != self.revision
            || self.frame_image_tab != tab_id
        {
            self.frame_image = Some(bgra_render_image(composite));
            self.frame_image_rev = self.revision;
            self.frame_image_tab = tab_id;
        }
        self.frame_image.clone().unwrap()
    }

    /// The cached thumbnail texture for one layer row, keyed by layer and by
    /// whether the mask is the row's subject (the mask thumbnail swaps in).
    pub fn thumbnail_image(
        &mut self,
        id: uuid::Uuid,
        layer: &ImageLayer,
        cx: &App,
    ) -> Option<Arc<RenderImage>> {
        let mask_selected = self.tab().session.read(cx).is_mask_selected && layer.mask.is_some();
        if let Some((rev, image)) = self.thumbnails.get(&(id, mask_selected))
            && *rev == self.revision
        {
            return Some(image.clone());
        }
        let document = self.tab().session.read(cx).document.as_ref()?;
        let canvas =
            compositor_core::geometry::CGSize::new(document.width as f64, document.height as f64);
        // CanvasThumbnail: the pixels (or the mask) sit on a canvas-shaped
        // frame, whatever the layer's own bounds.
        let thumbnail = if mask_selected {
            render::mask_canvas_thumbnail(layer.mask.as_ref()?, &layer.mask_transform(), canvas)?
        } else {
            render::layer_canvas_thumbnail(layer, canvas)?
        };
        let image = bgra_render_image(&thumbnail);
        self.thumbnails
            .insert((id, mask_selected), (self.revision, image.clone()));
        Some(image)
    }

    // MARK: Status

    pub fn set_status(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.status = Some(message.into());
        self.touch(cx);
    }

    // MARK: Rename

    pub fn begin_rename(&mut self, id: uuid::Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let current = self
            .tab()
            .session
            .read(cx)
            .document
            .as_ref()
            .and_then(|d| d.layer(id))
            .map(|l| l.name.clone())
            .unwrap_or_default();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
        let subscription = cx.subscribe(
            &input,
            |this: &mut Self,
             _input,
             event: &gpui_kit::component::input::InputEvent,
             cx: &mut Context<Self>| {
                match event {
                    gpui_kit::component::input::InputEvent::PressEnter { .. }
                    | gpui_kit::component::input::InputEvent::Blur => this.commit_rename(cx),
                    _ => {}
                }
            },
        );
        self.subscriptions.push(subscription);
        self.tab()
            .session
            .update(cx, |session, _| session.renaming_layer_id = Some(id));
        self.rename_input = Some(input);
        self.notify_only(cx);
    }

    pub fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let id = self.tab().session.read(cx).renaming_layer_id;
        if let (Some(id), Some(input)) = (id, self.rename_input.clone()) {
            let text = input.read(cx).value().to_string();
            self.tab().session.update(cx, |session, _| {
                session.rename_layer(id, &text);
                session.renaming_layer_id = None;
            });
        }
        self.rename_input = None;
        self.touch(cx);
    }

    #[allow(dead_code)]
    pub fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        self.tab()
            .session
            .update(cx, |session, _| session.renaming_layer_id = None);
        self.rename_input = None;
        self.notify_only(cx);
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
        self.tab().session.update(cx, |session, _| {
            session
                .history
                .begin(name, &session.document, session.active_layer_id);
            edit(session);
            session
                .history
                .end(&session.document, session.active_layer_id);
        });
    }

    pub fn close_sheet(&mut self, cx: &mut Context<Self>) {
        self.sheet = None;
        self.curves_editor = None;
        self.preview = None;
        self.preview_base = None;
        self.preview_base_transform = None;
        self.gradient_cursor = None;
        // The filter panel session is over: the next one draws a fresh noise
        // pattern (FilterEdit's `let seed = UInt32.random` at creation).
        self.filter_seed = time_seed();
        self.delete_ask_ids = None;
        self.touch(cx);
    }

    /// Commits the preview into the active layer's pixels as one undo step,
    /// then drops the preview (the canvas shows the layer's own pixels — now
    /// the committed ones). The sheet/panel caller closes its own UI after.
    pub fn commit_preview_pixels(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Some(preview) = self.preview.clone() {
            let transform = self.preview_base_transform.unwrap_or_default();
            self.history_edit(
                name,
                move |session| {
                    if let Some(id) = session.active_layer_id
                        && let Some(doc) = session.document.as_mut()
                        && let Some(layer) = doc.layer_mut(id)
                    {
                        layer.asset = Some(ImportedImage::new(
                            preview.as_ref().clone(),
                            layer.name.clone(),
                        ));
                        layer.transform = transform;
                    }
                },
                cx,
            );
        }
        self.preview = None;
        self.preview_base = None;
        self.preview_base_transform = None;
        // A committed filter closes its panel session: a fresh seed next time.
        self.filter_seed = time_seed();
        self.touch(cx);
    }

    /// The active selection clipped to the canvas (`selection?.clip(canvas:)`).
    pub(crate) fn selection_clip(
        &self,
        cx: &App,
    ) -> Option<compositor_core::selection::SelectionClip> {
        let session = self.current_tab().session.read(cx);
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
        let session = self.current_tab().session.read(cx);
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
        let session = self.current_tab().session.read(cx);
        match session.document.as_ref() {
            Some(document) => delete_candidate_ids(document, &session.selected_layer_ids),
            None => Vec::new(),
        }
    }

    /// Runs `filter` over the preview base and shows it live on canvas.
    pub fn apply_filter_preview(
        &mut self,
        kind: FilterKind,
        settings: FilterSettings,
        cx: &mut Context<Self>,
    ) {
        // Editing an adjustment layer: the settings go to the session's
        // filter editor and the layer previews through the live composite.
        if self.tab().session.read(cx).adjustment_editing_id.is_some() {
            self.tab().session.update(cx, |session, _| {
                session.filter_edit = Some(settings);
                session.preview_adjustment_editing(true);
            });
            return;
        }
        let Some(base) = self.preview_base.clone() else {
            return;
        };
        let selection = self.selection_clip(cx);
        let job = FilterJob::builder(kind, &base, settings)
            .with_selection(selection)
            // The seed was drawn when the panel opened and is reused for the
            // committed pixels, so the preview is exactly what lands
            // (FilterEdit.seed, Filters.swift:223).
            .with_seed(self.filter_seed)
            .build();
        let result = compositor_core::filters::run_filter(&job);
        self.preview = Some(Arc::new(result));
        self.revision += 1;
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
        let mut settings = self.filter_settings_from_panel(cx);
        compositor_core::adjust::camera_raw::apply_defringe_sample(
            &mut settings.camera_raw.optics,
            hue,
        );
        self.camera_raw_samples_defringe = false;
        self.apply_filter_preview(FilterKind::CameraRaw, settings, cx);
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
        let mut settings = self.filter_settings_from_panel(cx);
        settings.camera_raw.white_balance = CameraRawWhiteBalance::Auto;
        if let Some((temperature, tint)) =
            compositor_core::adjust::camera_raw::CameraRawSettings::auto_balance(&base)
        {
            settings.camera_raw.temperature = temperature;
            settings.camera_raw.tint = tint;
        }
        self.apply_filter_preview(FilterKind::CameraRaw, settings, cx);
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
        MAX_ZOOM, MIN_ZOOM, TransformField, byte_count_label, clamp_font_size, clamp_zoom,
        doc_to_screen_at, format_number, leading_from_text, leading_to_text, normalize_rotation,
        parse_field, resize_transform, screen_to_doc_at, transform_field_texts,
    };
    use compositor_core::geometry::{CGPoint, CGSize};
    use compositor_core::transform::LayerTransform;

    #[test]
    fn doc_to_screen_applies_zoom_and_pan() {
        // A 100x100 document at 2x zoom, panned so its origin sits at (40, 30).
        let (sx, sy) = doc_to_screen_at(2.0, (40.0, 30.0), 10.0, 5.0);
        assert_eq!((sx, sy), (60.0, 40.0));
        // Identity zoom and zero pan map document coordinates unchanged.
        assert_eq!(doc_to_screen_at(1.0, (0.0, 0.0), 12.5, 3.25), (12.5, 3.25));
    }

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
    fn screen_doc_roundtrip_recovers_document_point() {
        let zoom = 0.35;
        let pan = (-123.0, 456.5);
        let (sx, sy) = doc_to_screen_at(zoom, pan, 88.0, -21.0);
        let doc = screen_to_doc_at(zoom, pan, sx, sy);
        // Screen coordinates are f32, so the roundtrip loses some precision.
        assert!((doc.x - 88.0).abs() < 1e-3, "x = {}", doc.x);
        assert!((doc.y - -21.0).abs() < 1e-3, "y = {}", doc.y);
    }

    #[test]
    fn clamp_zoom_keeps_zoom_in_range() {
        assert_eq!(clamp_zoom(1.0), 1.0);
        assert_eq!(clamp_zoom(0.000_001), MIN_ZOOM);
        assert_eq!(clamp_zoom(1_000.0), MAX_ZOOM);
        assert_eq!(clamp_zoom(2.5), 2.5);
    }

    #[test]
    fn screen_doc_roundtrip_holds_at_zoom_extremes() {
        for zoom in [MIN_ZOOM, 0.1, 1.0, 8.0, MAX_ZOOM] {
            let pan = (37.5, -18.25);
            let (sx, sy) = doc_to_screen_at(zoom, pan, 250.0, 90.0);
            let doc = screen_to_doc_at(zoom, pan, sx, sy);
            assert!((doc.x - 250.0).abs() < 1e-2, "zoom {zoom}: x = {}", doc.x);
            assert!((doc.y - 90.0).abs() < 1e-2, "zoom {zoom}: y = {}", doc.y);
        }
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
}
