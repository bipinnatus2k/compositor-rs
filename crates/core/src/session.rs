//! Port of the session layer: `EditorSession` and its extensions
//! (`EditorSession.swift`, `Selection.swift`, `LayerAppearance.swift`,
//! `LayerMask.swift`, `LayerGroups.swift`, `LayerMerge.swift`,
//! `LayerFlip.swift`, `DocumentHistory` integration, `Crop.swift`,
//! `CanvasSize`/`ImageSize`/`Trim` operations).

use crate::blend::LayerBlendMode;
use crate::canvas::{
    CanvasSizeOptions, CropDrag, CropDragMode, CropGeometry, TrimOptions, mirrored_transform,
    resize_canvas, resize_image, trim as trim_document,
};
use crate::canvas_viewport::CanvasViewport;
use crate::document::PickerHSB;
use crate::document::{
    CanvasDocument, CanvasGuide, GuideAxis, ImageLayer, LayerMask, LayerTextStyle, NavigationTool,
    PaletteColor,
};
use crate::geometry::{CGAffineTransform, CGFloat, CGPoint, CGRect, CGSize};
use crate::history::DocumentHistory;
use crate::raster::{Bitmap, ImportedImage};
use crate::selection::{
    DocumentSelection, LassoDraft, LassoKind, SelectionMode, SelectionPath, drag_box_rect,
};
use crate::tools::brush::RasterEdit;
use crate::transform::{LayerSampling, LayerTransform, TransformEdit, TransformGroup};
use uuid::Uuid;
// Split impls (verbatim moves; one file per Swift extension boundary).
mod adjustment_editing;
mod brush;
pub use brush::LastBrushPoint;
mod crop;
mod effects;
mod gradient;
mod history;
mod layer_groups;
mod mask;
mod projects;
mod selection;
mod text;

/// The Blur tool's modes (`BlurToolMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlurToolMode {
    #[default]
    Liquify,
    Blur,
    Smudge,
}

/// The Brush's two modes (B and E).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrushToolMode {
    #[default]
    Paint,
    Erase,
}

/// Spot Healing's modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpotHealingMode {
    #[default]
    ContentAware,
    CreateTexture,
    ProximityMatch,
}

/// The Magic tool's settings (`WandSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WandSettings {
    /// How far (0–255) each channel may differ from the sampled color.
    pub tolerance: i32,
    /// Pixels either side of the click averaged into the color to match.
    pub sample_radius: usize,
    /// Only similar pixels connected to the clicked one.
    pub contiguous: bool,
    /// Read the visible composite rather than just the active layer.
    pub sample_all_layers: bool,
}

impl Default for WandSettings {
    fn default() -> Self {
        WandSettings {
            tolerance: 32,
            sample_radius: 0,
            contiguous: true,
            sample_all_layers: false,
        }
    }
}

/// Clone Stamp settings (`CloneSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloneSettings {
    /// Aligned strokes keep the offset from brush to source.
    pub aligned: bool,
    /// Copy from every visible layer as shown.
    pub sample_all_layers: bool,
}

impl Default for CloneSettings {
    fn default() -> Self {
        CloneSettings {
            aligned: true,
            sample_all_layers: false,
        }
    }
}

/// Gradient tool settings (`GradientSettings`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientSettings {
    pub shape: crate::tools::brush::GradientShape,
    pub foreground_to_background: bool,
    pub reversed: bool,
    pub opacity: f64,
}

impl Default for GradientSettings {
    fn default() -> Self {
        GradientSettings {
            shape: crate::tools::brush::GradientShape::Linear,
            foreground_to_background: false,
            reversed: false,
            opacity: 1.0,
        }
    }
}

/// `Select > Modify` operations that ask for an amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionAmountOperation {
    Expand,
    Contract,
    Feather,
}

/// The session: the open document plus every tool state. The Swift class is
/// Text being typed (`TextDraft`): which layer it edits, where it sits, and
/// the style it will carry.
#[derive(Clone)]
pub struct TextDraft {
    pub document_id: Uuid,
    pub layer_id: Option<Uuid>,
    pub origin: CGPoint,
    pub transform: Option<LayerTransform>,
    pub style: crate::document::LayerTextStyle,
}

/// A guide being dragged (`GuideDrag`): `isNew` guides vanish when the drag
/// ends on a ruler; existing guides move or delete.
#[derive(Clone)]
pub struct GuideDrag {
    pub id: Uuid,
    pub axis: GuideAxis,
    pub position: f64,
    pub is_new: bool,
    pub original: Option<f64>,
}

/// A pending gradient (`GradientEdit`): endpoints are document pixels; the
/// raster is rebuilt on every change and never touches the document until it
/// commits.
#[derive(Clone)]
pub struct GradientDraft {
    pub layer_id: Uuid,
    pub is_mask: bool,
    pub start: CGPoint,
    pub end: CGPoint,
}

/// Pixels copied from the canvas, with where they came from so Paste can put
/// them back in place (`PixelClipboard`).
#[derive(Clone)]
pub struct PixelClipboard {
    pub image: Bitmap,
    pub origin: CGPoint,
}

/// Selected pixels being dragged (`PixelMove` in SelectionEdits.swift): the
/// lifted raster plus the outline it started from. This is the Cmd-drag
/// temporary Move — a raster-level lift that previews by rebuilding the
/// layer's grid (hole at the origin, pixels at the offset), not the floating
/// "Floating Selection" layer that Cmd-T's `begin_selection_transform` opens
/// for full affine handles.
pub struct PixelMove {
    /// The lifted raster (`PixelMove.raster`).
    pub raster: RasterEdit,
    /// The selection the move started from (`PixelMove.origin`).
    pub origin: DocumentSelection,
    /// Option-drag duplicates instead of moving (`PixelMove.duplicate`).
    pub duplicate: bool,
    /// How far the pixels sit from where they started, whole document
    /// pixels (`PixelMove.offset`).
    pub offset: CGSize,
}

impl PixelMove {
    /// `movedSelection` — the original outline shifted by the drag.
    pub fn moved_selection(&self) -> DocumentSelection {
        DocumentSelection {
            path: self
                .origin
                .path
                .translated(self.offset.width, self.offset.height),
            antialiased: self.origin.antialiased,
            feather: self.origin.feather,
        }
    }

    /// The layer grid with the drag applied (`moveLifted(by:duplicate:)`):
    /// the selection becomes a transparent hole and the lifted pixels land at
    /// the offset. Pure — Swift's preview is the same rebuild each drag event.
    pub fn moved_pixels(&self) -> Option<Bitmap> {
        self.raster.move_lifted(self.offset, self.duplicate)
    }
}

/// `ColorPickerTarget` — what the open color picker edits
/// (ColorPalette.swift:162-180). The Gradient Map variant has no core
/// filter-edit session to attach to yet, so nothing opens it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorPickerTarget {
    Palette {
        background: bool,
    },
    /// A layer effect's own color.
    Effect {
        kind: crate::effects::LayerEffectKind,
    },
    GradientMap {
        highlights: bool,
    },
    Text {
        draft_id: Option<Uuid>,
    },
}

impl ColorPickerTarget {
    /// `var title` (ColorPalette.swift:169-178).
    pub fn title(&self) -> String {
        match self {
            ColorPickerTarget::Text { .. } => "Color Picker (Text Color)".to_string(),
            ColorPickerTarget::Effect { kind } => {
                format!("Color Picker ({} Color)", kind_label(*kind))
            }
            ColorPickerTarget::Palette { background } => format!(
                "Color Picker ({} Color)",
                if *background {
                    "Background"
                } else {
                    "Foreground"
                }
            ),
            ColorPickerTarget::GradientMap { highlights } => format!(
                "Color Picker (Gradient Map {})",
                if *highlights { "Highlights" } else { "Shadows" }
            ),
        }
    }
}

fn kind_label(kind: crate::effects::LayerEffectKind) -> &'static str {
    use crate::effects::LayerEffectKind::*;
    match kind {
        Stroke => "Stroke",
        Shadow => "Drop Shadow",
        ColorOverlay => "Color Overlay",
        InnerShadow => "Inner Shadow",
        OuterGlow => "Outer Glow",
        InnerGlow => "Inner Glow",
    }
}

/// `mergePlan`'s result: the layers that merge (bottom to top), everything the
/// merge removes, the result's name, parent, where it lands, and the undo name.
struct MergePlan {
    ids: Vec<Uuid>,
    removed: std::collections::HashSet<Uuid>,
    name: String,
    parent: Option<Uuid>,
    anchor: Uuid,
    action: &'static str,
}

/// `LayerHierarchy.validate` over live layers (LayerGroups.swift:28-44):
/// unique ids, groups carry no pixels, parents exist and are groups, and no
/// nesting or cycle beyond 64 ancestor levels.
fn hierarchy_is_valid(layers: &[ImageLayer]) -> bool {
    let mut by_id: std::collections::HashMap<Uuid, &ImageLayer> = std::collections::HashMap::new();
    for layer in layers {
        if by_id.insert(layer.id, layer).is_some() {
            return false;
        }
        if layer.is_group && layer.asset.is_some() {
            return false;
        }
    }
    for layer in layers {
        let mut seen: std::collections::HashSet<Uuid> = std::collections::HashSet::from([layer.id]);
        let mut parent = layer.parent_id;
        while let Some(id) = parent {
            if seen.len() > 64 || !seen.insert(id) {
                return false;
            }
            let Some(node) = by_id.get(&id) else {
                return false;
            };
            if !node.is_group {
                return false;
            }
            parent = node.parent_id;
        }
        if layer.is_group && seen.len() > 64 {
            return false;
        }
    }
    true
}

/// `ColorPickerState` — the open picker's working color. Nothing is written
/// to the palette until OK; `hsb` is the source of truth so hue survives
/// dragging through grays and black (ColorPalette.swift:182-196).
#[derive(Debug, Clone, PartialEq)]
pub struct ColorPickerState {
    pub target: ColorPickerTarget,
    pub original: PaletteColor,
    pub hsb: PickerHSB,
}

impl ColorPickerState {
    pub fn new(target: ColorPickerTarget, original: PaletteColor) -> Self {
        ColorPickerState {
            target,
            original,
            hsb: PickerHSB::from_rgb(original),
        }
    }

    pub fn background(&self) -> bool {
        self.target == ColorPickerTarget::Palette { background: true }
    }

    /// `var color` — the quantized working color.
    pub fn color(&self) -> PaletteColor {
        self.hsb.rgb().quantized()
    }
}

/// `transformDuplicate` (EditorSession.swift) — an Option-drag duplicate in
/// progress: the copies being transformed, the selection they were duplicated
/// from, and the primary selection then.
#[derive(Debug, Clone)]
pub struct TransformDuplicate {
    pub copies: Vec<Uuid>,
    pub source: Vec<Uuid>,
    pub primary: Option<Uuid>,
}

/// `@Observable`; here the UI reads the fields directly and mutation methods
/// keep the same guards and undo names.
pub struct EditorSession {
    pub document: Option<CanvasDocument>,
    pub tool: NavigationTool,
    pub active_layer_id: Option<Uuid>,
    pub selected_layer_ids: Vec<Uuid>,
    pub is_mask_selected: bool,
    /// The layer being renamed in place (`renamingLayerID`,
    /// EditorSession.swift:560, stage-D ticket 35). The edit field itself is
    /// app-side widget state with no Swift counterpart.
    pub renaming_layer_id: Option<Uuid>,
    pub collapsed_group_ids: std::collections::HashSet<Uuid>,
    pub history: DocumentHistory,
    /// The canvas view transform (`viewport`, EditorSession.swift:170,
    /// `Rendering/CanvasViewport.swift`): per-tab zoom/pan that survives tab
    /// switches; `followsFit` keeps refitting until the user zooms or pans.
    pub viewport: CanvasViewport,
    /// An Option-drag duplicate in progress (`transformDuplicate`): the
    /// copies, the selection they came from, and the primary then.
    pub transform_duplicate: Option<TransformDuplicate>,

    /// The `.comp` package this project was loaded from or last saved to
    /// (`projectURL`, EditorSession.swift:114). Written by the app's project
    /// controller on save/open; titles and open dedup read it.
    pub project_url: Option<std::path::PathBuf>,

    // Project-operation busy state (`isProjectBusy` EditorSession.swift:118,
    // `isImporting` :515, `brushStroke`/`warpStroke`), read by
    // `can_start_project_operation` and the other Swift guards that nil-test
    // the same names. They lived app-side (`tabs.rs`'s `ProjectBusy`) until
    // stage-D ticket 34 moved them in. Only `is_project_busy` has guarded
    // mutation in Swift (ProjectController.begin / the per-flow `defer`) —
    // those are `begin_project_access`/`end_project_access` below; the
    // other two are plain property writes in Swift and stay plain fields
    // here.
    /// `isProjectBusy`: a project operation (save/export/open/canvas size)
    /// is running. `@ObservationIgnored` in Swift (:118) — controls dim via
    /// `showsBusy` after the indicator delay; the port has no busy
    /// indicator yet.
    pub is_project_busy: bool,
    /// `isImporting`: an image import is in flight. In Swift it brackets the
    /// `importImages`/`drainImports` decode window (:726-755); the port's
    /// decode is synchronous, so the app holds it across the file prompt
    /// instead (the observable window Swift's importer flow has).
    pub is_importing: bool,
    /// A paint/warp stroke is in flight (`brushStroke != nil` and
    /// `warpStroke != nil` — every guard nil-tests the two together, so the
    /// port folds them into one flag). The strokes themselves stay
    /// canvas-side: `editor_canvas.rs`'s `set_drag` mirrors its drag kinds
    /// into this flag.
    pub has_brush_stroke: bool,

    // Appearance preview
    pub blend_preview: Option<(Uuid, LayerBlendMode)>,
    pub opacity_edit_layer_id: Option<Uuid>,

    // Effects panel editing (LayerEffects.swift:110-112); methods in effects_session.rs
    pub effects_editing: Option<crate::effects::LayerEffectSelection>,
    pub effects_editing_original: Option<crate::effects::LayerEffects>,
    pub effect_selection: Option<crate::effects::LayerEffectSelection>,

    // The open color picker (ColorPalette.swift:162-192); methods in color_picker.rs
    pub color_picker: Option<ColorPickerState>,

    /// Photoshop-style opacity keys: the first digit waits 0.6 s for a second
    /// one (EditorSession+Brush.swift:186). `(digit, uptime)`.
    pub pending_opacity_digit: Option<(i32, f64)>,

    // Painting
    pub brush_settings: crate::tools::brush::BrushSettings,
    pub brush_mode: BrushToolMode,
    pub spot_healing_mode: SpotHealingMode,
    pub blur_mode: BlurToolMode,
    pub wand_mode: crate::selection::WandMode,
    pub wand_settings: WandSettings,
    pub clone_settings: CloneSettings,
    pub clone_source: Option<CGPoint>,
    pub clone_offset: Option<CGSize>,
    pub mask_paint_white: bool,
    pub background_color: PaletteColor,
    /// Where the last stroke ended, with the layer/mask it painted on — the
    /// anchor a Shift-click draws a straight line from (`lastBrushPoint`,
    /// EditorSession+Brush.swift:61/70).
    pub last_brush_point: Option<LastBrushPoint>,
    pub gradient_settings: GradientSettings,

    // Selection editing
    pub lasso_draft: Option<LassoDraft>,
    pub lasso_kind: LassoKind,
    pub marquee_kind: LassoKind,
    pub selection_mode_choice: SelectionMode,
    pub selection_antialiased: bool,
    pub selection_feather_amount: i64,
    pub selection_expand_amount: i64,
    pub selection_contract_amount: i64,
    pub selection_amount_operation: Option<SelectionAmountOperation>,
    pub selection_move_origin: Option<DocumentSelection>,
    /// Selected pixels being dragged (`pixelMove`); see `begin_pixel_move`.
    pub pixel_move: Option<PixelMove>,

    // Clipboard (`pixelClipboard`)
    pub pixel_clipboard: Option<PixelClipboard>,

    // Text being typed (`textDraft`)
    pub text_draft: Option<TextDraft>,
    pub canvas_focus_request: u64,

    // Pending gradient (`gradientEdit`)
    pub gradient_edit: Option<GradientDraft>,

    // Guide being dragged (`guideDrag`)
    pub guide_drag: Option<GuideDrag>,

    // Transform
    pub transform_edit: Option<TransformEdit>,
    pub locks_transform_ratio: bool,
    pub snapping_enabled: bool,
    /// The guide lines the last snap landed on, for the canvas to draw.
    pub snap_guides: (Vec<CGFloat>, Vec<CGFloat>),

    // Crop
    pub crop_rect: Option<CGRect>,
    pub crop_ratio_choice: String,
    pub crop_drag: Option<CropDrag>,

    // Sheets & import flags (the UI reads these to present its dialogs).
    // `showsImporter`/`importError`/`showsConversionSheet` moved in with
    // stage-D ticket 35. Nothing writes them yet: the importer sheet and
    // the PSD conversion sheet are not ported (images run through the
    // native file dialog), and the failures Swift feeds `importError`
    // surface through the app's own channels instead — the status bar for
    // receives, the workspace alert queue for layer copies (Swift clears
    // the field when the alert's OK closes; a write without that alert
    // would latch the guards below). They are still counted here,
    // condition for condition with Swift :151.
    pub shows_new_document: bool,
    /// `showsImporter` (EditorSession.swift:514).
    pub shows_importer: bool,
    /// `importError` (:516): the last import's failure line(s) for the UI
    /// to surface.
    pub import_error: Option<String>,
    /// `showsConversionSheet` (:517): the PSD conversion-notes ask is up.
    pub shows_conversion_sheet: bool,
    pub levels: Option<crate::adjust::LevelsSettings>,
    pub levels_layer_id: Option<Uuid>,
    pub hue_saturation: Option<crate::adjust::HueSaturationSettings>,
    pub hue_saturation_layer_id: Option<Uuid>,

    // Adjustment layer editing (AdjustmentEditing.swift); methods below
    // `update_adjustment`. The ordinary editors send their changes to the
    // edited layer's metadata instead of pixels, and the canvas renders every
    // preview live through the layer's own adjustment.
    /// The layer being edited (`adjustmentEditingID`).
    pub adjustment_editing_id: Option<Uuid>,
    /// The edited adjustment's opening value (`adjustmentOriginal`): Cancel
    /// restores it, OK commits the editors' settings over it.
    pub adjustment_original: Option<crate::adjustment::LayerAdjustment>,
    /// The filter panel's live settings while an adjustment layer is being
    /// edited (`filterEdit.settings`); Levels and Hue/Saturation use the
    /// fields above.
    pub filter_edit: Option<crate::filters::FilterSettings>,

    // Guides
    /// The Move tool picks the layer under the pointer instead of dragging
    /// the active layer (`transformAutoSelect`); Cmd picks either way. The
    /// canvas press reads it; ToolDefaults persists it.
    pub transform_auto_select: bool,
    pub shows_transform_controls: bool,
    pub shows_guides: bool,
    pub shows_rulers: bool,
    pub shows_grid: bool,
    pub shows_pixel_grid: bool,
    pub snap_to_guides: bool,
    pub snap_to_grid: bool,
    pub snap_to_layers: bool,
    pub snap_to_document_bounds: bool,
    pub locks_guides: bool,

    // Text
    pub text_defaults: LayerTextStyle,

    // Shape tool
    pub shape_kind: crate::document::ShapeKind,
    /// Corner radius in pixels for rectangles the Shape tool draws.
    pub shape_corner_radius: f64,
    /// A Line shape's thickness in document pixels.
    pub shape_line_width: f64,

    // Errors (the UI surfaces these)
    pub brush_error: Option<String>,
}

impl Default for EditorSession {
    fn default() -> Self {
        EditorSession {
            document: None,
            tool: NavigationTool::Move,
            active_layer_id: None,
            selected_layer_ids: Vec::new(),
            is_mask_selected: false,
            renaming_layer_id: None,
            collapsed_group_ids: std::collections::HashSet::new(),
            transform_duplicate: None,
            pixel_clipboard: None,
            text_draft: None,
            canvas_focus_request: 0,
            gradient_edit: None,
            guide_drag: None,
            history: DocumentHistory::default(),
            viewport: crate::canvas_viewport::CanvasViewport::default(),
            project_url: None,
            is_project_busy: false,
            is_importing: false,
            has_brush_stroke: false,
            blend_preview: None,
            effects_editing: None,
            effects_editing_original: None,
            effect_selection: None,
            color_picker: None,
            pending_opacity_digit: None,
            opacity_edit_layer_id: None,
            brush_settings: crate::tools::brush::BrushSettings::default(),
            brush_mode: BrushToolMode::Paint,
            spot_healing_mode: SpotHealingMode::ContentAware,
            blur_mode: BlurToolMode::Liquify,
            wand_mode: crate::selection::WandMode::Wand,
            wand_settings: WandSettings::default(),
            clone_settings: CloneSettings::default(),
            clone_source: None,
            clone_offset: None,
            mask_paint_white: false,
            background_color: PaletteColor::WHITE,
            last_brush_point: None,
            gradient_settings: GradientSettings::default(),
            lasso_draft: None,
            lasso_kind: LassoKind::Freehand,
            marquee_kind: LassoKind::Rectangle,
            selection_mode_choice: SelectionMode::Replace,
            selection_antialiased: true,
            selection_feather_amount: 2,
            selection_expand_amount: 1,
            selection_contract_amount: 1,
            selection_amount_operation: None,
            selection_move_origin: None,
            pixel_move: None,
            transform_edit: None,
            locks_transform_ratio: true,
            snapping_enabled: true,
            snap_guides: (Vec::new(), Vec::new()),
            crop_rect: None,
            crop_ratio_choice: "Free".to_string(),
            crop_drag: None,
            shows_new_document: false,
            shows_importer: false,
            import_error: None,
            shows_conversion_sheet: false,
            levels: None,
            levels_layer_id: None,
            hue_saturation: None,
            hue_saturation_layer_id: None,
            adjustment_editing_id: None,
            adjustment_original: None,
            filter_edit: None,
            transform_auto_select: false,
            shows_transform_controls: true,
            shows_guides: true,
            shows_rulers: false,
            shows_grid: false,
            shows_pixel_grid: true,
            snap_to_guides: true,
            snap_to_grid: false,
            snap_to_layers: true,
            snap_to_document_bounds: true,
            locks_guides: false,
            shape_kind: crate::document::ShapeKind::Rectangle,
            shape_corner_radius: 0.0,
            shape_line_width: 4.0,
            text_defaults: LayerTextStyle::default(),
            brush_error: None,
        }
    }
}

impl EditorSession {
    pub fn new() -> Self {
        Self::default()
    }

    // MARK: Document

    pub fn document_size(&self) -> Option<CGSize> {
        self.document.as_ref().map(|d| d.size())
    }

    /// `fit()` (EditorSession.swift:908-911).
    pub fn fit(&mut self) {
        let Some(size) = self.document_size() else {
            return;
        };
        self.viewport.fit(size);
    }

    /// `zoom(to:anchor:)` (EditorSession.swift:913-916): a nil anchor means
    /// the view center.
    pub fn zoom(&mut self, value: CGFloat, anchor: Option<CGPoint>) {
        let Some(size) = self.document_size() else {
            return;
        };
        let anchor = anchor.unwrap_or_else(|| self.viewport.center());
        self.viewport.set_zoom(value, anchor, size);
    }

    /// `zoomKeyboard(by:)` (EditorSession.swift:918-924): step through the
    /// stable keyboard zoom levels, anchored at the view center.
    pub fn zoom_keyboard(&mut self, step: i32) {
        if step == 0 {
            return;
        }
        let Some(size) = self.document_size() else {
            return;
        };
        let target = self.viewport.keyboard_zoom_target(step);
        if target == self.viewport.zoom() {
            return;
        }
        let center = self.viewport.center();
        self.viewport.set_zoom(target, center, size);
    }

    pub fn active_layer(&self) -> Option<&ImageLayer> {
        let id = self.active_layer_id?;
        self.document.as_ref()?.layer(id)
    }

    pub fn selection(&self) -> Option<&DocumentSelection> {
        self.document.as_ref()?.selection.as_ref()
    }

    // MARK: Layers

    pub fn select_layer(&mut self, id: Option<Uuid>) {
        // selectLayer (EditorSession.swift:325-331): the effect selection
        // dies, a changed layer finishes pending text, and switching commits
        // a pending transform and applies the pending gradient first.
        self.effect_selection = None;
        if id != self.active_layer_id {
            if !self.finish_text() {
                return;
            }
            self.commit_transform();
            self.resolve_gradient();
            self.is_mask_selected = false;
        }
        self.active_layer_id = id;
        self.selected_layer_ids = id.map(|i| vec![i]).unwrap_or_default();
    }

    /// `selectTool` (EditorSession.swift:332-352) — the one gate every tool
    /// switch passes. A failing text apply refuses the switch (:333);
    /// the busy guard (:334) refuses while a project operation runs, a
    /// stroke is in flight, or the Levels sheet is up; switching commits the
    /// pending transform, applies the pending gradient and cancels the crop
    /// frame (:335); entering Crop with no frame presets the full-document
    /// frame and resets the ratio choice (:347-351). Returns false when the
    /// switch was refused, so the caller can leave its app-side companions
    /// of the Swift cancels (the shape drag) alone. The port keeps the lasso
    /// cancel unconditional after the assignment (Swift cancels it inside
    /// the switching branch), and where Swift parks per-family brush tips
    /// the port's eraser entry is a `brush_mode` on the shared Brush tool,
    /// so selecting Brush resets it to Paint.
    pub fn select_tool(&mut self, value: NavigationTool) -> bool {
        let switching = self.tool != value;
        if switching && !self.finish_text() {
            return false;
        }
        // :334 — the busy flags moved core-side (stage-D ticket 34).
        if self.is_project_busy || self.has_brush_stroke || self.levels.is_some() {
            return false;
        }
        if switching {
            self.commit_transform();
            self.resolve_gradient();
            self.cancel_crop();
        }
        if value == NavigationTool::Brush {
            self.brush_mode = BrushToolMode::Paint;
        }
        self.tool = value;
        self.cancel_lasso();
        if value == NavigationTool::Crop
            && self.crop_rect.is_none()
            && let Some(document) = self.document.as_ref()
        {
            self.crop_ratio_choice = "Free".to_string();
            self.crop_rect = Some(CGRect::new(
                0.0,
                0.0,
                document.width as f64,
                document.height as f64,
            ));
        }
        true
    }

    /// `addBlankLayer`.
    pub fn add_blank_layer(&mut self) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let names: Vec<String> = document.layers.iter().map(|l| l.name.clone()).collect();
        let mut number = 1;
        while names.contains(&format!("Layer {number}")) {
            number += 1;
        }
        let mut layer = ImageLayer::blank(format!("Layer {number}"), document.size());
        layer.parent_id = match self.active_layer() {
            Some(active) if active.is_group => self.active_layer_id,
            Some(active) => active.parent_id,
            None => None,
        };
        let mut insertion = self
            .active_layer_id
            .and_then(|id| document.layer_index(id))
            .map(|i| i + 1)
            .unwrap_or(document.layers.len());
        // With a folder selected the layer goes to the top of the folder:
        // just above its last (topmost) contents.
        if let (Some(active), Some(folder)) = (self.active_layer(), self.active_layer_id)
            && active.is_group
        {
            let parents: std::collections::HashMap<Uuid, Option<Uuid>> = document
                .layers
                .iter()
                .map(|l| (l.id, l.parent_id))
                .collect();
            let is_inside = |id: Uuid| -> bool {
                let mut parent = parents.get(&id).copied().flatten();
                let mut steps = 0;
                while let Some(current) = parent {
                    if current == folder {
                        return true;
                    }
                    parent = parents.get(&current).copied().flatten();
                    steps += 1;
                    if steps > 64 {
                        break;
                    }
                }
                false
            };
            if let Some(topmost) = document.layers.iter().rposition(|l| is_inside(l.id)) {
                insertion = insertion.max(topmost + 1);
            }
        }
        self.begin_edit("New Blank Layer");
        let id = layer.id;
        if let Some(doc) = self.document.as_mut() {
            doc.layers.insert(insertion.min(doc.layers.len()), layer);
            self.active_layer_id = Some(id);
            self.selected_layer_ids = vec![id];
        }
        self.end_edit();
    }

    /// `deleteLayer` — a selected folder takes its contents. When the layer
    /// supplies live masks to layers that stay, Swift routes through
    /// `deleteWithLiveMaskChoice` (EditorSession.swift:635-639), which asks
    /// bake-or-unlink. The core has no dialog; baking keeps the masked
    /// appearance in the dependents' pixels and stays one undo step, so the
    /// plain path bakes. The app can offer the real choice via
    /// `delete_with_live_mask_choice`.
    pub fn delete_layer(&mut self, id: Uuid) {
        if self.delete_with_live_mask_choice(&[id], true) {
            return;
        }
        self.delete_layer_with_baked(id, Default::default());
    }

    fn delete_layer_with_baked(
        &mut self,
        id: Uuid,
        baked: std::collections::HashMap<Uuid, ImportedImage>,
    ) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(index) = document.layer_index(id) else {
            return;
        };
        let removed = document.descendant_ids(id);
        self.begin_edit("Delete Layer");
        if let Some(doc) = self.document.as_mut() {
            doc.layers.retain(|l| !removed.contains(&l.id));
            // Layers clipped to a deleted layer keep their pixels — baked in
            // when the delete chose to bake — unlinked.
            for layer in doc.layers.iter_mut() {
                if layer
                    .mask_source_id
                    .map(|source| removed.contains(&source))
                    .unwrap_or(false)
                {
                    layer.mask_source_id = None;
                    if let Some(image) = baked.get(&layer.id) {
                        layer.asset = Some(image.clone());
                    }
                }
            }
        }
        // The deleted active layer's selection moves to a neighbor
        // (finishDeletingLayer): layers[min(index, count-1)].
        if self
            .active_layer_id
            .map(|a| removed.contains(&a))
            .unwrap_or(false)
        {
            let count = self.document.as_ref().map(|d| d.layers.len()).unwrap_or(0);
            if count == 0 {
                self.active_layer_id = None;
                self.selected_layer_ids.clear();
            } else {
                let neighbor = self
                    .document
                    .as_ref()
                    .map(|d| d.layers[index.min(count - 1)].id);
                self.active_layer_id = neighbor;
                self.selected_layer_ids = neighbor.map(|n| vec![n]).unwrap_or_default();
            }
        }
        self.end_edit();
    }

    pub fn delete_active_layer(&mut self) {
        if let Some(id) = self.active_layer_id {
            self.delete_layer(id);
        }
    }

    pub fn rename_layer(&mut self, id: Uuid, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        // canEditLayers (EditorSession.swift:646): a crop or open transform
        // blocks layer edits.
        if !self.can_edit_layers() {
            return;
        }
        if self.document.as_ref().and_then(|d| d.layer(id)).is_some() {
            self.begin_edit("Rename Layer");
            if let Some(doc) = self.document.as_mut() {
                doc.layer_mut(id).unwrap().name = name.to_string();
            }
            self.end_edit();
        }
    }

    pub fn toggle_layer_visibility(&mut self, id: Uuid) {
        // canEditLayers (EditorSession.swift:657).
        if !self.can_edit_layers() {
            return;
        }
        let was_visible = self
            .document
            .as_ref()
            .and_then(|d| d.layer(id))
            .map(|l| l.is_visible);
        if let Some(visible) = was_visible {
            self.begin_edit(if visible { "Hide Layer" } else { "Show Layer" });
            if let Some(doc) = self.document.as_mut() {
                doc.layer_mut(id).unwrap().is_visible = !visible;
            }
            self.end_edit();
        }
    }

    /// `moveActiveLayer(by:)` — within its siblings.
    pub fn move_active_layer(&mut self, offset: i64) {
        // canMoveActiveLayer (EditorSession.swift:710-715) requires
        // canEditLayers too.
        if !self.can_edit_layers() {
            return;
        }
        let Some(active_id) = self.active_layer_id else {
            return;
        };
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(active) = document.layer(active_id) else {
            return;
        };
        let siblings: Vec<Uuid> = document
            .layers
            .iter()
            .filter(|l| l.parent_id == active.parent_id)
            .map(|l| l.id)
            .collect();
        let Some(index) = siblings.iter().position(|i| *i == active_id) else {
            return;
        };
        let target = index as i64 + offset;
        if target < 0 || target >= siblings.len() as i64 {
            return;
        }
        let other = siblings[target as usize];
        let a = document.layer_index(active_id).unwrap();
        let b = document.layer_index(other).unwrap();
        self.begin_edit("Reorder Layers");
        if let Some(doc) = self.document.as_mut() {
            doc.layers.swap(a, b);
        }
        self.end_edit();
    }

    /// `duplicateActiveLayer` — a folder copies with all its descendants,
    /// parent and mask-source links remapped to the copies.
    pub fn duplicate_active_layer(&mut self) {
        if !self.can_edit_layers() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(source_id) = self.active_layer_id else {
            return;
        };
        let Some(_source) = document.layer(source_id).cloned() else {
            return;
        };
        let Some(index) = document.layer_index(source_id) else {
            return;
        };
        let included = document.descendant_ids(source_id);
        let originals: Vec<ImageLayer> = document
            .layers
            .iter()
            .filter(|l| included.contains(&l.id))
            .cloned()
            .collect();
        if document.layers.len() + originals.len() > 10_000 {
            return;
        }
        let mapping: std::collections::HashMap<Uuid, Uuid> =
            originals.iter().map(|l| (l.id, Uuid::new_v4())).collect();
        let mut copies = originals;
        for copy in copies.iter_mut() {
            copy.id = mapping[&copy.id];
            if copy.id == mapping[&source_id] {
                copy.name = format!("{} copy", copy.name);
            }
            if let Some(parent) = copy.parent_id {
                copy.parent_id = Some(mapping.get(&parent).copied().unwrap_or(parent));
            }
            if let Some(source) = copy.mask_source_id {
                copy.mask_source_id = Some(mapping.get(&source).copied().unwrap_or(source));
            }
        }
        self.begin_edit("Duplicate Layer");
        if let Some(doc) = self.document.as_mut() {
            let position = (index + 1).min(doc.layers.len());
            for (offset, copy) in copies.into_iter().enumerate() {
                doc.layers.insert(position + offset, copy);
            }
            let copy_id = mapping[&source_id];
            self.active_layer_id = Some(copy_id);
            self.selected_layer_ids = vec![copy_id];
        }
        self.end_edit();
    }

    /// `finishShape`: fills the dragged shape with the foreground color on a
    /// new layer above the active one, in one undo step named after the kind.
    pub fn add_shape_layer(
        &mut self,
        shape: crate::document::LayerShape,
        rect: CGRect,
        name: String,
    ) -> Option<Uuid> {
        let document = self.document.clone()?;
        if rect.width() < 1.0 || rect.height() < 1.0 {
            return None;
        }
        if (rect.width() as usize).saturating_mul(rect.height() as usize)
            > crate::document::LayerShape::MAX_SHAPE_PIXELS
        {
            self.brush_error =
                Some("That shape is too large. A shape can cover up to 100 megapixels.".into());
            return None;
        }
        let transform = LayerTransform::new(rect.origin, rect.size);
        let undo_name = shape.kind.label().to_string();
        let raster = crate::render::render_shape_image(
            &shape,
            transform.size.width.round() as usize,
            transform.size.height.round() as usize,
        );
        let mut layer =
            ImageLayer::with_asset(ImportedImage::new(raster, name.clone()), rect.origin);
        layer.transform = transform;
        layer.shape = Some(shape);
        // Above the active layer, in its folder.
        layer.parent_id = match self.active_layer() {
            Some(active) if active.is_group => self.active_layer_id,
            Some(active) => active.parent_id,
            None => None,
        };
        let index = self
            .active_layer_id
            .and_then(|id| document.layer_index(id))
            .map(|i| i + 1)
            .unwrap_or(document.layers.len());
        // The undo step is named after the kind; the layer gets the numbered name.
        self.begin_edit(&undo_name);
        let id = layer.id;
        if let Some(doc) = self.document.as_mut() {
            doc.layers.insert(index.min(doc.layers.len()), layer);
            self.active_layer_id = Some(id);
            self.selected_layer_ids = vec![id];
        }
        self.end_edit();
        Some(id)
    }

    /// `"Rectangle 1"`, `"Ellipse 2"`, ... skipping names already in the document.
    pub fn next_shape_name(&self, kind: crate::document::ShapeKind) -> String {
        let names: std::collections::HashSet<String> = self
            .document
            .as_ref()
            .map(|d| d.layers.iter().map(|l| l.name.clone()).collect())
            .unwrap_or_default();
        let mut number = 1;
        while names.contains(&format!("{} {number}", kind.label())) {
            number += 1;
        }
        format!("{} {number}", kind.label())
    }

    // MARK: Appearance

    /// `canEditLayers` — the states the core session models; the app
    /// conjuncts its own stroke/sheet state on top.
    pub fn can_edit_layers(&self) -> bool {
        self.document.is_some() && self.transform_edit.is_none() && self.crop_rect.is_none()
    }

    pub fn can_edit_appearance(&self) -> bool {
        self.can_edit_layers()
            && self.selected_layer_ids.len() == 1
            && self.active_layer().map(|l| !l.is_group).unwrap_or(false)
    }

    pub fn displayed_blend_mode(&self, layer: &ImageLayer) -> LayerBlendMode {
        if let Some((id, mode)) = self.blend_preview
            && id == layer.id
            && self.active_layer_id == Some(layer.id)
        {
            return mode;
        }
        layer.blend_mode
    }

    /// `previewBlendMode` (LayerAppearance.swift:78-82) — the appearance
    /// panel's hover preview: temporary, only for the active layer while it
    /// can be edited; any other layer, or a None mode, clears it. Nothing is
    /// written to the document and no undo step is taken. Swift also pokes
    /// `refreshCanvasPreview` here; that closure is app-side.
    pub fn set_preview_blend_mode(&mut self, mode: Option<LayerBlendMode>, id: Option<Uuid>) {
        self.blend_preview = match (mode, id) {
            (Some(mode), Some(id))
                if Some(id) == self.active_layer_id && self.can_edit_appearance() =>
            {
                Some((id, mode))
            }
            _ => None,
        };
    }

    /// Shift-+ / Shift-− steps the blend mode through the menu order.
    pub fn cycle_blend_mode(&mut self, forward: bool) {
        if !self.can_edit_appearance() {
            return;
        }
        let Some(layer) = self.active_layer() else {
            return;
        };
        let modes = LayerBlendMode::ALL;
        let index = modes
            .iter()
            .position(|m| *m == layer.blend_mode)
            .unwrap_or(0);
        let next = modes[(index + if forward { 1 } else { modes.len() - 1 }) % modes.len()];
        self.set_layer_blend_mode(next);
    }

    /// `EditorSession.changeBrushHardness` — Shift-[ / Shift-]: hardness in
    /// Photoshop's 25% steps (0, 25, 50, 75, 100%). Snaps to the next step up
    /// or down, so 80% goes to 100% or 75%.
    pub fn change_brush_hardness(&mut self, increase: bool) {
        let quarter = self.brush_settings.hardness * 4.0;
        let step = if increase {
            (quarter + 0.001).floor() + 1.0
        } else {
            (quarter - 0.001).ceil() - 1.0
        };
        self.brush_settings.hardness = step.clamp(0.0, 4.0) / 4.0;
    }

    pub fn set_layer_blend_mode(&mut self, mode: LayerBlendMode) {
        self.blend_preview = None;
        if !self.can_edit_appearance() {
            return;
        }
        let Some(id) = self.active_layer_id else {
            return;
        };
        self.finish_opacity_edit();
        self.begin_edit("Layer Blend Mode");
        if let Some(doc) = self.document.as_mut() {
            doc.layer_mut(id).unwrap().blend_mode = mode;
        }
        self.end_edit();
    }

    pub fn begin_opacity_edit(&mut self) {
        if self.opacity_edit_layer_id.is_none()
            && let Some(id) = self.active_layer_id
        {
            self.begin_edit("Layer Opacity");
            self.opacity_edit_layer_id = Some(id);
        }
    }

    pub fn finish_opacity_edit(&mut self) {
        if self.opacity_edit_layer_id.is_some() {
            self.opacity_edit_layer_id = None;
            self.end_edit();
        }
    }

    pub fn set_layer_opacity(&mut self, opacity: f64) {
        if !opacity.is_finite() {
            return;
        }
        let Some(id) = self.opacity_edit_layer_id.or(self.active_layer_id) else {
            return;
        };
        let standalone = self.opacity_edit_layer_id.is_none();
        if standalone {
            self.begin_edit("Layer Opacity");
        }
        if let Some(doc) = self.document.as_mut()
            && let Some(layer) = doc.layer_mut(id)
        {
            layer.opacity = opacity.clamp(0.0, 1.0);
        }
        if standalone {
            self.end_edit();
        }
    }

    /// `setSelectedLayersOpacity` (LayerAppearance.swift:108-119) — sets every
    /// selected layer's opacity as one undo step. A selected folder takes the
    /// value too, dimming its contents on top of their own opacity.
    pub fn set_selected_layers_opacity(&mut self, opacity: f64) {
        if !opacity.is_finite() || !self.can_edit_layers() {
            return;
        }
        let value = opacity.clamp(0.0, 1.0);
        let selected = self.selected_layer_ids.clone();
        let indices: Vec<usize> = self
            .document
            .as_ref()
            .map(|document| {
                document
                    .layers
                    .iter()
                    .enumerate()
                    .filter(|(_, layer)| selected.contains(&layer.id) && layer.opacity != value)
                    .map(|(index, _)| index)
                    .collect()
            })
            .unwrap_or_default();
        if indices.is_empty() {
            return;
        }
        self.finish_opacity_edit();
        self.begin_edit("Layer Opacity");
        if let Some(document) = self.document.as_mut() {
            for index in indices {
                document.layers[index].opacity = value;
            }
        }
        self.end_edit();
    }

    /// `usesOpacityKeys` (EditorSession+Brush.swift:188) — tools where number
    /// keys set opacity: the brush family or gradient's own opacity, or with
    /// Move the opacity of the selected layers.
    pub fn uses_opacity_keys(&self) -> bool {
        self.tool.is_brush_tool()
            || self.tool == NavigationTool::Gradient
            || self.tool == NavigationTool::Move
    }

    /// `typeOpacityDigit` (EditorSession+Brush.swift:191-208) — Photoshop-style
    /// opacity keys: 1 = 10% … 9 = 90%, 0 = 100%. Two digits typed within 0.6 s
    /// set an exact value (4 then 5 = 45%, 0 then 5 = 5%). `uptime` is a
    /// monotonic clock in seconds.
    pub fn type_opacity_digit(&mut self, digit: i32, uptime: f64) {
        if !self.uses_opacity_keys() || !(0..=9).contains(&digit) {
            return;
        }
        let mut percent = if digit == 0 { 100 } else { digit * 10 };
        match self.pending_opacity_digit {
            Some((first, at)) if uptime - at < 0.6 => {
                percent = (first * 10 + digit).max(1);
                self.pending_opacity_digit = None;
            }
            _ => self.pending_opacity_digit = Some((digit, uptime)),
        }
        let value = percent as f64 / 100.0;
        match self.tool {
            NavigationTool::Brush
            | NavigationTool::SpotHealing
            | NavigationTool::CloneStamp
            | NavigationTool::Blur => self.brush_settings.opacity = value,
            NavigationTool::Gradient => self.gradient_settings.opacity = value,
            _ => self.set_selected_layers_opacity(value),
        }
    }

    // MARK: Import pipeline (EditorSession+Import / ImageImporter)

    /// `insert(_:centeredAt:)` — inserts pixels as a new top layer centered on
    /// `point` (the canvas center without one). With no document the image
    /// determines the canvas size.
    pub fn insert_image(&mut self, asset: ImportedImage, center: Option<CGPoint>) {
        if self.document.is_none() {
            self.document = Some(CanvasDocument::new(
                asset.image.width.max(1),
                asset.image.height.max(1),
            ));
            // `viewport.fit(documentSize: document!.size)` in the
            // first-document branch (EditorSession.swift:795-798).
            if let Some(size) = self.document_size() {
                self.viewport.fit(size);
            }
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let center = center.unwrap_or(CGPoint::new(
            document.width as f64 / 2.0,
            document.height as f64 / 2.0,
        ));
        let (asset_width, asset_height) = (asset.image.width as f64, asset.image.height as f64);
        let mut layer = ImageLayer::with_asset(
            asset,
            CGPoint::new(
                (center.x - asset_width / 2.0).floor(),
                (center.y - asset_height / 2.0).floor(),
            ),
        );
        layer.parent_id = match self.active_layer() {
            Some(active) if active.is_group => self.active_layer_id,
            Some(active) => active.parent_id,
            None => None,
        };
        let layer_id = layer.id;
        self.begin_edit("Import Image");
        if let Some(doc) = self.document.as_mut() {
            doc.layers.push(layer);
        }
        self.active_layer_id = Some(layer_id);
        self.selected_layer_ids = vec![layer_id];
        self.end_edit();
    }

    /// Size guards for a bitmap import: 30,000 pixels per side, and the
    /// document's total pixel budget of 100 megapixels
    /// (`ImageImportError.tooLarge`).
    fn import_size_allowed(&self, width: usize, height: usize) -> bool {
        if width == 0 || height == 0 || width > 30_000 || height > 30_000 {
            return false;
        }
        let used: u64 = self
            .document
            .as_ref()
            .map(|d| {
                d.layers
                    .iter()
                    .filter_map(|l| l.asset.as_ref())
                    .map(|a| a.image.width as u64 * a.image.height as u64)
                    .sum()
            })
            .unwrap_or(0);
        used + (width as u64 * height as u64) <= 100_000_000
    }

    /// `importImages`/`drainImports` for bitmaps: decodes each file, applies
    /// the size guards, and inserts every success centered on `point` (which
    /// is ignored while there is no document — the first image determines the
    /// canvas). Returns one failure line per rejected file.
    pub fn import_images(
        &mut self,
        paths: &[std::path::PathBuf],
        center: Option<CGPoint>,
    ) -> Vec<String> {
        if paths.is_empty() {
            return Vec::new();
        }
        self.commit_transform();
        self.cancel_crop();
        self.begin_edit("Import Images");
        let mut failures = Vec::new();
        for path in paths {
            let name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_else(|| "Image".to_string());
            let Some((bitmap, _thumbnail)) = decode_image(path) else {
                failures.push(format!("{}: the image could not be read.", path.display()));
                continue;
            };
            if !self.import_size_allowed(bitmap.width, bitmap.height) {
                failures.push(format!(
                    "{}: the image is too large for the document.",
                    path.display()
                ));
                continue;
            }
            // No document yet: the drop point does not apply, the first image
            // determines the canvas.
            let point = if self.document.is_some() {
                center
            } else {
                None
            };
            let asset = ImportedImage::new(
                Bitmap::from_raw(bitmap.width, bitmap.height, bitmap.data),
                &name,
            );
            self.insert_image(asset, point);
        }
        self.end_edit();
        failures
    }

    /// `insertPhotoshop` for file data: parses the PSD, applies it as a fresh
    /// document (no document yet) or as a wrapping group, and reports how many
    /// conversion notes the import produced.
    pub fn import_photoshop_data(&mut self, name: &str, data: &[u8]) -> Result<usize, String> {
        let parsed = crate::psd::PsdReader::read_with_budget(data, 100_000_000)
            .map_err(|error| error.to_string())?;
        let assets = crate::psd::PsdDocumentBuilder::assets(&parsed);
        let imported = crate::psd::PsdDocumentBuilder::make_import_with_assets(&parsed, &assets);
        let mut incoming = imported.layers;
        let wrapping = self.document.is_some();
        let added = incoming.len() + usize::from(wrapping);
        let layer_count = self.document.as_ref().map(|d| d.layers.len()).unwrap_or(0);
        if layer_count + added > 10_000 {
            return Err("too many layers".to_string());
        }
        self.begin_edit("Import Photoshop File");
        if !wrapping {
            let mut document = CanvasDocument::new(imported.width, imported.height);
            document.resolution = imported.resolution;
            let active = incoming
                .iter()
                .rev()
                .find(|layer| layer.parent_id.is_none())
                .map(|layer| layer.id)
                .or_else(|| incoming.last().map(|layer| layer.id));
            document.layers = incoming;
            let size = document.size();
            self.document = Some(document);
            // `viewport.fit(documentSize: document!.size)` in the
            // first-document branch (EditorSession.swift:862-866).
            self.viewport.fit(size);
            self.select_layer(active);
        } else {
            let size = self.document.as_ref().unwrap().size();
            let mut group = ImageLayer::blank(name.to_string(), size);
            group.is_group = true;
            group.parent_id = match self.active_layer() {
                Some(active) if active.is_group => self.active_layer_id,
                Some(active) => active.parent_id,
                None => None,
            };
            let group_id = group.id;
            for layer in incoming.iter_mut() {
                if layer.parent_id.is_none() {
                    layer.parent_id = Some(group_id);
                }
            }
            if let Some(doc) = self.document.as_mut() {
                doc.layers.push(group);
                doc.layers.extend(incoming);
            }
            self.active_layer_id = Some(group_id);
            self.selected_layer_ids = vec![group_id];
        }
        self.end_edit();
        Ok(imported.conversions.len())
    }

    // MARK: Transform snapping (Guides.swift)

    /// `alignmentSnapTargets` — the canvas bounds and other layers' edges
    /// (plus centers on request), rounded to whole pixels.
    pub fn alignment_snap_targets(
        &self,
        excluding: &[Uuid],
        include_centers: bool,
    ) -> (Vec<CGFloat>, Vec<CGFloat>) {
        let Some(document) = self.document.as_ref() else {
            return (Vec::new(), Vec::new());
        };
        if !self.snapping_enabled {
            return (Vec::new(), Vec::new());
        }
        let mut xs = vec![0.0, document.width as CGFloat];
        let mut ys = vec![0.0, document.height as CGFloat];
        if include_centers {
            xs.push(document.width as CGFloat / 2.0);
            ys.push(document.height as CGFloat / 2.0);
        }
        for layer in document.render_layers() {
            if layer.asset.is_none() || excluding.contains(&layer.id) {
                continue;
            }
            let corners = layer.transform.corners();
            let min_x = corners
                .iter()
                .map(|p| p.x)
                .fold(f64::INFINITY, f64::min)
                .round();
            let max_x = corners
                .iter()
                .map(|p| p.x)
                .fold(f64::NEG_INFINITY, f64::max)
                .round();
            let min_y = corners
                .iter()
                .map(|p| p.y)
                .fold(f64::INFINITY, f64::min)
                .round();
            let max_y = corners
                .iter()
                .map(|p| p.y)
                .fold(f64::NEG_INFINITY, f64::max)
                .round();
            xs.push(min_x);
            xs.push(max_x);
            ys.push(min_y);
            ys.push(max_y);
            if include_centers {
                xs.push(((min_x + max_x) / 2.0).round());
                ys.push(((min_y + max_y) / 2.0).round());
            }
        }
        (xs, ys)
    }

    /// `transformSnapTargets` — moving layers snap to edges and centers.
    pub fn transform_snap_targets(&self, excluding: &[Uuid]) -> (Vec<CGFloat>, Vec<CGFloat>) {
        self.alignment_snap_targets(excluding, true)
    }

    /// `snappedMove` — `draft` nudged so the layer it places lines up with a
    /// nearby edge or center; `moving` is what is being dragged, and
    /// `tolerance` is in document pixels. The guides it landed on are stored
    /// for the canvas to draw.
    pub fn snapped_move(
        &mut self,
        draft: LayerTransform,
        moving: &[Uuid],
        tolerance: CGFloat,
    ) -> LayerTransform {
        if !self.snapping_enabled {
            self.snap_guides = (Vec::new(), Vec::new());
            return draft;
        }
        let corners = draft.corners();
        let min_x = corners.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
        let max_x = corners
            .iter()
            .map(|p| p.x)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_y = corners.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let max_y = corners
            .iter()
            .map(|p| p.y)
            .fold(f64::NEG_INFINITY, f64::max);
        let bounds = crate::geometry::CGRect::new(min_x, min_y, max_x - min_x, max_y - min_y);
        let targets = self.transform_snap_targets(moving);
        let (offset, guide_x, guide_y) =
            crate::transform::TransformSnap::offset(&bounds, &targets.0, &targets.1, tolerance);
        self.snap_guides = (
            guide_x.map(|g| vec![g]).unwrap_or_default(),
            guide_y.map(|g| vec![g]).unwrap_or_default(),
        );
        if offset.width == 0.0 && offset.height == 0.0 {
            return draft;
        }
        let mut snapped = draft;
        snapped.origin.x += offset.width;
        snapped.origin.y += offset.height;
        snapped
    }

    // MARK: Distort (Distort.swift)

    /// Cmd-drag on a transform handle: the corners start moving freely
    /// (`beginDistort`). The edit then waits for Apply rather than applying on
    /// mouse-up.
    pub fn begin_distort(&mut self) {
        let Some(mut edit) = self.transform_edit.clone() else {
            return;
        };
        if edit.corners.is_some() || !edit.draft.is_valid() {
            return;
        }
        edit.corners = Some(crate::tools::distort::corners_of(&edit.draft));
        self.transform_edit = Some(edit);
    }

    /// Moves the distortion's corners; a twisted or collapsed shape is ignored.
    pub fn preview_corners(&mut self, corners: [CGPoint; 4]) {
        if self
            .transform_edit
            .as_ref()
            .and_then(|edit| edit.corners)
            .is_none()
            || !crate::tools::distort::is_usable(&corners)
        {
            return;
        }
        if let Some(edit) = self.transform_edit.as_mut() {
            edit.corners = Some(corners);
        }
    }

    /// `distortTarget` — where a distortion takes `layer`: its transform under
    /// the edit and the corners that transform moves to.
    fn distort_target(
        &self,
        layer: &ImageLayer,
        edit: &TransformEdit,
        shape: &[CGPoint; 4],
    ) -> Option<(LayerTransform, [CGPoint; 4])> {
        let _ = layer;
        if edit.group.is_none() {
            return if edit.layer_id == layer.id {
                Some((edit.draft, *shape))
            } else {
                None
            };
        }
        let group = edit.group.as_ref()?;
        let original = group.original(layer.id)?;
        let transform = original.following(&group.box_, &edit.draft);
        let corners = crate::tools::distort::carried(&transform, &edit.draft, shape);
        if !crate::tools::distort::is_usable(&corners) {
            return None;
        }
        Some((transform, corners))
    }

    /// `commitDistort` — each distorted layer's pixels and mask are resampled
    /// into its shape, as one undo step.
    fn commit_distort(&mut self, edit: &TransformEdit, shape: [CGPoint; 4]) {
        let ids: Vec<Uuid> = match &edit.group {
            Some(group) => group.originals.iter().map(|(id, _)| *id).collect(),
            None => vec![edit.layer_id],
        };
        self.begin_edit(if edit.group.is_none() {
            "Distort"
        } else {
            "Distort Layers"
        });
        for id in ids {
            let Some(document) = self.document.as_ref() else {
                continue;
            };
            let Some(layer) = document.layer(id).cloned() else {
                continue;
            };
            let Some((transform, corners)) = self.distort_target(&layer, edit, &shape) else {
                continue;
            };
            self.distort_at(id, &transform, &corners);
        }
        self.end_edit();
    }

    /// `distort(at:transform:corners:)` — pixels warp into the shape (cropped
    /// to what is visible); a linked mask warps with the pixels, a linked
    /// mask placed apart carries the same perspective over its own bounds,
    /// and an unlinked mask stays put.
    fn distort_at(&mut self, id: Uuid, transform: &LayerTransform, corners: &[CGPoint; 4]) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(layer) = document.layer(id).cloned() else {
            return;
        };
        let Some(image) = layer.asset.as_ref().map(|a| a.image.as_ref().clone()) else {
            return;
        };
        let Ok((warped, placed, crop)) =
            crate::tools::distort::warp_trimmed(&image, transform, corners)
        else {
            return;
        };
        let mut mask = layer.mask.clone();
        if let Some(original) = layer.mask.clone() {
            if original.placement.is_none() && original.is_linked {
                // Warp with the pixels, then crop with them.
                if let Ok((warped_mask, _)) = crate::tools::distort::warp(
                    original.asset.image.as_ref(),
                    transform,
                    corners,
                    true,
                    None,
                ) {
                    if warped_mask.width == 1 && warped_mask.height == 1 {
                        // A uniform mask passes through.
                    } else {
                        let crop_x = crop.min_x().round().max(0.0) as usize;
                        let crop_y = crop.min_y().round().max(0.0) as usize;
                        let crop_w = crop.width().round().max(1.0) as usize;
                        let crop_h = crop.height().round().max(1.0) as usize;
                        // Defensive: a crop rect outside the warped mask must
                        // keep the mask as it is, never produce an empty 0×0
                        // bitmap that would replace the layer's mask.
                        let crop_w = crop_w.min(warped_mask.width.saturating_sub(crop_x));
                        let crop_h = crop_h.min(warped_mask.height.saturating_sub(crop_y));
                        if crop_w > 0 && crop_h > 0 {
                            let mut cropped = Bitmap::new(crop_w, crop_h);
                            for y in 0..cropped.height {
                                for x in 0..cropped.width {
                                    cropped.set_pixel(
                                        x,
                                        y,
                                        warped_mask.pixel(crop_x + x, crop_y + y),
                                    );
                                }
                            }
                            mask =
                                Some(original.replacing(ImportedImage::new(cropped, "Layer Mask")));
                        }
                    }
                }
            } else if original.is_linked {
                if let Some(placed_mask) = original.placement {
                    let placement = placed_mask.following(&layer.transform, transform);
                    let carried = crate::tools::distort::carried(&placement, transform, corners);
                    if crate::tools::distort::is_convex(&carried) {
                        let background = LayerMask::background_of(&original.asset.thumbnail);
                        if let Ok((moved, moved_transform)) = crate::tools::distort::warp_mask(
                            original.asset.image.as_ref(),
                            &placement,
                            &carried,
                            background,
                            None,
                        ) {
                            let mut updated = original.clone();
                            updated.asset = ImportedImage::new(moved, "Layer Mask");
                            updated.placement = Some(moved_transform);
                            mask = Some(updated);
                        }
                    } else {
                        mask.as_mut().unwrap().placement = Some(placement);
                    }
                }
            } else {
                // An unlinked mask keeps its place on the document.
                if let Some(mask_ref) = mask.as_mut() {
                    mask_ref.placement = original.placement.or(Some(layer.transform));
                }
            }
        }
        if let Some(document) = self.document.as_mut()
            && let Some(current) = document.layer_mut(id)
        {
            current.asset = Some(ImportedImage::new(warped, current.name.clone()));
            current.transform = placed;
            current.mask = mask;
        }
    }

    /// `commitMaskTransform`'s distortion branch: a distorted mask resamples
    /// into its shape over the layer's bounds ("Distort Layer Mask").
    fn commit_mask_distort(&mut self, edit: &TransformEdit, shape: [CGPoint; 4]) {
        if !edit.draft.is_valid() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(layer) = document.layer(edit.layer_id).cloned() else {
            return;
        };
        let Some(mask) = layer.mask.clone() else {
            return;
        };
        let background = LayerMask::background_of(&mask.asset.thumbnail);
        let Some((moved, moved_transform)) = crate::tools::distort::warp_mask(
            mask.asset.image.as_ref(),
            &edit.draft,
            &shape,
            background,
            None,
        )
        .ok() else {
            return;
        };
        let placement = if moved_transform.same_placement(&layer.transform) {
            None
        } else {
            Some(moved_transform)
        };
        self.begin_edit("Distort Layer Mask");
        if let Some(doc) = self.document.as_mut()
            && let Some(current) = doc.layer_mut(edit.layer_id)
            && let Some(current_mask) = current.mask.as_mut()
        {
            current_mask.asset = ImportedImage::new(moved, "Layer Mask");
            current_mask.placement = placement;
        }
        self.end_edit();
    }

    // MARK: Transforms

    pub fn commit_transform(&mut self) {
        // Swift declares the duplicate cleanup after `guard let edit`, so a
        // commit with no open transform leaves the folded edit alone.
        let had_edit = self.transform_edit.is_some();
        self.commit_transform_inner();
        // An Option-drag duplicate folds the duplication and the move into
        // one undo step: the edit opened in `begin_duplicate_transform` closes
        // here, whatever the transform commit decided (commitTransform's
        // defer, EditorSession.swift:423-426).
        if had_edit && self.transform_duplicate.is_some() {
            self.transform_duplicate = None;
            self.end_edit();
        }
    }

    fn commit_transform_inner(&mut self) {
        self.finish_opacity_edit();
        self.snap_guides = (Vec::new(), Vec::new());
        // A blend preview never survives a commit (EditorSession.swift:425).
        self.blend_preview = None;
        let Some(edit) = self.transform_edit.take() else {
            return;
        };
        // Cmd-T on a selection: unchanged drafts restore exactly (so soft
        // selection edges never pick up a seam); anything else merges.
        if let Some(floating) = edit.floating.clone() {
            if edit.draft == floating.original && edit.corners.is_none() {
                self.cancel_floating_transform(&floating);
            } else {
                self.merge_floating_transform(&edit, &floating);
            }
            return;
        }
        // A distortion (Cmd-dragged handles): pixels and mask resample into
        // the shape, as one undo step (commitDistort).
        if let Some(shape) = edit.corners {
            if edit.mask {
                self.commit_mask_distort(&edit, shape);
            } else {
                self.commit_distort(&edit, shape);
            }
            return;
        }
        // An unlinked mask transformed on its own takes the new placement;
        // its layer stays put (commitMaskTransform).
        if edit.mask {
            self.commit_mask_transform(&edit);
            return;
        }
        // A group transform carries every member from the box to the draft in
        // one undo step (commitTransform's group branch, EditorSession.swift:437-449).
        if let Some(group) = edit.group.clone() {
            if !edit.draft.is_valid() {
                return;
            }
            let Some(_document) = self.document.as_mut() else {
                return;
            };
            self.begin_edit("Transform Layers");
            let document = self.document.as_mut().unwrap();
            for (id, original) in &group.originals {
                let Some(index) = document.layer_index(*id) else {
                    continue;
                };
                let moved = original.following(&group.box_, &edit.draft);
                if !moved.is_valid() {
                    continue;
                }
                let layer = &mut document.layers[index];
                if let Some(mask) = layer.mask.as_mut() {
                    mask.placement = mask.placement_moving_layer(original, &moved);
                }
                if let Some(shape) = layer.shape.clone() {
                    let new_w = moved.size.width.round().max(1.0) as usize;
                    let new_h = moved.size.height.round().max(1.0) as usize;
                    if new_w <= 10_000 && new_h <= 10_000 {
                        let name = layer
                            .asset
                            .as_ref()
                            .map(|a| a.name.clone())
                            .unwrap_or_default();
                        let raster = crate::render::render_shape_image(&shape, new_w, new_h);
                        layer.asset = Some(ImportedImage::new(raster, name));
                    }
                }
                layer.transform = moved;
            }
            self.end_edit();
            return;
        }
        if !edit.draft.is_valid() {
            return;
        }
        let Some(document) = self.document.as_mut() else {
            return;
        };
        let Some(index) = document.layer_index(edit.layer_id) else {
            return;
        };
        self.begin_edit("Transform Layer");
        let document = self.document.as_mut().unwrap();
        let layer = &mut document.layers[index];
        if let Some(mask) = layer.mask.as_mut() {
            let old = layer.transform;
            mask.placement = mask.placement_moving_layer(&old, &edit.draft);
        }
        // A shape layer re-renders its raster at the new bounds (redrawShape):
        // resizing redraws, it never stretches the old pixels.
        if let Some(shape) = layer.shape.clone() {
            let new_w = edit.draft.size.width.round().max(1.0) as usize;
            let new_h = edit.draft.size.height.round().max(1.0) as usize;
            if new_w <= 10_000 && new_h <= 10_000 {
                let name = layer
                    .asset
                    .as_ref()
                    .map(|a| a.name.clone())
                    .unwrap_or_default();
                let raster = crate::render::render_shape_image(&shape, new_w, new_h);
                layer.asset = Some(ImportedImage::new(raster, name));
            }
        }
        layer.transform = edit.draft;
        self.end_edit();
    }

    /// `commitMaskTransform` — the mask alone moved: its pixels are untouched,
    /// it just takes the placement, and the undo step is named for the mask.
    fn commit_mask_transform(&mut self, edit: &TransformEdit) {
        if !edit.draft.is_valid() {
            return;
        }
        let placement = self.document.as_ref().and_then(|document| {
            let layer = document.layer(edit.layer_id)?;
            let mask = layer.mask.as_ref()?;
            Some(if edit.draft.same_placement(&layer.transform) {
                None
            } else {
                Some(edit.draft)
            })
            .filter(|placement| *placement != mask.placement)
        });
        let Some(placement) = placement else {
            return;
        };
        self.begin_edit("Transform Layer Mask");
        if let Some(doc) = self.document.as_mut()
            && let Some(layer) = doc.layer_mut(edit.layer_id)
            && let Some(mask) = layer.mask.as_mut()
        {
            mask.placement = placement;
        }
        self.end_edit();
    }

    /// `canTransformSelection` — Cmd-T with a selection floats the selected
    /// pixels instead of transforming the layer.
    pub fn can_transform_selection(&self) -> bool {
        self.transform_edit.is_none()
            && self.can_edit_pixels()
            && !self.is_mask_selected
            && self.selection().map(|s| !s.is_empty()).unwrap_or(false)
            && self.active_layer().and_then(|l| l.asset.as_ref()).is_some()
    }

    /// `beginSelectionTransform`: the selected pixels lift onto a temporary
    /// "Floating Selection" layer above their source — with the source's
    /// parent, opacity and blend mode — and the whole thing opens one
    /// "Transform Selection" undo step that merge/cancel closes.
    pub fn begin_selection_transform(&mut self) {
        if !self.can_transform_selection() {
            return;
        }
        let Some(source) = self.active_layer().cloned() else {
            return;
        };
        let Some((lifted_image, region)) = self.render_selected_pixels(false) else {
            return;
        };
        let Some(before) = self.document.clone() else {
            return;
        };
        let before_active = self.active_layer_id;
        self.begin_edit("Transform Selection");
        self.clear_selected_pixels();
        let mut floating = ImageLayer::with_asset(
            ImportedImage::new(lifted_image, "Floating Selection"),
            region.origin,
        );
        floating.parent_id = source.parent_id;
        floating.opacity = source.opacity;
        floating.blend_mode = source.blend_mode;
        let floating_id = floating.id;
        let original = floating.transform;
        if let Some(doc) = self.document.as_mut() {
            let index = doc
                .layer_index(source.id)
                .map(|i| i + 1)
                .unwrap_or(doc.layers.len());
            doc.layers.insert(index.min(doc.layers.len()), floating);
        }
        self.active_layer_id = Some(floating_id);
        self.tool = NavigationTool::Move;
        self.transform_edit = Some(TransformEdit {
            layer_id: floating_id,
            draft: original,
            persistent: true,
            floating: Some(crate::selection::FloatingTransform {
                source_id: source.id,
                before: Box::new(before),
                before_active,
                original,
                pixel_size: CGSize::new(region.width(), region.height()),
            }),
            corners: None,
            mask: false,
            group: None,
        });
    }

    /// `floatingSelectionTransform` — maps the original selection to where the
    /// floating pixels are now.
    pub fn floating_selection_transform(
        edit: &TransformEdit,
    ) -> Option<crate::geometry::CGAffineTransform> {
        let floating = edit.floating.as_ref()?;
        let width = floating.pixel_size.width;
        let height = floating.pixel_size.height;
        // Applied to a document point: undo the original placement, then the
        // current draft.
        Some(
            crate::raster::pixel_to_document(&edit.draft, width, height).concat(
                &crate::raster::pixel_to_document(&floating.original, width, height).inverted(),
            ),
        )
    }

    /// `mergeFloatingTransform`: composites the transformed pixels back into
    /// their layer, moves the selection with them, and closes the undo step.
    fn merge_floating_transform(
        &mut self,
        edit: &TransformEdit,
        floating: &crate::selection::FloatingTransform,
    ) {
        let merged = self.merge_floating_into_source(edit, floating);
        match merged {
            Some(()) => {
                self.end_edit();
            }
            None => {
                // Nothing partial survives a failed merge.
                self.document = Some((*floating.before).clone());
                self.active_layer_id = floating.before_active;
                self.brush_error = Some("The floating selection could not be merged.".into());
                self.end_edit();
            }
        }
    }

    fn merge_floating_into_source(
        &mut self,
        edit: &TransformEdit,
        floating: &crate::selection::FloatingTransform,
    ) -> Option<()> {
        if !edit.draft.is_valid() {
            return None;
        }
        let document = self.document.as_ref()?;
        let pixels = document
            .layer(edit.layer_id)
            .and_then(|l| l.asset.as_ref())
            .map(|a| a.image.as_ref().clone())?;
        let source = document.layer(floating.source_id).cloned()?;
        let (asset, merged_transform, merged_mask) = floating_merge(&pixels, &edit.draft, &source)?;
        // The selection moves with the pixels: a distortion carries the
        // outline through DistortWarp.mapPath, an affine through the matrix
        // (FloatingSelection.swift:80-100).
        let moved = self.selection().and_then(|selection| {
            if let Some(corners) = edit.corners {
                let placement = crate::raster::pixel_to_document(
                    &floating.original,
                    floating.pixel_size.width.max(1.0),
                    floating.pixel_size.height.max(1.0),
                );
                return crate::tools::distort::map_path(
                    &selection.path,
                    &placement,
                    floating.pixel_size,
                    &edit.draft,
                    &corners,
                )
                .map(|path| crate::selection::DocumentSelection {
                    path,
                    antialiased: selection.antialiased,
                    feather: selection.feather,
                });
            }
            let matrix = Self::floating_selection_transform(edit)?;
            Some(crate::selection::DocumentSelection {
                path: selection.path.transformed(&matrix),
                antialiased: selection.antialiased,
                feather: selection.feather,
            })
        });
        let document = self.document.as_mut()?;
        document.layers.retain(|l| l.id != edit.layer_id);
        let layer = document.layer_mut(floating.source_id)?;
        layer.asset = Some(asset);
        layer.transform = merged_transform;
        layer.mask = merged_mask;
        document.selection = moved;
        self.active_layer_id = Some(floating.source_id);
        Some(())
    }

    /// `cancelFloatingTransform` — Escape restores the document exactly.
    pub fn cancel_floating_transform(&mut self, floating: &crate::selection::FloatingTransform) {
        self.document = Some((*floating.before).clone());
        self.active_layer_id = floating.before_active;
        self.end_edit();
    }

    pub fn cancel_transform(&mut self) {
        self.snap_guides = (Vec::new(), Vec::new());
        let Some(edit) = self.transform_edit.take() else {
            return;
        };
        // Cancelling an Option-drag duplicate removes the copies, puts the
        // selection back, and closes the folded edit (cancelTransform,
        // EditorSession.swift:462-470).
        if let Some(duplicate) = self.transform_duplicate.take() {
            let mut removed: std::collections::HashSet<Uuid> =
                duplicate.copies.iter().copied().collect();
            if let Some(document) = self.document.as_ref() {
                for copy in &duplicate.copies {
                    removed.extend(document.descendant_ids(*copy));
                }
            }
            if let Some(document) = self.document.as_mut() {
                document.layers.retain(|l| !removed.contains(&l.id));
                self.collapsed_group_ids.retain(|id| !removed.contains(id));
            }
            self.select_layers(&duplicate.source, duplicate.primary);
            self.end_edit();
        }
        if let Some(floating) = edit.floating {
            self.cancel_floating_transform(&floating);
        }
    }

    /// `beginDuplicateTransform` (EditorSession.swift:402-421) — Option-drag
    /// duplicates the selected roots with their descendants and starts a
    /// transient transform on the copies, all folded into one undo step.
    pub fn begin_duplicate_transform(&mut self) {
        if self.transform_duplicate.is_some() {
            return;
        }
        let Some(primary) = self.active_layer_id else {
            return;
        };
        self.commit_transform();
        if !self.can_transform() {
            return;
        }
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let selection: std::collections::HashSet<Uuid> =
            self.selected_layer_ids.iter().copied().collect();
        // The descendants of a selected root are carried by its duplicate, not
        // duplicated again on their own. (Swift's descendantIDs(of:) excludes
        // the id itself; the Rust one includes it.)
        let mut carried: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
        for id in &selection {
            carried.extend(document.descendant_ids(*id));
            carried.remove(id);
        }
        // Bottom to top, so the copies keep the order they had.
        let targets: Vec<Uuid> = document
            .layers
            .iter()
            .filter(|l| selection.contains(&l.id) && !carried.contains(&l.id))
            .map(|l| l.id)
            .collect();
        if targets.is_empty() {
            return;
        }
        self.begin_edit(if targets.len() > 1 {
            "Duplicate Layers"
        } else {
            "Duplicate Layer"
        });
        let mut copies: Vec<Uuid> = Vec::new();
        for id in targets {
            self.select_layer(Some(id));
            self.duplicate_active_layer();
            if let Some(copy) = self.active_layer_id
                && copy != id
            {
                copies.push(copy);
            }
        }
        if copies.is_empty() {
            self.end_edit();
            let source: Vec<Uuid> = selection.into_iter().collect();
            self.select_layers(&source, Some(primary));
            return;
        }
        let source: Vec<Uuid> = selection.into_iter().collect();
        let primary = copies.last().copied();
        self.transform_duplicate = Some(TransformDuplicate {
            copies: copies.clone(),
            source,
            primary,
        });
        self.select_layers(&copies, primary);
        self.begin_transform_persistent(false);
    }

    /// `beginTransform()` — the common case: a user-opened transform that
    /// stays open for its handles (`persistent: true`).
    pub fn begin_transform(&mut self) {
        self.begin_transform_persistent(true);
    }

    /// `beginTransform(persistent:)` (EditorSession.swift:375-391). Swift's
    /// transient transforms pass `persistent: false` — a nudge or an
    /// Option-drag duplicate opens, moves, and commits in one go, so no
    /// handles are shown for it. The commit and cancel paths treat both the
    /// same; only the render side reads the flag.
    pub fn begin_transform_persistent(&mut self, persistent: bool) {
        self.cancel_crop();
        if self.transform_edit.is_some() {
            return;
        }
        if !self.can_transform() {
            return;
        }
        let Some(layer) = self.active_layer().cloned() else {
            return;
        };
        self.tool = NavigationTool::Move;
        // Several selected layers transform together in one box
        // (EditorSession.swift:379-386).
        if self.transforms_as_group() {
            let Some(box_) = self.group_transform_box() else {
                return;
            };
            let originals = self
                .group_transform_member_ids()
                .into_iter()
                .filter_map(|id| {
                    self.document
                        .as_ref()
                        .and_then(|d| d.layer(id))
                        .map(|l| (id, l.transform))
                })
                .collect();
            self.transform_edit = Some(TransformEdit {
                layer_id: layer.id,
                draft: box_,
                persistent,
                floating: None,
                corners: None,
                mask: false,
                group: Some(TransformGroup { box_, originals }),
            });
            return;
        }
        let mask_alone =
            self.is_mask_selected && layer.mask.as_ref().map(|m| !m.is_linked).unwrap_or(false);
        let draft = if mask_alone {
            layer.mask_transform()
        } else {
            layer.transform
        };
        self.transform_edit = Some(TransformEdit {
            layer_id: layer.id,
            draft,
            persistent,
            floating: None,
            corners: None,
            mask: mask_alone,
            group: None,
        });
    }

    pub fn preview_transform(&mut self, value: LayerTransform) {
        if value.is_valid()
            && let Some(edit) = self.transform_edit.as_mut()
        {
            edit.draft = value;
        }
    }

    pub fn can_transform(&self) -> bool {
        if !self.can_edit_layers() {
            return false;
        }
        // Several selected layers, or a folder's contents, transform together
        // (EditorSession.swift:295-299).
        if self.transforms_as_group() {
            return !self.group_transform_member_ids().is_empty();
        }
        let Some(layer) = self.active_layer() else {
            return false;
        };
        layer.asset.is_some() && !layer.is_group
    }

    /// `transformsAsGroup` (EditorSession.swift:300) — several layers selected,
    /// or a single selected folder.
    pub fn transforms_as_group(&self) -> bool {
        self.selected_layer_ids.len() > 1
            || (self.selected_layer_ids.len() == 1
                && self.active_layer().map(|l| l.is_group).unwrap_or(false))
    }

    /// `groupTransformMembers` (EditorSession.swift:304-317) — the visible
    /// pixel layers selected or inside selected folders, bottom to top.
    pub fn group_transform_member_ids(&self) -> Vec<Uuid> {
        let Some(document) = self.document.as_ref() else {
            return Vec::new();
        };
        if !self.transforms_as_group() {
            return Vec::new();
        }
        let selected: std::collections::HashSet<Uuid> =
            self.selected_layer_ids.iter().copied().collect();
        let visible = document.effective_visible_ids();
        let mut members = Vec::new();
        for layer in &document.layers {
            if layer.asset.is_none() || layer.is_group || !visible.contains(&layer.id) {
                continue;
            }
            let mut current = Some(layer.id);
            for _ in 0..64 {
                let Some(id) = current else {
                    break;
                };
                if selected.contains(&id) {
                    members.push(layer.id);
                    break;
                }
                current = document
                    .layers
                    .iter()
                    .find(|l| l.id == id)
                    .and_then(|l| l.parent_id);
            }
        }
        members
    }

    /// `groupTransformBox` (EditorSession.swift:320-327) — the upright box
    /// around the group members.
    pub fn group_transform_box(&self) -> Option<LayerTransform> {
        let document = self.document.as_ref()?;
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for id in self.group_transform_member_ids() {
            let layer = document.layer(id)?;
            for corner in layer.transform.corners() {
                min_x = min_x.min(corner.x);
                max_x = max_x.max(corner.x);
                min_y = min_y.min(corner.y);
                max_y = max_y.max(corner.y);
            }
        }
        if !min_x.is_finite() {
            return None;
        }
        Some(LayerTransform::new(
            CGPoint::new(min_x, min_y),
            CGSize::new((max_x - min_x).max(1.0), (max_y - min_y).max(1.0)),
        ))
    }

    pub fn nudge_layer(&mut self, dx: f64, dy: f64) {
        let already = self.transform_edit.is_some();
        if !already {
            // A nudge is a transient transform (nudgeLayer, EditorSession.swift:505).
            self.begin_transform_persistent(false);
        }
        if let Some(edit) = self.transform_edit.as_mut() {
            let mut value = edit.draft;
            value.origin.x += dx;
            value.origin.y += dy;
            self.preview_transform(value);
        }
        if !already {
            self.commit_transform();
        }
    }

    /// Flips the selected layer about its own middle.
    pub fn flip_layers(&mut self, horizontally: bool) {
        self.commit_transform();
        if !self.can_transform() {
            return;
        }
        // Several selected layers flip about the middle of the box around
        // them (flipLayers); one layer flips about its own middle.
        let members: Vec<Uuid> = if self.selected_layer_ids.len() > 1 {
            let Some(document) = self.document.clone() else {
                return;
            };
            self.selected_layer_ids
                .iter()
                .filter(|id| document.layer(**id).is_some())
                .copied()
                .collect()
        } else {
            match self.active_layer_id {
                Some(id) => vec![id],
                None => return,
            }
        };
        let Some(document) = self.document.clone() else {
            return;
        };
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for id in &members {
            let Some(layer) = document.layer(*id) else {
                continue;
            };
            let corners = layer.transform.corners();
            min_x = min_x.min(corners.iter().map(|p| p.x).fold(f64::INFINITY, f64::min));
            max_x = max_x.max(
                corners
                    .iter()
                    .map(|p| p.x)
                    .fold(f64::NEG_INFINITY, f64::max),
            );
            min_y = min_y.min(corners.iter().map(|p| p.y).fold(f64::INFINITY, f64::min));
            max_y = max_y.max(
                corners
                    .iter()
                    .map(|p| p.y)
                    .fold(f64::NEG_INFINITY, f64::max),
            );
        }
        if !members.is_empty() && min_x.is_finite() {
            let axis = if horizontally {
                (min_x + max_x) / 2.0
            } else {
                (min_y + max_y) / 2.0
            };
            self.finish_opacity_edit();
            self.begin_edit(if horizontally {
                "Flip Horizontal"
            } else {
                "Flip Vertical"
            });
            if let Some(doc) = self.document.as_mut() {
                for id in &members {
                    if let Some(layer) = doc.layer_mut(*id) {
                        let flipped = mirrored_transform(&layer.transform, horizontally, axis);
                        if let Some(mask) = layer.mask.as_mut() {
                            let old = layer.transform;
                            mask.placement = mask.placement_moving_layer(&old, &flipped);
                        }
                        layer.transform = flipped;
                    }
                }
            }
            self.end_edit();
        }
    }

    /// Flips the whole canvas.
    pub fn flip_canvas(&mut self, horizontally: bool) {
        self.commit_transform();
        self.cancel_crop();
        let Some(document) = self.document.clone() else {
            return;
        };
        let axis = if horizontally {
            document.size().width / 2.0
        } else {
            document.size().height / 2.0
        };
        self.finish_opacity_edit();
        self.begin_edit(if horizontally {
            "Flip Canvas Horizontal"
        } else {
            "Flip Canvas Vertical"
        });
        if let Some(doc) = self.document.as_mut() {
            for layer in doc.layers.iter_mut() {
                layer.transform = mirrored_transform(&layer.transform, horizontally, axis);
                if let Some(mask) = layer.mask.as_mut()
                    && let Some(placement) = mask.placement.as_mut()
                {
                    *placement = mirrored_transform(placement, horizontally, axis);
                }
            }
            // The selection mirrors with the canvas.
            if let Some(selection) = doc.selection.as_ref() {
                let mirror = if horizontally {
                    CGAffineTransform {
                        a: -1.0,
                        b: 0.0,
                        c: 0.0,
                        d: 1.0,
                        tx: document.size().width,
                        ty: 0.0,
                    }
                } else {
                    CGAffineTransform {
                        a: 1.0,
                        b: 0.0,
                        c: 0.0,
                        d: -1.0,
                        tx: 0.0,
                        ty: document.size().height,
                    }
                };
                doc.selection = Some(crate::selection::DocumentSelection {
                    path: selection.path.transformed(&mirror),
                    antialiased: selection.antialiased,
                    feather: selection.feather,
                });
            }
            doc.guides = doc
                .guides
                .iter()
                .map(|g| g.mirrored(horizontally, axis))
                .collect();
        }
        self.end_edit();
    }

    // MARK: Merge

    /// ⌘E: merges as the canvas shows them — blend modes, opacity, masks,
    /// clipping and adjustments baked in — into one pixel layer.
    /// `mergePlan` (LayerMerge.swift:8-30) — what ⌘E merges, in stacking
    /// order, and where the result goes; None when there is nothing to merge.
    /// One layer merges with the layer beneath it in the same folder; several
    /// selected layers merge together (with anything their folders hold); a
    /// folder merges its contents, and the folder goes.
    fn merge_plan(&self) -> Option<MergePlan> {
        if !self.can_edit_layers() {
            return None;
        }
        let document = self.document.as_ref()?;
        let active = self.active_layer()?;
        if self.selected_layer_ids.len() > 1 {
            let mut picked: std::collections::HashSet<Uuid> =
                self.selected_layer_ids.iter().copied().collect();
            for id in &self.selected_layer_ids {
                picked.extend(document.descendant_ids(*id));
            }
            let ordered: Vec<Uuid> = document
                .layers
                .iter()
                .filter(|l| picked.contains(&l.id))
                .map(|l| l.id)
                .collect();
            let top = document
                .layers
                .iter()
                .rev()
                .find(|l| self.selected_layer_ids.contains(&l.id))?;
            // The merge must include something with pixels of its own.
            if !ordered
                .iter()
                .any(|id| document.layer(*id).map(|l| !l.is_group).unwrap_or(false))
            {
                return None;
            }
            return Some(MergePlan {
                ids: ordered,
                removed: picked,
                name: top.name.clone(),
                parent: top.parent_id,
                anchor: top.id,
                action: "Merge Layers",
            });
        }
        if active.is_group {
            let inside = document.descendant_ids(active.id);
            // A folder merges its contents; an empty folder has nothing to merge.
            if !document
                .layers
                .iter()
                .any(|l| inside.contains(&l.id) && !l.is_group)
            {
                return None;
            }
            let ids: Vec<Uuid> = document
                .layers
                .iter()
                .filter(|l| inside.contains(&l.id) || l.id == active.id)
                .map(|l| l.id)
                .collect();
            return Some(MergePlan {
                removed: ids.iter().copied().collect(),
                ids,
                name: active.name.clone(),
                parent: active.parent_id,
                anchor: active.id,
                action: "Merge Group",
            });
        }
        let index = document.layer_index(active.id)?;
        // One layer merges with the nearest layer beneath it in the same
        // folder; a folder below refuses the merge (Swift never skips it).
        let below = document.layers[..index]
            .iter()
            .rev()
            .find(|l| l.parent_id == active.parent_id)?;
        if below.is_group {
            return None;
        }
        Some(MergePlan {
            ids: vec![below.id, active.id],
            removed: [below.id, active.id].into_iter().collect(),
            name: below.name.clone(),
            parent: active.parent_id,
            anchor: active.id,
            action: "Merge Down",
        })
    }

    /// `canMergeLayers` — whether ⌘E has anything to merge right now.
    pub fn can_merge_layers(&self) -> bool {
        self.merge_plan().is_some()
    }

    /// `mergeTitle` — the menu item's name for the pending merge.
    pub fn merge_title(&self) -> &'static str {
        self.merge_plan()
            .map(|plan| plan.action)
            .unwrap_or("Merge Down")
    }

    /// ⌘E: the layers composited as the canvas shows them — blend modes,
    /// opacity, masks, clipping and adjustments baked in — into one pixel
    /// layer, trimmed to what is there, in their place, as one undo step
    /// (LayerMerge.swift:65-108). A plan that would leave an unloadable
    /// hierarchy (as when a folder and its own contents are both selected)
    /// backs out silently, like Swift's `LayerHierarchy.validate` fallback.
    pub fn merge_layers(&mut self) {
        self.commit_transform();
        let Some(plan) = self.merge_plan() else {
            return;
        };
        let Some(document) = self.document.clone() else {
            return;
        };
        let kept: std::collections::HashSet<Uuid> = plan.ids.iter().copied().collect();

        // Composite the subset onto a canvas; only the merged layers, cut
        // loose from anything outside the merge.
        let mut subset_doc = document.clone();
        subset_doc.layers = document
            .layers
            .iter()
            .filter(|l| kept.contains(&l.id))
            .cloned()
            .collect();
        for layer in subset_doc.layers.iter_mut() {
            if let Some(parent) = layer.parent_id
                && !kept.contains(&parent)
            {
                layer.parent_id = None;
            }
            if let Some(source) = layer.mask_source_id
                && !kept.contains(&source)
            {
                layer.mask_source_id = None;
            }
        }
        let full = crate::render::draw_live_composite(&subset_doc);
        let canvas_transform = LayerTransform::new(
            CGPoint::ZERO,
            CGSize::new(document.width as f64, document.height as f64),
        );
        let (trimmed_image, trimmed_transform) = crate::filters::trimmed(&full, canvas_transform);
        let mut merged = ImageLayer::with_asset(
            ImportedImage::new(trimmed_image, plan.name.clone()),
            trimmed_transform.origin,
        );
        merged.transform = trimmed_transform;
        merged.parent_id = plan.parent;

        let mut next: Vec<ImageLayer> = document
            .layers
            .iter()
            .filter(|l| !plan.removed.contains(&l.id))
            .cloned()
            .collect();
        // Layers clipped to anything that was merged now clip to the result.
        for layer in next.iter_mut() {
            if let Some(source) = layer.mask_source_id
                && plan.removed.contains(&source)
            {
                layer.mask_source_id = Some(merged.id);
            }
        }
        let slot = document
            .layer_index(plan.anchor)
            .unwrap_or(document.layers.len());
        let insertion = slot
            - document.layers[..slot]
                .iter()
                .filter(|l| plan.removed.contains(&l.id))
                .count();
        let insertion = insertion.min(next.len());
        let merged_id = merged.id;
        next.insert(insertion, merged);

        if !hierarchy_is_valid(&next) {
            return;
        }
        self.finish_opacity_edit();
        self.begin_edit(plan.action);
        if let Some(doc) = self.document.as_mut() {
            doc.layers = next;
            self.active_layer_id = Some(merged_id);
            self.selected_layer_ids = vec![merged_id];
        }
        self.end_edit();
    }

    // MARK: Guides

    pub fn clear_guides(&mut self) {
        if self
            .document
            .as_ref()
            .map(|d| !d.guides.is_empty())
            .unwrap_or(false)
        {
            self.begin_edit("Clear Guides");
            if let Some(doc) = self.document.as_mut() {
                doc.guides.clear();
            }
            self.end_edit();
        }
    }

    pub fn add_guide(&mut self, axis: GuideAxis, position: f64) {
        self.shows_guides = true;
        self.begin_edit("New Guide");
        if let Some(doc) = self.document.as_mut() {
            doc.guides.push(CanvasGuide::new(axis, position));
        }
        self.end_edit();
    }

    // MARK: Guide interaction (Guides.swift)

    /// `hitGuide(at:)` — the nearest guide within `tolerance` document
    /// pixels of the point (x for vertical guides, y for horizontal).
    pub fn hit_guide(&self, at: CGPoint, tolerance: f64) -> Option<Uuid> {
        if !self.shows_guides || self.locks_guides {
            return None;
        }
        let document = self.document.as_ref()?;
        let mut best: Option<(Uuid, f64)> = None;
        for guide in &document.guides {
            let distance = if guide.axis == GuideAxis::Vertical {
                (at.x - guide.position).abs()
            } else {
                (at.y - guide.position).abs()
            };
            if distance <= tolerance && best.map(|(_, d)| distance < d).unwrap_or(true) {
                best = Some((guide.id, distance));
            }
        }
        best.map(|(id, _)| id)
    }

    /// `snappedGuidePosition` — snaps to other guides on the same axis, the
    /// canvas edges and middle, and other layers' edges and centers.
    pub fn snapped_guide_position(
        &self,
        position: f64,
        axis: GuideAxis,
        excluding: Option<Uuid>,
    ) -> f64 {
        if !self.snapping_enabled {
            return position;
        }
        let Some(document) = self.document.as_ref() else {
            return position;
        };
        let tolerance = 10.0;
        let length = if axis == GuideAxis::Vertical {
            document.width as f64
        } else {
            document.height as f64
        };
        let mut targets: Vec<f64> = vec![0.0, length / 2.0, length];
        for guide in &document.guides {
            if guide.axis == axis && Some(guide.id) != excluding {
                targets.push(guide.position);
            }
        }
        for layer in document.render_layers() {
            if layer.asset.is_none() {
                continue;
            }
            let corners = layer.transform.corners();
            let values: Vec<f64> = if axis == GuideAxis::Vertical {
                corners.iter().map(|p| p.x).collect()
            } else {
                corners.iter().map(|p| p.y).collect()
            };
            let min = values.iter().cloned().fold(f64::INFINITY, f64::min).round();
            let max = values
                .iter()
                .cloned()
                .fold(f64::NEG_INFINITY, f64::max)
                .round();
            targets.push(min);
            targets.push(((min + max) / 2.0).round());
            targets.push(max);
        }
        let mut best = position;
        let mut best_distance = tolerance;
        for target in targets {
            let distance = (target - position).abs();
            if distance <= best_distance {
                best = target;
                best_distance = distance;
            }
        }
        best
    }

    /// `beginGuideCreation(axis:at:)`.
    pub fn begin_guide_creation(&mut self, axis: GuideAxis, position: f64) {
        if !self.can_edit_layers() || self.locks_guides {
            return;
        }
        self.shows_guides = true;
        let snapped = self.snapped_guide_position(position, axis, None);
        let id = Uuid::new_v4();
        self.guide_drag = Some(GuideDrag {
            id,
            axis,
            position: snapped,
            is_new: true,
            original: None,
        });
    }

    /// `beginGuideMove(_:)`.
    pub fn begin_guide_move(&mut self, guide_id: Uuid) {
        if !self.can_edit_layers() || self.locks_guides {
            return;
        }
        let Some(guide) = self
            .document
            .as_ref()
            .and_then(|d| d.guides.iter().find(|g| g.id == guide_id))
            .cloned()
        else {
            return;
        };
        self.guide_drag = Some(GuideDrag {
            id: guide.id,
            axis: guide.axis,
            position: guide.position,
            is_new: false,
            original: Some(guide.position),
        });
    }

    /// `moveGuideDrag(to:)`.
    pub fn move_guide_drag(&mut self, position: f64) {
        let (axis, id) = match self.guide_drag.as_ref() {
            Some(drag) => (drag.axis, drag.id),
            None => return,
        };
        let snapped = self.snapped_guide_position(position, axis, Some(id));
        if let Some(drag) = self.guide_drag.as_mut() {
            drag.position = snapped;
        }
    }

    /// `finishGuideDrag(delete:)` — releasing on a ruler (`delete`) cancels a
    /// new guide or removes the existing one.
    pub fn finish_guide_drag(&mut self, delete: bool) {
        let Some(drag) = self.guide_drag.clone() else {
            return;
        };
        self.guide_drag = None;
        if delete {
            if drag.is_new {
                return;
            }
            self.begin_edit("Delete Guide");
            if let Some(doc) = self.document.as_mut() {
                doc.guides.retain(|g| g.id != drag.id);
            }
            self.end_edit();
            return;
        }
        if drag.is_new {
            self.begin_edit("New Guide");
            if let Some(doc) = self.document.as_mut() {
                doc.guides
                    .push(crate::document::CanvasGuide::new(drag.axis, drag.position));
            }
            self.end_edit();
        } else if drag.original != Some(drag.position) {
            self.begin_edit("Move Guide");
            if let Some(doc) = self.document.as_mut()
                && let Some(guide) = doc.guides.iter_mut().find(|g| g.id == drag.id)
            {
                guide.position = drag.position;
            }
            self.end_edit();
        }
    }

    /// `cancelGuideDrag`.
    pub fn cancel_guide_drag(&mut self) {
        self.guide_drag = None;
    }
}

/// `FloatingMerge.merge`: draws the floating pixels (with their transform)
/// onto the source layer's own pixel grid, growing the layer where they now
/// extend past it. A mask without its own placement grows with it, revealing
/// the new area. Returns the merged asset, transform, and mask.
fn floating_merge(
    pixels: &Bitmap,
    transform: &LayerTransform,
    source: &ImageLayer,
) -> Option<(ImportedImage, LayerTransform, Option<LayerMask>)> {
    let source_image = source.asset.as_ref()?.image.as_ref();
    let width = source_image.width;
    let height = source_image.height;
    let to_document =
        crate::raster::pixel_to_document(&source.transform, width as f64, height as f64);
    let to_pixels = to_document.inverted();
    let pw = pixels.width as f64;
    let ph = pixels.height as f64;
    // Where the floating pixels land, in the source's pixel grid.
    let floating_bounds = CGRect::new(0.0, 0.0, pw, ph)
        .transformed(&crate::raster::pixel_to_document(transform, pw, ph))
        .transformed(&to_pixels);
    let original = CGRect::new(0.0, 0.0, width as f64, height as f64);
    let extent = original.union(&floating_bounds).integral();
    if extent.width() < 1.0
        || extent.height() < 1.0
        || extent.width() > 30_000.0
        || extent.height() > 30_000.0
        || extent.width() * extent.height() > 100_000_000.0
    {
        return None;
    }
    let (min_x, min_y) = (extent.min_x(), extent.min_y());
    let placed = original.offset_by(-min_x, -min_y);
    let mut canvas = Bitmap::new(extent.width() as usize, extent.height() as usize);
    crate::raster::draw_over(&mut canvas, source_image, placed, LayerSampling::High);
    // The floating pixels: grid → document → the grown source grid.
    let mapping = CGAffineTransform::translation(-min_x, -min_y)
        .concat(&to_pixels)
        .concat(&crate::raster::pixel_to_document(transform, pw, ph));
    crate::raster::draw_affine(&mut canvas, pixels, &mapping, transform.sampling, None);
    let asset = ImportedImage::new(canvas, source.name.clone());
    let mut merged = source.transform;
    merged.size = CGSize::new(
        extent.width() * source.transform.size.width / width as f64,
        extent.height() * source.transform.size.height / height as f64,
    );
    let center = CGPoint::new(extent.mid_x(), extent.mid_y()).applying(&to_document);
    merged.origin = CGPoint::new(
        center.x - merged.size.width / 2.0,
        center.y - merged.size.height / 2.0,
    );
    let mut mask = source.mask.clone();
    if let Some(current) = source.mask.as_ref()
        && current.placement.is_none()
        && extent != original
    {
        // Reveal the grown area: white behind the old mask.
        let mut grown = Bitmap::new(extent.width() as usize, extent.height() as usize);
        for chunk in grown.data.chunks_exact_mut(4) {
            chunk[0] = 255;
            chunk[1] = 255;
            chunk[2] = 255;
            chunk[3] = 255;
        }
        crate::raster::draw_over(
            &mut grown,
            current.asset.image.as_ref(),
            placed,
            LayerSampling::High,
        );
        mask = Some(current.replacing(ImportedImage::new(grown, "Layer Mask")));
    }
    Some((asset, merged, mask))
}

/// `LiveLayerMask.adoptClipping` — after a move, a layer rejoins the clipping
/// stack that now sits right below it in the same group.
fn adopt_clipping(id: Uuid, layers: &mut [ImageLayer]) {
    let position = match layers.iter().position(|l| l.id == id) {
        Some(position) if !layers[position].is_group => position,
        _ => return,
    };
    let parent = layers[position].parent_id;
    let siblings: Vec<&ImageLayer> = layers.iter().filter(|l| l.parent_id == parent).collect();
    let index = match siblings.iter().position(|l| l.id == id) {
        Some(index) if index > 0 && index + 1 < siblings.len() => index,
        _ => return,
    };
    let source = match siblings[index + 1].mask_source_id {
        Some(source) if source != id => source,
        _ => return,
    };
    let below = siblings[index - 1];
    if below.id == source || below.mask_source_id == Some(source) {
        layers[position].mask_source_id = Some(source);
    }
}

/// `LiveLayerMask.releaseDetachedClipping` — clipping links whose base no
/// longer sits directly below them release.
fn release_detached_clipping(layers: &mut [ImageLayer]) {
    let mut groups: std::collections::HashMap<Option<Uuid>, Vec<Uuid>> =
        std::collections::HashMap::new();
    for layer in layers.iter() {
        groups.entry(layer.parent_id).or_default().push(layer.id);
    }
    let by_id: std::collections::HashMap<Uuid, &ImageLayer> =
        layers.iter().map(|l| (l.id, l)).collect();
    let mut release: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    for stack in groups.values() {
        let mut base: Option<Uuid> = None;
        for id in stack {
            let layer = by_id.get(id).expect("own list");
            if let Some(source) = layer.mask_source_id {
                if Some(source) != base {
                    release.insert(*id);
                    base = Some(*id);
                }
            } else {
                base = if layer.is_group { None } else { Some(*id) };
            }
        }
    }
    for layer in layers.iter_mut() {
        if release.contains(&layer.id) {
            layer.mask_source_id = None;
        }
    }
}

/// `ImageImporter.decode`: an image file into a premultiplied canvas bitmap
/// plus its thumbnail.
fn decode_image(path: &std::path::Path) -> Option<(Bitmap, Bitmap)> {
    // Camera RAW develops first (RawImporter): the CIRAWFilter output is
    // opaque 8-bit sRGB, so the premultiply below is a no-op.
    let mut bitmap = if crate::raw::is_raw(path) {
        crate::raw::decode_raw(path)?
    } else {
        let rgba = image::open(path).ok()?.to_rgba8();
        let (width, height) = (rgba.width() as usize, rgba.height() as usize);
        let mut bitmap = Bitmap::new(width, height);
        bitmap.data = rgba.into_raw();
        bitmap
    };
    for pixel in bitmap.data.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        for channel in pixel[..3].iter_mut() {
            *channel = ((*channel as u32 * alpha + 127) / 255) as u8;
        }
    }
    let thumbnail = crate::raster::thumbnail_of(&bitmap);
    Some((bitmap, thumbnail))
}

/// A fresh pseudo-random seed for noise and grain (`UInt32.random`).
fn rand_seed() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u32)
        .unwrap_or(0)
        ^ 0x9e3779b9
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::LayerEffectKind;

    fn session() -> EditorSession {
        let mut s = EditorSession::new();
        s.create_document(100, 100, true);
        s
    }

    fn red_image(size: usize) -> Bitmap {
        let mut image = Bitmap::new(size, size);
        for i in (0..image.data.len()).step_by(4) {
            image.data[i] = 255;
            image.data[i + 3] = 255;
        }
        image
    }

    #[test]
    fn new_canvas_starts_with_layer_1() {
        let s = session();
        assert_eq!(s.document.as_ref().unwrap().layers[0].name, "Layer 1");
        assert_eq!(
            s.active_layer_id,
            Some(s.document.as_ref().unwrap().layers[0].id)
        );
        assert!(s.is_modified());
    }

    #[test]
    fn blank_layers_number_sequentially() {
        let mut s = session();
        s.add_blank_layer();
        assert_eq!(s.document.as_ref().unwrap().layers.len(), 2);
        assert_eq!(s.document.as_ref().unwrap().layers[1].name, "Layer 2");
    }

    #[test]
    fn undo_reverts_layer_add() {
        let mut s = session();
        s.add_blank_layer();
        assert_eq!(s.document.as_ref().unwrap().layers.len(), 2);
        s.undo();
        assert_eq!(s.document.as_ref().unwrap().layers.len(), 1);
        s.redo();
        assert_eq!(s.document.as_ref().unwrap().layers.len(), 2);
    }

    #[test]
    fn delete_layer_removes_descendants() {
        let mut s = session();
        let group_id = s.document.as_ref().unwrap().layers[0].id;
        s.document.as_mut().unwrap().layers[0].is_group = true;
        let mut child = ImageLayer::blank("Child", CGSize::new(100.0, 100.0));
        child.parent_id = Some(group_id);
        let child_id = child.id;
        s.document.as_mut().unwrap().layers.push(child);
        s.select_layer(Some(child_id));
        s.delete_layer(child_id);
        assert_eq!(s.document.as_ref().unwrap().layers.len(), 1);
        s.select_layer(Some(group_id));
        s.delete_layer(group_id);
        assert!(s.document.as_ref().unwrap().layers.is_empty());
    }

    #[test]
    fn deleting_the_active_layer_selects_a_neighbor() {
        // LayerTests.swift:27: the selection lands on layers[min(index, count-1)].
        let mut s = session();
        s.add_blank_layer();
        s.add_blank_layer();
        let ids: Vec<Uuid> = s
            .document
            .as_ref()
            .unwrap()
            .layers
            .iter()
            .map(|l| l.id)
            .collect();
        s.select_layer(Some(ids[1]));
        s.delete_layer(ids[1]);
        assert_eq!(s.active_layer_id, Some(ids[2]));
        // Deleting the last layer moves the selection up to the new last one.
        s.select_layer(Some(ids[2]));
        s.delete_layer(ids[2]);
        assert_eq!(
            s.active_layer_id,
            s.document.as_ref().unwrap().layers.last().map(|l| l.id)
        );
    }

    #[test]
    fn rename_and_visibility_are_undoable() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        s.rename_layer(id, "Base");
        s.toggle_layer_visibility(id);
        assert_eq!(s.document.as_ref().unwrap().layers[0].name, "Base");
        assert!(!s.document.as_ref().unwrap().layers[0].is_visible);
        s.undo();
        s.undo();
        assert_eq!(s.document.as_ref().unwrap().layers[0].name, "Layer 1");
        assert!(s.document.as_ref().unwrap().layers[0].is_visible);
    }

    #[test]
    fn no_op_edit_preserves_history() {
        let mut s = session();
        let undo_count = s.history.undo_count();
        let id = s.active_layer_id.unwrap();
        s.rename_layer(id, "");
        assert_eq!(s.history.undo_count(), undo_count);
    }

    #[test]
    fn blend_mode_cycles_through_menu_order() {
        let mut s = session();
        s.cycle_blend_mode(true);
        assert_eq!(s.active_layer().unwrap().blend_mode, LayerBlendMode::Darken);
        s.cycle_blend_mode(false);
        assert_eq!(s.active_layer().unwrap().blend_mode, LayerBlendMode::Normal);
        s.cycle_blend_mode(false);
        assert_eq!(
            s.active_layer().unwrap().blend_mode,
            LayerBlendMode::Luminosity
        );
    }

    #[test]
    fn brush_hardness_snaps_to_quarter_steps() {
        // EditorSession.changeBrushHardness: 80% steps to 100% up, 75% down.
        let mut s = session();
        s.brush_settings.hardness = 0.8;
        s.change_brush_hardness(true);
        assert_eq!(s.brush_settings.hardness, 1.0);
        s.brush_settings.hardness = 0.8;
        s.change_brush_hardness(false);
        assert_eq!(s.brush_settings.hardness, 0.75);
        // Exact steps stay put within rounding slop.
        s.brush_settings.hardness = 0.5;
        s.change_brush_hardness(true);
        assert_eq!(s.brush_settings.hardness, 0.75);
        // Both ends clamp.
        s.brush_settings.hardness = 0.0;
        s.change_brush_hardness(false);
        assert_eq!(s.brush_settings.hardness, 0.0);
        s.brush_settings.hardness = 1.0;
        s.change_brush_hardness(true);
        assert_eq!(s.brush_settings.hardness, 1.0);
        // Off-grid low values round down, never up, when decreasing.
        s.brush_settings.hardness = 0.1;
        s.change_brush_hardness(false);
        assert_eq!(s.brush_settings.hardness, 0.0);
    }

    #[test]
    fn opacity_digits_set_the_active_tool_opacity() {
        // EditorSession+Brush.swift:191-208 — 1…9 = 10%…90%, 0 = 100%.
        let mut s = session();
        s.tool = NavigationTool::Brush;
        s.type_opacity_digit(4, 10.0);
        assert_eq!(s.brush_settings.opacity, 0.4);
        s.type_opacity_digit(0, 11.0);
        assert_eq!(s.brush_settings.opacity, 1.0);
        // Two digits within 0.6 s: 4 then 5 = 45%, and 0 then 5 = 5% (min 1%).
        s.type_opacity_digit(4, 20.0);
        s.type_opacity_digit(5, 20.3);
        assert_eq!(s.brush_settings.opacity, 0.45);
        s.type_opacity_digit(0, 30.0);
        s.type_opacity_digit(5, 30.4);
        assert_eq!(s.brush_settings.opacity, 0.05);
        // A second digit past the window starts a new pair.
        s.type_opacity_digit(7, 40.0);
        s.type_opacity_digit(2, 40.9);
        assert_eq!(s.brush_settings.opacity, 0.2, "7 stands alone, 2 restarts");
        // The gradient tool steps its own opacity.
        s.tool = NavigationTool::Gradient;
        s.type_opacity_digit(3, 50.0);
        assert_eq!(s.gradient_settings.opacity, 0.3);
        // Any other tool family is untouched; wrong digits no-op.
        s.tool = NavigationTool::Marquee;
        let before = s.brush_settings.opacity;
        s.type_opacity_digit(5, 60.0);
        assert_eq!(s.brush_settings.opacity, before);
        s.tool = NavigationTool::Brush;
        s.type_opacity_digit(11, 61.0);
        s.type_opacity_digit(-1, 61.0);
        assert_eq!(s.brush_settings.opacity, before);
    }

    #[test]
    fn move_tool_digits_set_every_selected_layers_opacity() {
        // LayerAppearance.swift:108-119 — one undo step for all selected.
        let mut s = session();
        s.add_blank_layer();
        s.tool = NavigationTool::Move;
        let first = s.document.as_ref().unwrap().layers[0].id;
        let second = s.document.as_ref().unwrap().layers[1].id;
        s.selected_layer_ids = vec![first, second];
        let before = s.history.undo_count();
        s.type_opacity_digit(5, 10.0);
        assert_eq!(s.active_layer().unwrap().opacity, 0.5);
        let doc = s.document.as_ref().unwrap();
        assert_eq!(
            doc.layers
                .iter()
                .filter(|l| l.id == first || l.id == second)
                .map(|l| l.opacity)
                .collect::<Vec<_>>(),
            vec![0.5, 0.5]
        );
        assert_eq!(s.history.undo_count(), before + 1);
        assert_eq!(s.history.undo_name(), "Layer Opacity");
    }

    #[test]
    fn opacity_edit_folds_into_one_step() {
        let mut s = session();
        let before = s.history.undo_count();
        s.begin_opacity_edit();
        s.set_layer_opacity(0.3);
        s.set_layer_opacity(0.7);
        s.finish_opacity_edit();
        assert!((s.active_layer().unwrap().opacity - 0.7).abs() < 1e-9);
        assert_eq!(s.history.undo_count(), before + 1);
        assert_eq!(s.history.undo_name(), "Layer Opacity");
    }

    #[test]
    fn marquee_selects_and_composes() {
        let mut s = session();
        s.tool = NavigationTool::Marquee;
        s.begin_lasso(CGPoint::new(10.0, 10.0), SelectionMode::Replace);
        s.drag_marquee(CGPoint::new(40.0, 30.0), false, false);
        s.finish_lasso();
        let sel = s.selection().unwrap();
        assert_eq!(sel.path.bounds(), Some(CGRect::new(10.0, 10.0, 30.0, 20.0)));
        s.begin_lasso(CGPoint::new(20.0, 20.0), SelectionMode::Add);
        s.drag_marquee(CGPoint::new(50.0, 40.0), false, false);
        s.finish_lasso();
        assert_eq!(
            s.selection().unwrap().path.bounds(),
            Some(CGRect::new(10.0, 10.0, 40.0, 30.0))
        );
        s.deselect();
        assert!(s.selection().is_none());
        s.undo();
        assert!(s.selection().is_some());
    }

    #[test]
    fn lasso_click_enclosing_nothing_deselects() {
        let mut s = session();
        s.select_all();
        s.tool = NavigationTool::Lasso;
        s.begin_lasso(CGPoint::new(50.0, 50.0), SelectionMode::Replace);
        s.finish_lasso();
        assert!(s.selection().is_none());
    }

    #[test]
    fn expand_contract_and_feather() {
        let mut s = session();
        s.tool = NavigationTool::Marquee;
        s.begin_lasso(CGPoint::new(20.0, 20.0), SelectionMode::Replace);
        s.drag_marquee(CGPoint::new(40.0, 40.0), false, false);
        s.finish_lasso();
        s.expand_selection(5);
        assert_eq!(
            s.selection().unwrap().path.bounds(),
            Some(CGRect::new(15.0, 15.0, 30.0, 30.0))
        );
        s.contract_selection(5);
        assert_eq!(
            s.selection().unwrap().path.bounds(),
            Some(CGRect::new(20.0, 20.0, 20.0, 20.0))
        );
        s.feather_selection(4);
        assert!((s.selection().unwrap().feather - 4.0).abs() < 1e-9);
        s.feather_selection(3);
        let expected = (16.0f64 + 9.0).sqrt();
        assert!((s.selection().unwrap().feather - expected).abs() < 1e-9);
    }

    #[test]
    fn invert_selection() {
        let mut s = session();
        s.tool = NavigationTool::Marquee;
        s.begin_lasso(CGPoint::new(10.0, 10.0), SelectionMode::Replace);
        s.drag_marquee(CGPoint::new(20.0, 20.0), false, false);
        s.finish_lasso();
        s.invert_selection();
        let sel = s.selection().unwrap();
        assert!(!sel.path.contains(CGPoint::new(15.0, 15.0)));
        assert!(sel.path.contains(CGPoint::new(50.0, 50.0)));
    }

    /// A selection outline move shifts the path only, one undo step
    /// (Selection.swift:263-290).
    #[test]
    fn selection_outline_move_offsets_and_commits() {
        let mut s = session();
        s.tool = NavigationTool::Marquee;
        s.begin_lasso(CGPoint::new(10.0, 10.0), SelectionMode::Replace);
        s.drag_marquee(CGPoint::new(30.0, 30.0), false, false);
        s.finish_lasso();
        assert!(s.begin_selection_move());
        // A second begin is refused while one is open.
        assert!(!s.begin_selection_move());
        s.move_selection_outline(CGSize::new(10.4, -7.6));
        let moved = s.selection().unwrap();
        // (10.4, -7.6) rounds to (10, -8): the rect now spans (20,2)-(40,22).
        assert!(
            moved.path.contains(CGPoint::new(30.0, 12.0)),
            "offset by whole pixels"
        );
        assert!(
            !moved.path.contains(CGPoint::new(15.0, 25.0)),
            "the old spot is vacated"
        );
        let undo_count = s.history.undo_count();
        s.end_selection_move();
        assert_eq!(s.history.undo_count(), undo_count + 1);
        assert!(s.selection_move_origin.is_none());
    }

    /// Cmd-click on a layer thumbnail: the ≥ 50% opaque pixels become the
    /// selection, ignoring the mask (MaskTracing.swift:87).
    #[test]
    fn load_layer_selection_traces_opaque_pixels() {
        let mut s = session();
        let mut image = Bitmap::new(100, 100);
        for y in 0..100 {
            for x in 0..100 {
                let alpha = if x < 50 { 255 } else { 0 };
                image.set_pixel(x, y, [255, 0, 0, alpha]);
            }
        }
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(image, "L"));
        s.load_layer_selection(s.active_layer_id.unwrap(), SelectionMode::Replace);
        let sel = s.selection().expect("opaque half traced");
        assert!(sel.path.contains(CGPoint::new(25.0, 50.0)));
        assert!(!sel.path.contains(CGPoint::new(75.0, 50.0)));
    }

    /// Cmd-click on a mask thumbnail: the mask's black (hidden) areas become
    /// the selection (MaskTracing.swift:76).
    #[test]
    fn load_mask_selection_traces_dark_pixels() {
        let mut s = session();
        let layer_id = s.active_layer_id.unwrap();
        let mut gray = Bitmap::new(100, 100);
        for y in 0..100 {
            for x in 0..100 {
                // Black on the left (hidden → selected), white on the right.
                let v = if x < 50 { 0 } else { 255 };
                gray.set_pixel(x, y, [v, v, v, 255]);
            }
        }
        let layer = s.document.as_mut().unwrap().layer_mut(layer_id).unwrap();
        layer.mask = Some(crate::document::LayerMask {
            asset: ImportedImage::new(gray, "mask"),
            is_enabled: true,
            placement: None,
            is_linked: true,
        });
        s.load_mask_selection(layer_id, SelectionMode::Replace);
        let sel = s.selection().expect("dark half traced");
        assert!(sel.path.contains(CGPoint::new(25.0, 50.0)));
        assert!(!sel.path.contains(CGPoint::new(75.0, 50.0)));
    }

    /// An all-revealing mask has no dark pixels: the load is refused with a
    /// status instead of clearing the selection (the Swift beep).
    #[test]
    fn load_mask_selection_without_dark_pixels_keeps_the_selection() {
        let mut s = session();
        let layer_id = s.active_layer_id.unwrap();
        let layer = s.document.as_mut().unwrap().layer_mut(layer_id).unwrap();
        layer.mask = Some(crate::document::LayerMask::solid(true));
        s.select_all();
        let before = s.selection().unwrap().path.clone();
        s.load_mask_selection(layer_id, SelectionMode::Replace);
        assert!(s.brush_error.is_some());
        assert_eq!(s.selection().unwrap().path, before);
    }

    #[test]
    fn magic_wand_selects_uniform_region() {
        let mut s = session();
        let mut image = red_image(100);
        for y in 0..50 {
            for x in 0..50 {
                image.set_pixel(x, y, [0, 255, 0, 255]);
            }
        }
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(image, "L"));
        s.magic_wand(CGPoint::new(10.0, 10.0), SelectionMode::Replace);
        let sel = s.selection().unwrap();
        assert!(sel.path.contains(CGPoint::new(10.0, 10.0)));
        assert!(!sel.path.contains(CGPoint::new(75.0, 75.0)));
    }

    #[test]
    fn add_and_delete_mask() {
        let mut s = session();
        s.add_layer_mask(true);
        assert!(s.active_layer().unwrap().mask.is_some());
        assert!(s.is_mask_selected);
        s.toggle_layer_mask();
        assert!(!s.active_layer().unwrap().mask.as_ref().unwrap().is_enabled);
        s.delete_layer_mask();
        assert!(s.active_layer().unwrap().mask.is_none());
    }

    #[test]
    fn undo_keeps_the_mask_target_on_the_same_layer() {
        // EditorSession.restore: an undo that keeps editing the same layer
        // keeps the mask as the target, backwards and forwards.
        let mut s = session();
        s.add_layer_mask(true);
        let id = s.active_layer_id.unwrap();
        assert!(s.is_mask_selected);
        s.rename_layer(id, "Renamed");
        s.undo();
        assert!(s.is_mask_selected, "same layer, still its mask");
        assert!(s.active_layer().unwrap().mask.is_some());
        s.redo();
        assert!(s.is_mask_selected);
    }

    #[test]
    fn undo_drops_the_mask_target_when_the_layer_changes() {
        let mut s = session();
        s.add_blank_layer();
        let layer2 = s.active_layer_id.unwrap();
        // Outside any undo step: Layer 2's mask becomes the target.
        if let Some(doc) = s.document.as_mut() {
            doc.layer_mut(layer2).unwrap().mask = Some(LayerMask::solid(true));
        }
        s.is_mask_selected = true;
        s.undo();
        // The undo lands back on Layer 1 — another layer, without a mask.
        assert_ne!(s.active_layer_id, Some(layer2));
        assert!(!s.is_mask_selected);
        assert!(s.active_layer().unwrap().mask.is_none());
    }

    #[test]
    fn undo_drops_a_mask_target_the_restored_layer_does_not_have() {
        let mut s = session();
        s.add_layer_mask(true);
        assert!(s.is_mask_selected);
        s.undo();
        assert!(!s.is_mask_selected, "the restored layer has no mask yet");
        assert!(s.active_layer().unwrap().mask.is_none());
    }

    #[test]
    fn undo_and_redo_cancel_the_crop_and_gradient_drafts() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        s.rename_layer(id, "Renamed");
        s.crop_rect = Some(CGRect::new(0.0, 0.0, 40.0, 40.0));
        s.gradient_edit = Some(GradientDraft {
            layer_id: id,
            is_mask: false,
            start: CGPoint::ZERO,
            end: CGPoint::new(10.0, 10.0),
        });
        // Like Photoshop, the first Undo only discards the pending gradient;
        // it does not also consume a history entry (EditorSession.swift:569).
        s.undo();
        assert!(s.gradient_edit.is_none(), "the pending gradient dies first");
        assert_eq!(
            s.crop_rect,
            Some(CGRect::new(0.0, 0.0, 40.0, 40.0)),
            "the history step is not consumed by the gradient discard"
        );
        s.undo();
        assert_eq!(s.crop_rect, None, "the crop draft dies with the undo");
        s.crop_rect = Some(CGRect::new(0.0, 0.0, 40.0, 40.0));
        s.redo();
        assert_eq!(s.crop_rect, None);
    }

    #[test]
    fn undo_is_blocked_while_a_transform_edit_is_open() {
        // canUseHistory (EditorSession.swift:563-567): undoing mid-transform
        // would swap the document out and let commit_transform write the
        // stale draft into the restored one.
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(50, 50), "L"));
        s.rename_layer(s.active_layer_id.unwrap(), "Renamed");
        s.begin_transform_persistent(true);
        assert!(s.transform_edit.is_some());
        assert!(!s.can_undo(), "a busy transform blocks undo");
        assert!(!s.can_redo(), "and redo");
        s.undo();
        assert_eq!(
            s.active_layer().unwrap().name,
            "Renamed",
            "the blocked undo changed nothing"
        );
        s.cancel_transform();
        assert!(s.can_undo(), "undo works again once the transform closes");
        s.undo();
        assert_ne!(s.active_layer().unwrap().name, "Renamed");
    }

    #[test]
    fn select_layer_target_switches_pixels_and_mask() {
        let mut s = session();
        s.add_layer_mask(true);
        let id = s.active_layer_id.unwrap();
        s.is_mask_selected = false;
        s.select_layer_target(id, true);
        assert!(s.is_mask_selected);
        s.select_layer_target(id, false);
        assert!(!s.is_mask_selected);
        // A target without a mask never selects one.
        s.add_blank_layer();
        let other = s.active_layer_id.unwrap();
        s.select_layer(Some(id));
        s.select_layer_target(other, true);
        assert!(!s.is_mask_selected);
        assert_eq!(s.active_layer_id, Some(other));
    }

    #[test]
    fn select_layer_target_leaves_the_effects_panel() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        s.effect_selection = Some(crate::effects::LayerEffectSelection {
            layer_id: id,
            kind: crate::effects::LayerEffectKind::Stroke,
        });
        s.select_layer_target(id, false);
        assert_eq!(s.effect_selection, None);
    }

    #[test]
    fn copy_mask_moves_a_mask_between_layers() {
        let mut s = session();
        s.add_blank_layer();
        let source = s.active_layer_id.unwrap();
        s.add_layer_mask(true);
        // The source's mask sits apart, unlinked; the copy carries the spot.
        if let Some(doc) = s.document.as_mut() {
            let mask = doc.layer_mut(source).unwrap().mask.as_mut().unwrap();
            mask.is_linked = false;
            mask.placement = Some(LayerTransform::new(
                CGPoint::new(10.0, 20.0),
                CGSize::new(100.0, 100.0),
            ));
        }
        let target = s.document.as_ref().unwrap().layers[0].id;
        assert!(s.can_copy_mask(source, target));
        s.copy_mask(source, target);
        assert_eq!(s.history.undo_name(), "Copy Layer Mask");
        assert_eq!(s.active_layer_id, Some(target));
        assert!(s.is_mask_selected);
        let copied = s
            .document
            .as_ref()
            .unwrap()
            .layer(target)
            .unwrap()
            .mask
            .clone()
            .unwrap();
        assert_eq!(
            copied.placement.map(|p| p.origin),
            Some(CGPoint::new(10.0, 20.0))
        );
        assert!(!copied.is_linked);
        // Dropping it again replaces what the layer had. The copy has to
        // actually change the document to be recorded (identical masks fold
        // away, by image identity, exactly as in Swift), so nudge it first.
        if let Some(doc) = s.document.as_mut() {
            let mask = doc.layer_mut(target).unwrap().mask.as_mut().unwrap();
            mask.placement = Some(LayerTransform::new(
                CGPoint::new(50.0, 50.0),
                CGSize::new(100.0, 100.0),
            ));
        }
        s.copy_mask(source, target);
        assert_eq!(s.history.undo_name(), "Replace Layer Mask");
        assert!(
            !s.can_copy_mask(source, source),
            "a layer cannot copy from itself"
        );
    }

    #[test]
    fn copy_mask_refuses_folders_and_missing_masks() {
        let mut s = session();
        s.add_layer_mask(true);
        let source = s.active_layer_id.unwrap();
        s.add_group();
        let folder = s.active_layer_id.unwrap();
        assert!(!s.can_copy_mask(source, folder), "a folder takes no mask");
        let count = s.history.undo_count();
        s.copy_mask(source, folder);
        assert_eq!(
            s.history.undo_count(),
            count,
            "nothing merged, nothing recorded"
        );
        assert!(
            !s.can_copy_mask(folder, source),
            "the folder has no mask to give"
        );
    }

    #[test]
    fn toggle_mask_link_names_the_edit() {
        let mut s = session();
        s.add_layer_mask(true);
        let id = s.active_layer_id.unwrap();
        assert!(s.active_layer().unwrap().mask.as_ref().unwrap().is_linked);
        s.toggle_mask_link(id);
        assert_eq!(s.history.undo_name(), "Unlink Layer Mask");
        assert!(!s.active_layer().unwrap().mask.as_ref().unwrap().is_linked);
        s.toggle_mask_link(id);
        assert_eq!(s.history.undo_name(), "Link Layer Mask");
        assert!(s.active_layer().unwrap().mask.as_ref().unwrap().is_linked);
        // A layer without a mask is a no-op.
        s.add_blank_layer();
        let other = s.active_layer_id.unwrap();
        let count = s.history.undo_count();
        s.toggle_mask_link(other);
        assert_eq!(s.history.undo_count(), count);
    }

    #[test]
    fn displayed_mask_placement_follows_the_pending_transform() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(50, 50), "L"));
        s.add_layer_mask(true);
        let id = s.active_layer_id.unwrap();
        // A real-sized mask: a uniform 1 × 1 one never takes a placement.
        if let Some(doc) = s.document.as_mut() {
            let layer = doc.layer_mut(id).unwrap();
            let mut mask = layer.mask.take().unwrap();
            mask.asset = ImportedImage::new(Bitmap::new(50, 50), "Layer Mask");
            layer.mask = Some(mask);
        }
        let layer = |s: &EditorSession| s.document.as_ref().unwrap().layer(id).unwrap().clone();

        // With nothing pending, the mask just covers the layer.
        assert_eq!(s.displayed_mask_placement(&layer(&s)), None);

        // Linked: wherever the layer is headed, the mask keeps covering it.
        s.begin_transform();
        if let Some(edit) = s.transform_edit.as_mut() {
            edit.draft.origin = CGPoint::new(30.0, 0.0);
        }
        assert_eq!(
            s.displayed_mask_placement(&layer(&s)),
            None,
            "linked stays covering"
        );
        s.cancel_transform();

        // Unlinked: the mask stays where it was while the layer drags, and the
        // commit gives it that spot. The pixels are the target again first,
        // like the Swift test's `selectLayerTarget(layer.id, mask: false)`.
        s.toggle_mask_link(id);
        s.select_layer_target(id, false);
        s.begin_transform();
        if let Some(edit) = s.transform_edit.as_mut() {
            edit.draft.origin = CGPoint::new(30.0, 0.0);
        }
        let shown = s.displayed_mask_placement(&layer(&s)).unwrap();
        assert_eq!(
            shown.origin,
            CGPoint::ZERO,
            "the mask stays while the layer drags"
        );
        s.commit_transform();
        assert_eq!(
            layer(&s).mask.as_ref().unwrap().placement.map(|p| p.origin),
            Some(CGPoint::ZERO)
        );

        // A pending mask transform shows its draft.
        s.is_mask_selected = true;
        s.begin_transform();
        assert!(s.transform_edit.as_ref().unwrap().mask);
        if let Some(edit) = s.transform_edit.as_mut() {
            edit.draft.origin = CGPoint::new(15.0, 0.0);
        }
        assert_eq!(
            s.displayed_mask_placement(&layer(&s)).map(|p| p.origin),
            Some(CGPoint::new(15.0, 0.0))
        );
        s.commit_transform();
        assert_eq!(
            layer(&s).mask.as_ref().unwrap().placement.map(|p| p.origin),
            Some(CGPoint::new(15.0, 0.0))
        );
    }

    #[test]
    fn displayed_mask_placement_under_a_distortion() {
        // A distortion carries a linked covering mask with the layer (it keeps
        // covering, so None); any other mask stays put.
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(50, 50), "L"));
        s.add_layer_mask(true);
        let id = s.active_layer_id.unwrap();
        let draft = LayerTransform::new(CGPoint::new(5.0, 5.0), CGSize::new(60.0, 60.0));
        s.transform_edit = Some(TransformEdit {
            layer_id: id,
            draft,
            persistent: true,
            floating: None,
            corners: Some(crate::tools::distort::corners_of(&draft)),
            mask: false,
            group: None,
        });
        let layer = s.document.as_ref().unwrap().layer(id).unwrap().clone();
        assert_eq!(
            s.displayed_mask_placement(&layer),
            None,
            "linked keeps covering"
        );
        let unlinked_layer = {
            let mut l = layer.clone();
            let mask = l.mask.as_mut().unwrap();
            mask.is_linked = false;
            l
        };
        assert_eq!(
            s.displayed_mask_placement(&unlinked_layer),
            Some(layer.transform),
            "an unlinked covering mask stays at the layer's transform"
        );
    }

    #[test]
    fn transform_moves_layer_as_one_edit() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(50, 50), "L"));
        s.begin_transform();
        assert!(s.transform_edit.is_some());
        if let Some(edit) = s.transform_edit.as_mut() {
            edit.draft.origin = CGPoint::new(10.0, 5.0);
        }
        s.commit_transform();
        let layer = s.document.as_ref().unwrap().layer(id).unwrap();
        assert_eq!(layer.transform.origin, CGPoint::new(10.0, 5.0));
        assert_eq!(s.history.undo_name(), "Transform Layer");
    }

    #[test]
    fn mask_only_transform_moves_the_placement() {
        // An unlinked mask dragged on its own takes the placement; the layer
        // stays put and the undo step is named for the mask.
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(50, 50), "L"));
        {
            let doc = s.document.as_mut().unwrap();
            let layer = doc.layer_mut(id).unwrap();
            let mut mask = LayerMask::solid(true);
            mask.is_linked = false;
            layer.mask = Some(mask);
        }
        s.is_mask_selected = true;
        s.begin_transform();
        assert!(s.transform_edit.is_some());
        if let Some(edit) = s.transform_edit.as_mut() {
            edit.draft.origin = CGPoint::new(10.0, 5.0);
        }
        s.commit_transform();
        let layer = s.document.as_ref().unwrap().layer(id).unwrap();
        assert_eq!(layer.transform.origin, CGPoint::ZERO);
        assert_eq!(
            layer.mask.as_ref().unwrap().placement.map(|p| p.origin),
            Some(CGPoint::new(10.0, 5.0))
        );
        assert_eq!(s.history.undo_name(), "Transform Layer Mask");
    }

    #[test]
    fn magic_wand_on_a_blank_layer_selects_the_canvas() {
        // selectionSample reads a blank layer as transparent; the wand then
        // matches the empty canvas (MagicWandTests.swift:101).
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset = None;
        s.magic_wand(CGPoint::new(50.0, 50.0), SelectionMode::Replace);
        let sel = s.selection().expect("blank layer still selects");
        let bounds = sel.path.bounds().expect("nonempty path");
        assert!(bounds.width() >= 99.0, "{bounds:?}");
        assert!(bounds.height() >= 99.0, "{bounds:?}");
    }

    fn rectangular_selection(s: &mut EditorSession, x0: f64, y0: f64, x1: f64, y1: f64) {
        s.tool = NavigationTool::Marquee;
        s.begin_lasso(CGPoint::new(x0, y0), SelectionMode::Replace);
        s.drag_marquee(CGPoint::new(x1, y1), false, false);
        s.finish_lasso();
    }

    #[test]
    fn fill_selection_fills_only_the_selection() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(100, 100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.fill_selection(false);
        let image = s
            .active_layer()
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        assert_eq!(image.pixel(20, 20)[3], 255, "inside fills");
        assert_eq!(image.pixel(20, 20)[0], 0, "default foreground is black");
        assert_eq!(image.pixel(60, 60)[3], 0, "outside stays untouched");
        assert_eq!(s.history.undo_name(), "Fill");
    }

    #[test]
    fn clear_selected_pixels_clears_only_the_selection() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.clear_selected_pixels();
        let image = s
            .active_layer()
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        assert_eq!(image.pixel(20, 20)[3], 0);
        assert_eq!(image.pixel(60, 60), [255, 0, 0, 255]);
        assert_eq!(s.history.undo_name(), "Clear");
    }

    // deleteKeyPressed (SelectionEdits.swift:63)

    #[test]
    fn delete_key_pressed_clears_a_selection() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        assert!(s.delete_key_pressed());
        let image = s
            .active_layer()
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        assert_eq!(image.pixel(20, 20)[3], 0);
        assert_eq!(s.history.undo_name(), "Clear");
    }

    #[test]
    fn delete_key_pressed_falls_through_without_a_selection_or_effect() {
        let mut s = session();
        assert!(!s.delete_key_pressed());
    }

    #[test]
    fn delete_key_pressed_removes_the_selected_effect_first() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        // Effects need a layer that carries pixels.
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(red_image(8), "L"));
        s.add_effect(LayerEffectKind::Shadow);
        s.finish_effects_editing(true);
        s.select_effect(LayerEffectKind::Shadow, id, false);
        assert!(s.delete_key_pressed());
        // The effect went in one "Remove" step; no layer deletion ran.
        // Removing a layer's only effect stores an empty set as `None`.
        assert!(
            !s.active_layer()
                .unwrap()
                .effects
                .unwrap_or_default()
                .contains(LayerEffectKind::Shadow)
        );
        assert_eq!(s.history.undo_name(), "Remove Drop Shadow");
        assert_eq!(s.selected_effect(), None);
    }

    #[test]
    fn pixel_move_moves_the_selected_pixels() {
        let mut s = session();
        // Red left third, green rest, so the landing is distinguishable from
        // what was already there.
        let mut image = Bitmap::new(100, 100);
        for y in 0..100 {
            for x in 0..100 {
                image.set_pixel(
                    x,
                    y,
                    if x < 30 {
                        [255, 0, 0, 255]
                    } else {
                        [0, 180, 0, 255]
                    },
                );
            }
        }
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(image, "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        assert!(s.begin_pixel_move(false), "the drag starts");
        s.drag_pixel_move(CGSize::new(25.5, 0.0));
        // Offsets round to whole pixels so edges stay crisp (movePixels).
        assert_eq!(
            s.pixel_move.as_ref().unwrap().offset,
            CGSize::new(26.0, 0.0)
        );
        s.end_pixel_move();
        let image = s
            .active_layer()
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        assert_eq!(image.pixel(20, 20)[3], 0, "the origin becomes a hole");
        assert_eq!(
            image.pixel(36, 20),
            [255, 0, 0, 255],
            "the pixels land 26 px over"
        );
        assert_eq!(
            image.pixel(60, 20),
            [0, 180, 0, 255],
            "the rest stays untouched"
        );
        // The outline moves with the pixels, as one undo step.
        let selection = s.selection().unwrap();
        assert_eq!(selection.path.bounds().unwrap().min_x(), 36.0);
        assert_eq!(s.history.undo_name(), "Move Pixels");
    }

    #[test]
    fn pixel_move_undo_restores_pixels_and_outline() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        let before = s.document.clone().unwrap();
        s.begin_pixel_move(false);
        s.drag_pixel_move(CGSize::new(25.0, -5.0));
        s.end_pixel_move();
        s.undo();
        let after = s.document.clone().unwrap();
        let image = after.layers[0].asset.as_ref().unwrap().image.as_ref();
        assert_eq!(image.pixel(20, 20), [255, 0, 0, 255], "pixels return");
        assert_eq!(after.selection, before.selection, "the outline returns");
    }

    #[test]
    fn pixel_move_without_a_selection_is_a_no_op() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        assert!(!s.begin_pixel_move(false));
        assert!(s.pixel_move.is_none());
        s.nudge_pixels(5.0, 0.0);
        assert_eq!(s.history.undo_name(), "New Canvas", "nothing was committed");
    }

    #[test]
    fn pixel_move_refuses_a_mask_target() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.add_mask_from_selection(true);
        assert!(s.is_mask_selected);
        assert!(
            !s.begin_pixel_move(false),
            "a mask target has no pixels to lift"
        );
        assert!(s.pixel_move.is_none());
    }

    #[test]
    fn add_mask_from_selection_reports_an_oversized_layer() {
        // Swift throws ProjectError.tooLarge and surfaces it via brushError
        // (LayerMask.swift:241, :259); the port mirrors the surfacing.
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].transform.size = CGSize::new(20_000.0, 20_000.0);
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.add_mask_from_selection(true);
        assert!(
            s.brush_error
                .as_deref()
                .is_some_and(|e| e.contains("100-megapixel")),
            "the tooLarge failure must be surfaced"
        );
        assert!(!s.is_mask_selected);
    }

    #[test]
    fn pixel_move_duplicate_keeps_the_source_pixels() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        assert!(s.begin_pixel_move(true));
        s.drag_pixel_move(CGSize::new(15.0, 0.0));
        s.end_pixel_move();
        let image = s
            .active_layer()
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        assert_eq!(image.pixel(20, 20), [255, 0, 0, 255], "the source stays");
        assert_eq!(
            image.pixel(40, 20),
            [255, 0, 0, 255],
            "the copy lands 15 px over"
        );
        assert_eq!(s.history.undo_name(), "Duplicate Pixels");
    }

    #[test]
    fn nudge_pixels_moves_one_step_as_one_undo() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.nudge_pixels(0.0, 3.0);
        let image = s
            .active_layer()
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        // Moving down refills all but the top 3 rows of the original outline.
        assert_eq!(image.pixel(20, 11)[3], 0, "the strip the pixels left");
        assert_eq!(
            image.pixel(20, 23),
            [255, 0, 0, 255],
            "the pixels land 3 px down"
        );
        let selection = s.selection().unwrap();
        assert_eq!(selection.path.bounds().unwrap().min_y(), 13.0);
        assert_eq!(s.history.undo_name(), "Move Pixels");
    }

    #[test]
    fn pixel_move_with_no_motion_commits_nothing() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        let undo_count = s.history.undo_count();
        s.begin_pixel_move(false);
        s.drag_pixel_move(CGSize::new(4.0, 4.0));
        s.drag_pixel_move(CGSize::new(0.0, 0.0));
        s.end_pixel_move();
        assert_eq!(
            s.history.undo_count(),
            undo_count,
            "a canceled-out drag is a no-op"
        );
        assert!(s.pixel_move.is_none());
        let image = s
            .active_layer()
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        assert_eq!(image.pixel(20, 20), [255, 0, 0, 255]);
    }

    #[test]
    fn displayed_selection_follows_the_pixel_drag() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        assert_eq!(
            s.displayed_selection()
                .unwrap()
                .path
                .bounds()
                .unwrap()
                .min_x(),
            10.0,
            "without a move the outline is the selection"
        );
        s.begin_pixel_move(false);
        s.drag_pixel_move(CGSize::new(7.0, 0.0));
        assert_eq!(
            s.displayed_selection()
                .unwrap()
                .path
                .bounds()
                .unwrap()
                .min_x(),
            17.0,
            "during the drag the outline leads the pixels"
        );
        // The stored selection stays put until the commit.
        assert_eq!(s.selection().unwrap().path.bounds().unwrap().min_x(), 10.0);
        s.end_pixel_move();
        assert_eq!(s.selection().unwrap().path.bounds().unwrap().min_x(), 17.0);
    }

    #[test]
    fn pixel_move_refused_while_a_transform_edit_is_open() {
        // canPaint needs canEditLayers; a pending transform closes editing.
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.begin_transform();
        assert!(!s.begin_pixel_move(false));
        s.cancel_transform();
        assert!(s.begin_pixel_move(false));
    }

    #[test]
    fn invert_pixels_inverts_inside_the_selection() {
        let mut s = session();
        let mut image = red_image(100);
        for y in 0..100 {
            for x in 0..100 {
                // Black left half, white right half.
                let v = if x < 50 { 0 } else { 255 };
                image.set_pixel(x, y, [v, v, v, 255]);
            }
        }
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(image, "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.invert_pixels();
        let image = s
            .active_layer()
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        assert!(
            image.pixel(20, 20)[0] > 200,
            "black inverts to white in place"
        );
        assert_eq!(image.pixel(75, 75)[0], 255, "unselected white stays");
        assert_eq!(image.pixel(5, 75)[0], 0, "unselected black stays");
        assert_eq!(s.history.undo_name(), "Invert");
    }

    #[test]
    fn mask_from_selection_knocks_out_the_selection() {
        // Add White Mask with a selection: white everywhere but the selected
        // pixels (LayerMask.addMask(revealing:) fills the selection with the
        // opposite gray).
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.add_mask_from_selection(true);
        assert!(s.selection().is_none(), "the selection becomes the mask");
        assert!(s.is_mask_selected);
        let mask = s.active_layer().unwrap().mask.as_ref().unwrap();
        let image = mask.asset.image.as_ref();
        assert_eq!((image.width, image.height), (100, 100));
        assert!(image.pixel(20, 20)[0] < 55, "selected: knocked out");
        assert!(image.pixel(60, 60)[0] > 200, "unselected: revealed");
        assert_eq!(s.history.undo_name(), "Add Mask from Selection");
    }

    #[test]
    fn add_adjustment_inserts_a_layer_above_the_active_one() {
        let mut s = session();
        s.add_adjustment(crate::adjustment::AdjustmentKind::Exposure);
        let document = s.document.as_ref().unwrap();
        assert_eq!(document.layers.len(), 2);
        let top = document.layers.last().unwrap();
        assert!(top.adjustment.is_some());
        assert!(top.asset.is_none());
        assert_eq!(s.active_layer_id, Some(top.id));
        assert_eq!(s.history.undo_name(), "New Exposure Adjustment");
    }

    #[test]
    fn duplicating_a_folder_copies_descendants_and_remaps_links() {
        // duplicateActiveLayer: a folder copies with its contents, the copy is
        // named " copy", and children land under the copied folder.
        let mut s = session();
        let folder_id = s.document.as_ref().unwrap().layers[0].id;
        {
            let doc = s.document.as_mut().unwrap();
            doc.layers[0].is_group = true;
            let mut child = ImageLayer::blank("Child", CGSize::new(100.0, 100.0));
            child.parent_id = Some(folder_id);
            doc.layers.push(child);
        }
        let child_id = s.document.as_ref().unwrap().layers[1].id;
        s.select_layer(Some(folder_id));
        s.duplicate_active_layer();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 4);
        // insert(contentsOf:at:) lands the copies as a block: folder,
        // folder copy, child copy, original child.
        let copy = &doc.layers[1];
        assert_eq!(copy.name, "Layer 1 copy");
        let copied_child = &doc.layers[2];
        assert_eq!(copied_child.name, "Child");
        assert_eq!(copied_child.parent_id, Some(copy.id));
        assert_eq!(doc.layers[3].id, child_id, "the original child stays");
        assert_eq!(s.history.undo_name(), "Duplicate Layer");
    }

    #[test]
    fn copy_paste_round_trips_pixels_in_place() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.copy_selection();
        let clipboard = s.pixel_clipboard.as_ref().unwrap();
        assert_eq!((clipboard.image.width, clipboard.image.height), (20, 20));
        assert_eq!(clipboard.origin, CGPoint::new(10.0, 10.0));
        // Pasting adds a layer above with the pixels where they came from.
        s.paste_pixels();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 2);
        let pasted = doc.layers.last().unwrap();
        assert_eq!(pasted.transform.origin, CGPoint::new(10.0, 10.0));
        assert_eq!(
            pasted.asset.as_ref().unwrap().image.pixel(10, 10),
            [255, 0, 0, 255]
        );
        assert!(s.selection().is_none(), "pasting drops the selection");
        assert_eq!(s.history.undo_name(), "Paste");
    }

    #[test]
    fn cut_selection_copies_then_clears() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.cut_selection();
        let image = s
            .active_layer()
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        assert_eq!(image.pixel(20, 20)[3], 0, "cut pixels leave the layer");
        assert_eq!(image.pixel(60, 60), [255, 0, 0, 255], "the rest stays");
        assert!(
            s.pixel_clipboard.is_some(),
            "cut pixels land on the clipboard"
        );
    }

    #[test]
    fn layer_via_copy_lifts_the_selection_into_a_layer() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        let active_before = s.active_layer_id;
        s.layer_via_copy();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 2);
        let copied = doc.layers.last().unwrap();
        assert_ne!(Some(copied.id), active_before);
        assert_eq!(copied.transform.origin, CGPoint::new(10.0, 10.0));
        assert_eq!(
            copied.asset.as_ref().unwrap().image.pixel(5, 5),
            [255, 0, 0, 255]
        );
        assert_eq!(s.history.undo_name(), "Layer via Copy");
    }

    #[test]
    fn floating_transform_lifts_pixels_onto_a_temporary_layer() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.begin_selection_transform();
        assert!(s.transform_edit.as_ref().unwrap().floating.is_some());
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 2, "the floating layer joins the stack");
        let floating = &doc.layers[1];
        assert_eq!(floating.name, "Floating Selection");
        assert_eq!(floating.transform.origin, CGPoint::new(10.0, 10.0));
        assert_eq!(
            floating.asset.as_ref().unwrap().image.pixel(5, 5),
            [255, 0, 0, 255]
        );
        assert_eq!(s.active_layer_id, Some(floating.id));
        // The source keeps the rest of its pixels but the selection is empty.
        let source = &doc.layers[0];
        assert_eq!(source.asset.as_ref().unwrap().image.pixel(20, 20)[3], 0);
        assert_eq!(
            source.asset.as_ref().unwrap().image.pixel(60, 60),
            [255, 0, 0, 255]
        );
    }

    #[test]
    fn committing_an_untouched_floating_transform_restores_the_document() {
        // No-op merge: restore exactly, so soft selection edges never pick up
        // a seam.
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.begin_selection_transform();
        s.commit_transform();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 1);
        let layer = &doc.layers[0];
        assert_eq!(
            layer.asset.as_ref().unwrap().image.pixel(20, 20),
            [255, 0, 0, 255]
        );
        assert_eq!(s.active_layer_id, Some(layer.id));
    }

    #[test]
    fn merging_a_moved_floating_selection_moves_pixels_and_selection() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.begin_selection_transform();
        if let Some(edit) = s.transform_edit.as_mut() {
            edit.draft.origin = CGPoint::new(20.0, 15.0);
        }
        s.commit_transform();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 1, "the floating layer merges away");
        let image = doc.layers[0].asset.as_ref().unwrap().image.as_ref().clone();
        assert_eq!(image.pixel(12, 12)[3], 0, "the vacated area stays clear");
        assert_eq!(
            image.pixel(25, 20),
            [255, 0, 0, 255],
            "the pixels land moved"
        );
        assert_eq!(image.pixel(60, 60), [255, 0, 0, 255], "the rest stays");
        let selection = doc.selection.as_ref().unwrap();
        let bounds = selection.path.bounds().unwrap();
        assert!((bounds.min_x() - 20.0).abs() <= 1.0, "{bounds:?}");
        assert!((bounds.min_y() - 15.0).abs() <= 1.0, "{bounds:?}");
        assert_eq!(s.active_layer_id, Some(doc.layers[0].id));
        // One undo step puts the whole thing back.
        s.undo();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(
            doc.layers[0].asset.as_ref().unwrap().image.pixel(20, 20),
            [255, 0, 0, 255]
        );
    }

    #[test]
    fn cancelling_a_floating_transform_restores_everything() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.begin_selection_transform();
        if let Some(edit) = s.transform_edit.as_mut() {
            edit.draft.origin = CGPoint::new(50.0, 50.0);
        }
        s.cancel_transform();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(
            doc.layers[0].asset.as_ref().unwrap().image.pixel(20, 20),
            [255, 0, 0, 255]
        );
        assert!(doc.selection.is_some(), "the selection comes back");
        assert_eq!(s.active_layer_id, Some(doc.layers[0].id));
    }

    fn blank_layer(s: &mut EditorSession, name: &str) -> Uuid {
        let mut layer = ImageLayer::blank(name, CGSize::new(100.0, 100.0));
        layer.id = uuid::Uuid::new_v4();
        let id = layer.id;
        s.document.as_mut().unwrap().layers.push(layer);
        id
    }

    #[test]
    fn group_selected_layers_wraps_the_selection() {
        let mut s = session();
        let first = s.document.as_ref().unwrap().layers[0].id;
        let second = blank_layer(&mut s, "Second");
        let third = blank_layer(&mut s, "Third");
        s.select_layers(&[first, second], Some(second));
        s.group_selected_layers();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 4);
        let folder = doc.layers.first().unwrap().clone();
        assert!(folder.is_group);
        assert_eq!(folder.name, "Folder 1");
        // The two selected layers are the folder's children.
        let children: Vec<&ImageLayer> = doc
            .layers
            .iter()
            .filter(|l| l.parent_id == Some(folder.id))
            .collect();
        assert_eq!(children.len(), 2);
        let ids: Vec<Uuid> = children.iter().map(|l| l.id).collect();
        assert!(ids.contains(&first) && ids.contains(&second));
        assert!(!ids.contains(&third));
        assert_eq!(s.active_layer_id, Some(folder.id));
        assert_eq!(s.history.undo_name(), "Group Layers");
        s.undo();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 3);
        assert!(doc.layers.iter().all(|l| l.parent_id.is_none()));
    }

    #[test]
    fn grouping_a_folder_carries_its_subtree() {
        // Selected descendants must not be pulled out of a selected folder:
        // the folder is a root, its child rides along.
        let mut s = session();
        let folder_id = s.document.as_ref().unwrap().layers[0].id;
        {
            let doc = s.document.as_mut().unwrap();
            doc.layers[0].is_group = true;
            let mut child = ImageLayer::blank("Child", CGSize::new(100.0, 100.0));
            child.parent_id = Some(folder_id);
            doc.layers.push(child);
        }
        let child_id = s.document.as_ref().unwrap().layers[1].id;
        let sibling = blank_layer(&mut s, "Sibling");
        s.select_layers(&[folder_id, child_id], Some(folder_id));
        s.group_selected_layers();
        let doc = s.document.as_ref().unwrap();
        // Only the folder lands in the new group; the child stays with it.
        let new_folder = doc.layers.first().unwrap();
        assert_eq!(new_folder.name, "Folder 1");
        let folder_child: Vec<&ImageLayer> = doc
            .layers
            .iter()
            .filter(|l| l.parent_id == Some(new_folder.id))
            .collect();
        assert_eq!(folder_child.len(), 1);
        assert_eq!(folder_child[0].id, folder_id);
        let _ = sibling;
    }

    #[test]
    fn collapse_hides_rows_but_layers_rows_show_folders() {
        let mut s = session();
        let folder_id = s.document.as_ref().unwrap().layers[0].id;
        {
            let doc = s.document.as_mut().unwrap();
            doc.layers[0].is_group = true;
            let mut child = ImageLayer::blank("Child", CGSize::new(100.0, 100.0));
            child.parent_id = Some(folder_id);
            doc.layers.push(child);
        }
        let rows = s.layer_rows();
        assert_eq!(rows.len(), 2, "folder then child");
        assert_eq!((rows[0].depth, rows[1].depth), (0, 1));
        s.toggle_group_expansion(folder_id);
        let rows = s.layer_rows();
        assert_eq!(rows.len(), 1, "the child row folds away");
        // Toggling back restores it.
        s.toggle_group_expansion(folder_id);
        assert_eq!(s.layer_rows().len(), 2);
    }

    #[test]
    fn place_layer_moves_into_groups_and_refuses_cycles() {
        let mut s = session();
        let layer_id = s.document.as_ref().unwrap().layers[0].id;
        s.add_group();
        let folder_id = s.active_layer_id.unwrap();
        // A folder can never be placed inside its own subtree.
        assert!(!s.can_place_layer(folder_id, Some(folder_id)));
        assert!(s.place_layer(layer_id, Some(folder_id), None, false));
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layer(layer_id).unwrap().parent_id, Some(folder_id));
        assert_eq!(s.history.undo_name(), "Move Layer");
        // Out again: above the group, at the group's parent (the root).
        s.select_layer(Some(layer_id));
        s.move_active_layer_out_of_group();
        assert_eq!(
            s.document
                .as_ref()
                .unwrap()
                .layer(layer_id)
                .unwrap()
                .parent_id,
            None
        );
    }

    #[test]
    fn reorder_layers_moves_in_top_first_panel_order() {
        // Document order is bottom-to-top: [A, B]. The panel shows [B, A].
        let mut s = session();
        let a = s.document.as_ref().unwrap().layers[0].id;
        let b = blank_layer(&mut s, "B");
        // Panel order: [B, A]. Moving panel row 0 (B) to the panel end drops B
        // to the bottom of the canvas — the first slot in storage order.
        s.reorder_layers(&[0], 2);
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.first().unwrap().id, b, "B lands at the bottom");
        assert_eq!(doc.layers.last().unwrap().id, a, "A rises to the top");
    }

    #[test]
    fn visibility_swipe_batches_into_one_undo_step() {
        let mut s = session();
        let first = s.document.as_ref().unwrap().layers[0].id;
        let second = blank_layer(&mut s, "B");
        let before = s.history.undo_count();
        s.begin_visibility_swipe(first).unwrap();
        s.set_visibility_in_swipe(second, false);
        s.end_visibility_swipe();
        let doc = s.document.as_ref().unwrap();
        assert!(!doc.layer(first).unwrap().is_visible);
        assert!(!doc.layer(second).unwrap().is_visible);
        assert_eq!(s.history.undo_count(), before + 1, "one step for the swipe");
        s.undo();
        let doc = s.document.as_ref().unwrap();
        assert!(doc.layer(first).unwrap().is_visible);
        assert!(doc.layer(second).unwrap().is_visible);
    }

    #[test]
    fn delete_selected_layers_takes_every_selection_in_one_step() {
        let mut s = session();
        let first = s.document.as_ref().unwrap().layers[0].id;
        let second = blank_layer(&mut s, "B");
        let third = blank_layer(&mut s, "C");
        s.select_layers(&[first, second], Some(first));
        s.delete_selected_layers();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].id, third);
        assert_eq!(s.active_layer_id, Some(third));
        assert_eq!(s.history.undo_name(), "Delete Layers");
    }

    #[test]
    fn importing_images_creates_a_canvas_then_centers_layers() {
        let mut s = EditorSession::new();
        let dir = std::env::temp_dir();
        let first = dir.join(format!("compositor-import-{}-a.png", std::process::id()));
        let image = image::RgbaImage::from_pixel(20, 10, image::Rgba([200, 40, 40, 255]));
        image
            .save_with_format(&first, image::ImageFormat::Png)
            .unwrap();
        let failures = s.import_images(std::slice::from_ref(&first), None);
        assert!(failures.is_empty(), "{failures:?}");
        let doc = s
            .document
            .as_ref()
            .expect("the first image makes the canvas");
        assert_eq!((doc.width, doc.height), (20, 10));
        assert_eq!(doc.layers.len(), 1);
        // A second image centers on the canvas.
        let second = dir.join(format!("compositor-import-{}-b.png", std::process::id()));
        let image = image::RgbaImage::from_pixel(10, 10, image::Rgba([40, 40, 200, 255]));
        image
            .save_with_format(&second, image::ImageFormat::Png)
            .unwrap();
        let failures = s.import_images(std::slice::from_ref(&second), None);
        assert!(failures.is_empty(), "{failures:?}");
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 2);
        let pasted = doc.layers.last().unwrap();
        assert_eq!(pasted.transform.origin, CGPoint::new(5.0, 0.0));
        assert_eq!(s.history.undo_name(), "Import Images");
        // Unreadable paths report failures.
        let failures = s.import_images(&[dir.join("compositor-missing-file.png")], None);
        assert_eq!(failures.len(), 1);
        let _ = std::fs::remove_file(first);
        let _ = std::fs::remove_file(second);
    }

    #[test]
    fn import_size_budget_rejects_oversized_bitmaps() {
        let mut s = session();
        // Over the per-side cap.
        assert!(!s.import_size_allowed(30_001, 1));
        // Over the 100-megapixel document budget once content exists.
        assert!(
            s.import_size_allowed(5_000, 5_000),
            "25 MP fits an empty canvas"
        );
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(9_500), "L")); // 90.25 MP
        assert!(
            !s.import_size_allowed(5_000, 5_000),
            "90.25 + 25 exceeds 100 MP"
        );
    }

    #[test]
    fn applying_text_creates_a_live_text_layer() {
        let mut s = session();
        let point = CGPoint::new(20.0, 15.0);
        s.begin_text(point, true);
        let mut draft = s.text_draft.clone().unwrap();
        draft.style.content = "Hello".to_string();
        assert!(s.apply_text(draft));
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 2);
        let layer = doc.layers.last().unwrap();
        assert_eq!(layer.name, "Hello");
        // The layer is live text: its pixels carry the rasterized glyphs.
        let live = layer.live_text().expect("live text");
        assert_eq!(live.style.content, "Hello");
        assert!(live.image.width >= 16);
        assert!(
            live.image.data.chunks_exact(4).any(|px| px[3] > 0),
            "glyphs are rasterized"
        );
        assert_eq!(s.active_layer_id, Some(layer.id));
        assert_eq!(s.history.undo_name(), "New Text Layer");
        // The style becomes the default for the next text layer.
        assert_eq!(s.text_defaults.content, "Hello");
    }

    #[test]
    fn applying_empty_new_text_is_a_quiet_noop() {
        let mut s = session();
        let before = s.document.as_ref().unwrap().layers.len();
        s.begin_text(CGPoint::new(20.0, 15.0), true);
        let draft = s.text_draft.clone().unwrap();
        assert!(draft.style.content.is_empty());
        assert!(s.apply_text(draft));
        assert_eq!(s.document.as_ref().unwrap().layers.len(), before);
        assert!(s.text_draft.is_none());
    }

    #[test]
    fn editing_updates_the_live_text_layer() {
        let mut s = session();
        let point = CGPoint::new(20.0, 15.0);
        s.begin_text(point, true);
        let mut draft = s.text_draft.clone().unwrap();
        draft.style.content = "First".to_string();
        s.apply_text(draft);
        let layer_id = s.document.as_ref().unwrap().layers.last().unwrap().id;
        // A click inside the layer edits it rather than making a new one.
        s.begin_text(CGPoint::new(25.0, 20.0), false);
        let draft = s.text_draft.clone().expect("draft targets the layer");
        assert_eq!(draft.layer_id, Some(layer_id));
        let mut edited = draft;
        edited.style.content = "Second".to_string();
        assert!(s.apply_text(edited));
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 2, "no extra layer");
        let layer = doc.layer(layer_id).unwrap();
        assert_eq!(layer.live_text().unwrap().style.content, "Second");
        assert_eq!(s.history.undo_name(), "Edit Text");
    }

    #[test]
    fn recolor_text_keeps_it_editable() {
        let mut s = session();
        let point = CGPoint::new(20.0, 15.0);
        s.begin_text(point, true);
        let mut draft = s.text_draft.clone().unwrap();
        draft.style.content = "Tint".to_string();
        s.apply_text(draft);
        let layer_id = s.document.as_ref().unwrap().layers.last().unwrap().id;
        assert!(s.recolor_text(layer_id, PaletteColor::new(1.0, 0.0, 0.0)));
        let layer = s.document.as_ref().unwrap().layer(layer_id).unwrap();
        let live = layer.live_text().expect("still live text");
        assert_eq!(
            (live.style.red, live.style.green, live.style.blue),
            (1.0, 0.0, 0.0)
        );
        assert_eq!(s.history.undo_name(), "Fill Text");
    }

    #[test]
    fn moving_snaps_to_the_canvas_edge() {
        // A layer at (50,50,10,10): dragging it so its right/bottom edge comes
        // within tolerance of the canvas's right/bottom edge snaps it to
        // x = 90, y = 90.
        let mut s = session();
        let id = s.document.as_ref().unwrap().layers[0].id;
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(red_image(10), "L"));
        s.document.as_mut().unwrap().layers[0].transform =
            LayerTransform::new(CGPoint::new(50.0, 50.0), CGSize::new(10.0, 10.0));
        s.select_layer(Some(id));
        s.begin_transform();
        let mut draft = s.transform_edit.as_ref().unwrap().draft;
        draft.origin = CGPoint::new(51.0, 51.0);
        let snapped = s.snapped_move(draft, &[id], 10.0);
        // The draft's right edge lands at 101, one pixel shy of the canvas —
        // within the 10 px tolerance.
        assert!((snapped.origin.x - 50.0).abs() <= 11.0, "{snapped:?}");
        assert!((snapped.origin.y - 50.0).abs() <= 11.0, "{snapped:?}");
        let _ = id;
        // An exact positioning: max at 101 snaps to 100 → origin 90.
        draft.origin = CGPoint::new(91.0, 91.0);
        let snapped = s.snapped_move(draft, &[id], 10.0);
        assert!((snapped.origin.x - 90.0).abs() <= 10.0, "{snapped:?}");
    }

    #[test]
    fn snapping_off_disables_move_snapping() {
        let mut s = session();
        s.snapping_enabled = false;
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(red_image(10), "L"));
        s.document.as_mut().unwrap().layers[0].transform =
            LayerTransform::new(CGPoint::new(50.0, 50.0), CGSize::new(10.0, 10.0));
        s.select_layer(Some(s.document.as_ref().unwrap().layers[0].id));
        s.begin_transform();
        let mut draft = s.transform_edit.as_ref().unwrap().draft;
        draft.origin = CGPoint::new(91.0, 91.0);
        let moved = s.snapped_move(draft, &[], 10.0);
        assert!((moved.origin.x - 91.0).abs() < 1e-9, "no snap: {moved:?}");
        assert_eq!(s.snap_guides, (Vec::new(), Vec::new()));
    }

    #[test]
    fn change_text_style_edits_the_pending_draft() {
        let mut s = session();
        s.begin_text(CGPoint::new(20.0, 15.0), true);
        s.change_text_style(|style| style.font_size = 100.0);
        assert_eq!(s.text_draft.as_ref().unwrap().style.font_size, 100.0);
        assert_eq!(s.current_text_style().font_size, 100.0);
        // With no draft and no live text, edits land in the defaults.
        let mut s = session();
        s.change_text_style(|style| style.font_size = 36.0);
        assert_eq!(s.text_defaults.font_size, 36.0);
    }

    #[test]
    fn flipping_several_layers_uses_their_shared_box() {
        let mut s = session();
        let first = s.document.as_ref().unwrap().layers[0].id;
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(red_image(10), "A"));
        s.document.as_mut().unwrap().layers[0].transform =
            LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(10.0, 10.0));
        let second = blank_layer(&mut s, "B");
        s.document.as_mut().unwrap().layers[1].asset = Some(ImportedImage::new(red_image(10), "B"));
        s.document.as_mut().unwrap().layers[1].transform =
            LayerTransform::new(CGPoint::new(50.0, 0.0), CGSize::new(10.0, 10.0));
        s.select_layers(&[first, second], Some(first));
        s.flip_layers(true);
        // The shared box spans 0..60; flipping about its middle swaps the
        // two sides.
        let first_layer = s.document.as_ref().unwrap().layer(first).unwrap();
        assert!(
            (first_layer.transform.center().x - 55.0).abs() < 1.0,
            "{:?}",
            first_layer.transform
        );
        assert_eq!(s.history.undo_name(), "Flip Horizontal");
    }

    #[test]
    fn flipping_the_canvas_mirrors_the_selection() {
        let mut s = session();
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.flip_canvas(true);
        let selection = s.document.as_ref().unwrap().selection.as_ref().unwrap();
        let bounds = selection.path.bounds().unwrap();
        // The 10..30 range mirrors to 70..90 across the 100-wide canvas.
        assert!((bounds.min_x() - 70.0).abs() <= 1.0, "{bounds:?}");
    }

    #[test]
    fn snapped_move_records_the_guide_it_landed_on() {
        // Dragging a layer so its center hits the canvas middle records the
        // vertical center guide for the canvas to draw.
        let mut s = session();
        let id = s.document.as_ref().unwrap().layers[0].id;
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(red_image(10), "L"));
        s.document.as_mut().unwrap().layers[0].transform =
            LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(10.0, 10.0));
        s.select_layer(Some(id));
        s.begin_transform();
        let mut draft = s.transform_edit.as_ref().unwrap().draft;
        draft.origin = CGPoint::new(45.0, 0.0);
        let moved = s.snapped_move(draft, &[id], 10.0);
        assert!((moved.origin.x - 45.0).abs() < 1e-9, "{moved:?}");
        assert_eq!(
            s.snap_guides.0,
            vec![50.0],
            "vertical center guide recorded"
        );
        // Committing clears the guides.
        s.commit_transform();
        assert_eq!(s.snap_guides, (Vec::new(), Vec::new()));
    }

    #[test]
    fn nudging_the_selection_moves_whole_pixels() {
        let mut s = session();
        rectangular_selection(&mut s, 10.4, 10.0, 30.0, 30.0);
        s.nudge_selection(0.6, 0.0);
        let bounds = s
            .document
            .as_ref()
            .unwrap()
            .selection
            .as_ref()
            .unwrap()
            .path
            .bounds()
            .unwrap();
        // 0.6 + 0.6 rounds to whole pixels: the outline lands on an integer.
        assert!(
            (bounds.min_x() - bounds.min_x().round()).abs() < 1e-6,
            "{bounds:?}"
        );
    }

    #[test]
    fn clone_aligned_strokes_reuse_the_first_offset() {
        let mut s = session();
        s.clone_source = Some(CGPoint::new(30.0, 40.0));
        s.clone_settings.aligned = true;
        // The first stroke fixes the offset.
        let first = s.begin_clone_stroke(CGPoint::new(50.0, 60.0)).unwrap();
        assert_eq!(first, CGSize::new(-20.0, -20.0));
        assert_eq!(s.clone_offset, Some(first));
        // A later stroke elsewhere reuses it.
        let second = s.begin_clone_stroke(CGPoint::new(10.0, 10.0)).unwrap();
        assert_eq!(second, first, "aligned strokes share the offset");
        // Unaligned strokes recompute from the brush to the source.
        s.clone_settings.aligned = false;
        let unaligned = s.clone_stroke_offset(CGPoint::new(10.0, 10.0)).unwrap();
        assert_eq!(unaligned, CGSize::new(20.0, 30.0));
    }

    #[test]
    fn clone_without_a_source_has_no_offset() {
        let mut s = session();
        s.clone_settings.aligned = true;
        assert!(s.clone_stroke_offset(CGPoint::new(10.0, 10.0)).is_none());
        assert!(s.begin_clone_stroke(CGPoint::new(10.0, 10.0)).is_none());
    }

    #[test]
    fn gradient_drag_applies_to_the_layer() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        s.tool = NavigationTool::Gradient;
        s.begin_gradient(CGPoint::new(10.0, 50.0));
        s.move_gradient(
            Some(CGPoint::new(10.0, 50.0)),
            Some(CGPoint::new(90.0, 50.0)),
        );
        // The preview rebuilds from the endpoints.
        assert!(s.refresh_gradient().is_some());
        assert!(s.apply_gradient());
        let image = s.document.as_ref().unwrap().layers[0]
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        // Black foreground to transparent: the start edge is inky black,
        // the end edge fades to the clear red underneath.
        let start = image.pixel(12, 50);
        let end = image.pixel(88, 50);
        // Black foreground fades out over the red layer: the start is inky,
        // the end lets the red show through.
        assert!(start[0] < 60 && start[3] == 255, "{start:?}");
        assert!(end[0] > start[0] + 150, "{start:?} {end:?}");
        assert_eq!(s.history.undo_name(), "Gradient");
        s.undo();
        let image = s.document.as_ref().unwrap().layers[0]
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        assert_eq!(
            image.pixel(12, 50),
            [255, 0, 0, 255],
            "undo removes the gradient"
        );
    }

    #[test]
    fn gradient_commits_to_the_layer_its_draft_captured() {
        // Gradient.swift:88-100 commits `edit.raster`, bound to the layer from
        // begin time — switching the active layer afterwards must not move
        // the target.
        let mut s = session();
        s.tool = NavigationTool::Gradient;
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "A"));
        let a = s.active_layer_id.unwrap();
        s.begin_gradient(CGPoint::new(10.0, 50.0));
        s.move_gradient(
            Some(CGPoint::new(10.0, 50.0)),
            Some(CGPoint::new(90.0, 50.0)),
        );
        // The active layer changes after the draft was opened.
        s.add_blank_layer();
        assert_ne!(s.active_layer_id, Some(a));
        assert!(s.apply_gradient());
        let a_image = s
            .document
            .as_ref()
            .unwrap()
            .layer(a)
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        assert!(a_image.pixel(12, 50)[0] < 60, "gradient landed on layer A");
        // And a deleted draft target commits nothing anywhere.
        let mut s = session();
        s.tool = NavigationTool::Gradient;
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "A"));
        let a = s.active_layer_id.unwrap();
        s.begin_gradient(CGPoint::new(10.0, 50.0));
        s.move_gradient(
            Some(CGPoint::new(10.0, 50.0)),
            Some(CGPoint::new(90.0, 50.0)),
        );
        s.add_blank_layer();
        s.delete_layer(a);
        let before = s
            .active_layer()
            .unwrap()
            .asset
            .as_ref()
            .map(|a| a.image.as_ref().clone());
        assert!(!s.apply_gradient(), "a deleted target rejects the commit");
        let after = s
            .active_layer()
            .unwrap()
            .asset
            .as_ref()
            .map(|a| a.image.as_ref().clone());
        assert_eq!(before.map(|b| b.data), after.map(|b| b.data));
    }

    #[test]
    fn gradient_on_a_mask_uses_gray() {
        let mut s = session();
        let id = s.document.as_ref().unwrap().layers[0].id;
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        s.add_layer_mask(true);
        s.is_mask_selected = true;
        s.tool = NavigationTool::Gradient;
        s.begin_gradient(CGPoint::new(10.0, 50.0));
        s.move_gradient(
            Some(CGPoint::new(10.0, 50.0)),
            Some(CGPoint::new(90.0, 50.0)),
        );
        assert!(s.apply_gradient());
        let mask = s
            .document
            .as_ref()
            .unwrap()
            .layer(id)
            .unwrap()
            .mask
            .as_ref()
            .unwrap();
        // Black foreground on a white mask: the gray ramps up left to right.
        let left = mask.asset.image.pixel(12, 50)[0];
        let right = mask.asset.image.pixel(88, 50)[0];
        assert!(left < right, "left={left} right={right}");
        assert_eq!(s.history.undo_name(), "Gradient Mask");
    }

    #[test]
    fn switching_layers_applies_the_pending_gradient() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        s.tool = NavigationTool::Gradient;
        s.begin_gradient(CGPoint::new(10.0, 50.0));
        s.move_gradient(
            Some(CGPoint::new(10.0, 50.0)),
            Some(CGPoint::new(90.0, 50.0)),
        );
        let second = blank_layer(&mut s, "B");
        s.select_layer(Some(second));
        assert!(
            s.gradient_edit.is_none(),
            "switching layers applies the pending gradient"
        );
        let image = s.document.as_ref().unwrap().layers[0]
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        assert!(image.pixel(12, 50)[0] < 150, "the gradient committed");
    }

    #[test]
    fn guide_interaction_create_move_delete() {
        let mut s = session();
        s.begin_guide_creation(GuideAxis::Vertical, 50.0);
        assert!(s.guide_drag.as_ref().unwrap().is_new);
        s.finish_guide_drag(false);
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.guides.len(), 1, "new guide lands");
        assert_eq!(doc.guides[0].position, 50.0);
        assert_eq!(s.history.undo_name(), "New Guide");

        // Move it.
        let guide_id = doc.guides[0].id;
        s.begin_guide_move(guide_id);
        s.move_guide_drag(70.0);
        s.finish_guide_drag(false);
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.guides[0].position, 70.0);
        assert_eq!(s.history.undo_name(), "Move Guide");

        // Releasing on a ruler deletes it.
        s.begin_guide_move(guide_id);
        s.finish_guide_drag(true);
        assert_eq!(s.document.as_ref().unwrap().guides.len(), 0);
        assert_eq!(s.history.undo_name(), "Delete Guide");
    }

    #[test]
    fn guide_creation_snaps_to_layer_edges() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(red_image(10), "L"));
        s.document.as_mut().unwrap().layers[0].transform =
            LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(10.0, 10.0));
        // 8 is within the 10px tolerance of the layer's right edge at 10.
        s.begin_guide_creation(GuideAxis::Vertical, 8.0);
        assert_eq!(s.guide_drag.as_ref().unwrap().position, 10.0);
    }

    #[test]
    fn hit_guide_finds_the_nearest_within_tolerance() {
        let mut s = session();
        s.shows_guides = true;
        s.add_guide(GuideAxis::Vertical, 20.0);
        s.add_guide(GuideAxis::Vertical, 60.0);
        assert_eq!(
            s.hit_guide(CGPoint::new(22.0, 10.0), 10.0),
            Some(s.document.as_ref().unwrap().guides[0].id)
        );
        assert_eq!(
            s.hit_guide(CGPoint::new(59.0, 10.0), 10.0),
            Some(s.document.as_ref().unwrap().guides[1].id)
        );
        assert_eq!(
            s.hit_guide(CGPoint::new(40.0, 10.0), 10.0),
            None,
            "no guide within tolerance"
        );
        assert_eq!(
            s.hit_guide(CGPoint::new(59.0, 10.0), 10.0),
            s.hit_guide(CGPoint::new(59.0, 10.0), 10.0)
        );
    }

    #[test]
    fn locked_guides_refuse_creation_moves_and_hits() {
        // Guides.swift:74/92 — locked guides block hitGuide and both guide drags.
        let mut s = session();
        s.shows_guides = true;
        s.add_guide(GuideAxis::Vertical, 20.0);
        let guide_id = s.document.as_ref().unwrap().guides[0].id;
        s.locks_guides = true;
        assert_eq!(s.hit_guide(CGPoint::new(20.0, 10.0), 10.0), None);
        s.begin_guide_creation(GuideAxis::Vertical, 35.0);
        assert!(s.guide_drag.is_none(), "creation refused while locked");
        s.begin_guide_move(guide_id);
        assert!(s.guide_drag.is_none(), "move refused while locked");
        // Unlocking restores everything.
        s.locks_guides = false;
        assert_eq!(s.hit_guide(CGPoint::new(20.0, 10.0), 10.0), Some(guide_id));
        s.begin_guide_move(guide_id);
        assert!(s.guide_drag.is_some());
    }

    #[test]
    fn displayed_transform_prefers_the_pending_draft() {
        let mut s = session();
        let id = s.document.as_ref().unwrap().layers[0].id;
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(red_image(10), "L"));
        let layer = s.document.as_ref().unwrap().layer(id).unwrap().clone();
        assert_eq!(s.displayed_transform(&layer), layer.transform);
        s.begin_transform();
        if let Some(edit) = s.transform_edit.as_mut() {
            edit.draft.origin = CGPoint::new(5.0, 5.0);
        }
        let layer = s.document.as_ref().unwrap().layer(id).unwrap().clone();
        assert_eq!(
            s.displayed_transform(&layer).origin,
            CGPoint::new(5.0, 5.0),
            "the draft wins while the transform is pending"
        );
    }

    #[test]
    fn shape_layers_rerender_on_transform_commit() {
        // redrawShape: resizing a shape layer redraws the raster at the new
        // bounds instead of stretching the old pixels.
        let mut s = session();
        let id = s.document.as_ref().unwrap().layers[0].id;
        {
            let doc = s.document.as_mut().unwrap();
            let shape = crate::document::LayerShape {
                kind: crate::document::ShapeKind::Rectangle,
                corner_radius: 0.0,
                line_width: 0.0,
                red: 1.0,
                green: 0.0,
                blue: 0.0,
                filled: true,
                start: CGPoint::ZERO,
                end: CGPoint::new(1.0, 1.0),
            };
            let raster = crate::render::render_shape_image(&shape, 10, 10);
            let mut layer =
                ImageLayer::with_asset(ImportedImage::new(raster, "Shape"), CGPoint::ZERO);
            layer.id = id;
            layer.shape = Some(shape);
            doc.layers[0] = layer;
        }
        s.begin_transform();
        if let Some(edit) = s.transform_edit.as_mut() {
            edit.draft.size = CGSize::new(30.0, 30.0);
        }
        s.commit_transform();
        let layer = s.document.as_ref().unwrap().layer(id).unwrap();
        assert_eq!(layer.transform.size, CGSize::new(30.0, 30.0));
        let asset = layer.asset.as_ref().unwrap();
        assert_eq!(
            asset.image.width, 30,
            "the raster was redrawn, not stretched"
        );
        assert_eq!(asset.image.pixel(2, 2), [255, 0, 0, 255]);
    }

    #[test]
    fn toggle_shape_kind_cycles_the_draw_kind() {
        let mut s = session();
        s.shape_kind = crate::document::ShapeKind::Rectangle;
        s.toggle_shape_kind();
        assert_eq!(s.shape_kind, crate::document::ShapeKind::Ellipse);
        s.toggle_shape_kind();
        assert_eq!(s.shape_kind, crate::document::ShapeKind::RoundedRectangle);
        s.toggle_shape_kind();
        assert_eq!(s.shape_kind, crate::document::ShapeKind::Rectangle);
    }

    #[test]
    fn pending_gradient_counts_as_undoable() {
        // A pending gradient can still be applied or cancelled, so the menu
        // treats it as undoable (EditorSession.swift:567).
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        s.tool = NavigationTool::Gradient;
        s.begin_gradient(CGPoint::new(10.0, 50.0));
        s.move_gradient(
            Some(CGPoint::new(10.0, 50.0)),
            Some(CGPoint::new(90.0, 50.0)),
        );
        assert!(s.gradient_edit.is_some());
        assert!(s.can_undo(), "pending gradient is undoable");
        assert!(s.apply_gradient());
        assert!(s.can_undo(), "applied gradient remains undoable");
    }

    #[test]
    fn blank_layer_lands_at_the_top_of_a_selected_folder() {
        // EditorSession.swift:614-632: with a folder selected the layer goes
        // just above the folder's topmost contents, not below everything.
        let mut s = session();
        let folder_id = s.document.as_ref().unwrap().layers[0].id;
        {
            let doc = s.document.as_mut().unwrap();
            doc.layers[0].is_group = true;
            let mut inner = ImageLayer::blank("Inner", CGSize::new(100.0, 100.0));
            inner.parent_id = Some(folder_id);
            doc.layers.push(inner);
        }
        s.select_layer(Some(folder_id));
        s.add_blank_layer();
        let doc = s.document.as_ref().unwrap();
        // Storage is bottom-to-top: the new layer lands above the folder's
        // topmost contents (Swift: insertion = lastIndex(where: inside) + 1).
        assert_eq!(doc.layers.len(), 3);
        assert_eq!(doc.layers[0].id, folder_id);
        assert_eq!(doc.layers[1].name, "Inner");
        assert_eq!(doc.layers[2].name, "Layer 2");
        assert_eq!(doc.layers[2].parent_id, Some(folder_id));
    }

    #[test]
    fn gaussian_blur_keeps_far_regions_untouched() {
        // The blur works on the visible pixels' bounds: content far from the
        // layer's bitmap edge still blurs, and empty far regions stay empty.
        let mut img = Bitmap::new(100, 100);
        for y in 40..60 {
            for x in 40..60 {
                img.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        let out = crate::filters::run_filter(
            &crate::filters::FilterJob::builder(
                crate::filters::FilterKind::GaussianBlur,
                &img,
                crate::filters::FilterSettings {
                    radius: 6.0,
                    ..Default::default()
                },
            )
            .build(),
        );
        // Unclamped (CI semantics): the 20px block blurs into transparency,
        // so the core thins slightly but stays strongly opaque.
        let core = out.pixel(50, 50);
        assert!(core[3] > 180, "core stays mostly opaque: {core:?}");
        assert!(core[0] > 180, "{core:?}");
        // The blur spreads into the transparent surroundings by a few pixels.
        assert!(
            out.pixel(36, 50)[3] > 0,
            "edge softens outward: {:?}",
            out.pixel(36, 50)
        );
        assert_eq!(out.pixel(5, 5)[3], 0, "far regions stay empty");
    }

    #[test]
    fn lasso_ignores_non_finite_points() {
        let mut s = session();
        s.tool = NavigationTool::Lasso;
        s.begin_lasso(CGPoint::new(10.0, 10.0), SelectionMode::Replace);
        s.extend_lasso(CGPoint::new(f64::NAN, 20.0));
        let draft = s.lasso_draft.as_ref().unwrap();
        assert_eq!(draft.points.len(), 1, "the NaN point is refused");
        assert!(!s.can_modify_selection(), "an open lasso blocks edits");
    }

    #[test]
    fn moving_the_crop_frame_snaps_both_edges() {
        // The frame's left edge comes within tolerance of the canvas's left
        // edge (x = 0): the move snaps onto it, keeping the size.
        let mut s = session();
        s.crop_rect = Some(CGRect::new(6.0, 20.0, 50.0, 40.0));
        s.begin_crop_drag(CGPoint::new(8.0, 20.0), crate::canvas::CropDragMode::Move);
        s.drag_crop(CGPoint::new(-2.0, 20.0), None, false);
        let rect = s.crop_rect.expect("crop active");
        assert_eq!(rect.min_x(), 0.0, "left edge snapped to the canvas");
        assert_eq!(rect.width(), 50.0, "the size is unchanged by a move");
    }

    #[test]
    fn resizing_snaps_only_the_dragged_edge() {
        let mut s = session();
        s.crop_rect = Some(CGRect::new(20.0, 20.0, 50.0, 40.0));
        // Drag the right-middle handle: the right edge snaps to the canvas's
        // right edge, the left edge stays put.
        s.begin_crop_drag(
            CGPoint::new(70.0, 40.0),
            crate::canvas::CropDragMode::Resize(2),
        );
        s.drag_crop(CGPoint::new(101.0, 40.0), None, false);
        let rect = s.crop_rect.expect("crop active");
        assert!(
            (rect.max_x() - 100.0).abs() <= 1.0,
            "right edge snapped: {rect:?}"
        );
        assert!(
            (rect.min_x() - 20.0).abs() <= 1.0,
            "left edge untouched: {rect:?}"
        );
    }

    #[test]
    fn a_fixed_ratio_skips_edge_snapping() {
        let mut s = session();
        s.crop_rect = Some(CGRect::new(4.0, 20.0, 50.0, 40.0));
        s.begin_crop_drag(CGPoint::new(2.0, 20.0), crate::canvas::CropDragMode::Move);
        s.drag_crop(CGPoint::new(-2.0, 20.0), Some(1.5), false);
        let rect = s.crop_rect.expect("crop active");
        assert!(
            (rect.min_x() - 2.0).abs() < 3.0,
            "no snap with a ratio: {rect:?}"
        );
    }

    #[test]
    fn snapping_off_leaves_the_frame_alone() {
        let mut s = session();
        s.snapping_enabled = false;
        s.crop_rect = Some(CGRect::new(4.0, 20.0, 50.0, 40.0));
        s.begin_crop_drag(CGPoint::new(2.0, 20.0), crate::canvas::CropDragMode::Move);
        s.drag_crop(CGPoint::new(-2.0, 20.0), None, false);
        let rect = s.crop_rect.expect("crop active");
        assert!((rect.min_x() - 2.0).abs() < 3.0, "free placement: {rect:?}");
    }

    #[test]
    fn toggle_clipping_mask_clips_to_the_lower_sibling() {
        let mut s = session();
        let first = s.document.as_ref().unwrap().layers[0].id;
        let second = blank_layer(&mut s, "B");
        let third = blank_layer(&mut s, "C");
        // The bottom layer has nothing above it to clip to — and cannot clip.
        assert!(!s.can_toggle_clipping_mask(first));
        // Option-click the middle layer: clipped to the bottom one.
        s.toggle_clipping_mask(second);
        assert_eq!(
            s.document
                .as_ref()
                .unwrap()
                .layer(second)
                .unwrap()
                .mask_source_id,
            Some(first)
        );
        assert_eq!(s.history.undo_name(), "Create Clipping Mask");
        // Option-click the top layer: it shares the bottom one as its base.
        s.toggle_clipping_mask(third);
        assert_eq!(
            s.document
                .as_ref()
                .unwrap()
                .layer(third)
                .unwrap()
                .mask_source_id,
            Some(first)
        );
        // Releasing the first clipped child releases the whole stack above it.
        s.remove_live_mask(second);
        assert_eq!(
            s.document
                .as_ref()
                .unwrap()
                .layer(second)
                .unwrap()
                .mask_source_id,
            None
        );
        assert_eq!(
            s.document
                .as_ref()
                .unwrap()
                .layer(third)
                .unwrap()
                .mask_source_id,
            None
        );
        assert_eq!(s.history.undo_name(), "Release Clipping Mask");
    }

    #[test]
    fn releasing_a_child_also_releases_the_clipped_sibling_above_it() {
        // Releasing a child of a shared base releases the contiguous stack
        // above it sharing that base; lower siblings stay put.
        let mut s = session();
        let first = s.document.as_ref().unwrap().layers[0].id;
        let second = blank_layer(&mut s, "B");
        let third = blank_layer(&mut s, "C");
        s.toggle_clipping_mask(second);
        s.toggle_clipping_mask(third);
        s.toggle_clipping_mask(second);
        assert_eq!(
            s.document
                .as_ref()
                .unwrap()
                .layer(second)
                .unwrap()
                .mask_source_id,
            None
        );
        assert_eq!(
            s.document
                .as_ref()
                .unwrap()
                .layer(third)
                .unwrap()
                .mask_source_id,
            None
        );
        assert_eq!(s.history.undo_name(), "Release Clipping Mask");
        let _ = first;
    }

    #[test]
    fn link_mask_refuses_cycles() {
        let mut s = session();
        let first = s.document.as_ref().unwrap().layers[0].id;
        let second = blank_layer(&mut s, "B");
        s.toggle_clipping_mask(second);
        // Linking the base to its own clipped child would close a cycle.
        assert!(!s.can_link_mask(second, first));
        assert!(!s.link_mask(second, first));
        assert_eq!(
            s.document
                .as_ref()
                .unwrap()
                .layer(first)
                .unwrap()
                .mask_source_id,
            None
        );
    }

    #[test]
    fn baking_carries_the_clipped_appearance_into_pixels() {
        let mut s = session();
        let base = s.document.as_ref().unwrap().layers[0].id;
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        let mut target_layer = ImageLayer::blank("B", CGSize::new(100.0, 100.0));
        target_layer.asset = Some(ImportedImage::new(Bitmap::new(20, 20), "B"));
        let target = target_layer.id;
        s.document.as_mut().unwrap().layers.push(target_layer);
        s.toggle_clipping_mask(target);
        // Deleting the base with bake keeps the clipped look in the pixels.
        assert!(s.delete_with_live_mask_choice(&[base], true));
        let doc = s.document.as_ref().unwrap();
        assert!(doc.layer(base).is_none(), "the base is gone");
        let kept = doc.layer(target).unwrap();
        assert_eq!(kept.mask_source_id, None, "the link is gone");
        let image = kept.asset.as_ref().unwrap().image.as_ref().clone();
        assert_eq!(image.pixel(5, 5), [255, 0, 0, 255], "the base is baked in");
    }

    #[test]
    fn deleting_without_dependencies_reports_nothing() {
        let mut s = session();
        let first = s.document.as_ref().unwrap().layers[0].id;
        assert!(!s.delete_with_live_mask_choice(&[first], true));
    }

    #[test]
    fn plain_delete_bakes_a_live_mask_dependent_in_one_undo_step() {
        // deleteLayer routes through deleteWithLiveMaskChoice
        // (EditorSession.swift:637). The core's plain path bakes: the
        // dependent keeps its masked appearance, and the baked pixels live
        // inside the same undo step.
        let mut s = session();
        let base = s.document.as_ref().unwrap().layers[0].id;
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        let mut target_layer = ImageLayer::blank("B", CGSize::new(100.0, 100.0));
        target_layer.asset = Some(ImportedImage::new(Bitmap::new(20, 20), "B"));
        let target = target_layer.id;
        s.document.as_mut().unwrap().layers.push(target_layer);
        s.toggle_clipping_mask(target);
        s.delete_layer(base);
        let doc = s.document.as_ref().unwrap();
        assert!(doc.layer(base).is_none());
        let kept = doc.layer(target).unwrap();
        assert_eq!(kept.mask_source_id, None);
        let image = kept.asset.as_ref().unwrap().image.as_ref().clone();
        assert_eq!(image.pixel(5, 5), [255, 0, 0, 255], "the base is baked in");
        assert_eq!(s.history.undo_name(), "Delete Layer");
        s.undo();
        assert!(
            s.document.as_ref().unwrap().layer(base).is_some(),
            "one undo step restores everything"
        );
        // Multi-select: the passed ids are deleted (not the selection), the
        // baked pixels land in the "Delete Layers" after-snapshot, and undo
        // restores them.
        let mut s = session();
        let base = s.document.as_ref().unwrap().layers[0].id;
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        let mut target_layer = ImageLayer::blank("B", CGSize::new(100.0, 100.0));
        target_layer.asset = Some(ImportedImage::new(Bitmap::new(20, 20), "B"));
        let target = target_layer.id;
        s.document.as_mut().unwrap().layers.push(target_layer);
        s.toggle_clipping_mask(target);
        // Select the base plus an unrelated layer; the dependent stays.
        s.add_blank_layer();
        let extra = s.active_layer_id.unwrap();
        s.selected_layer_ids = vec![base, extra];
        s.delete_selected_layers();
        let doc = s.document.as_ref().unwrap();
        assert!(doc.layer(base).is_none());
        assert!(doc.layer(extra).is_none());
        assert!(doc.layer(target).is_some(), "the dependent stays");
        assert_eq!(s.history.undo_name(), "Delete Layers");
        let steps_before = s.history.undo_count();
        s.undo();
        assert_eq!(
            s.history.undo_count(),
            steps_before - 1,
            "the whole multi-delete is one undo step"
        );
        let doc = s.document.as_ref().unwrap();
        assert!(
            doc.layer(base).is_some() && doc.layer(extra).is_some(),
            "undo restores the deleted layers"
        );
        s.redo();
        let doc = s.document.as_ref().unwrap();
        assert!(doc.layer(base).is_none());
        let kept = doc.layer(target).unwrap();
        assert_eq!(
            kept.asset.as_ref().unwrap().image.pixel(5, 5),
            [255, 0, 0, 255],
            "redo keeps the baked pixels (they are in the after-snapshot)"
        );
    }

    #[test]
    fn distorting_resamples_pixels_into_the_shape() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(red_image(40), "L"));
        s.begin_transform();
        s.begin_distort();
        assert!(s.transform_edit.as_ref().unwrap().corners.is_some());
        // Drag the top-left corner inward; an unusable shape is refused.
        s.preview_corners([CGPoint::ZERO; 4]);
        assert!(s.transform_edit.as_ref().unwrap().corners.is_some());
        let shape = [
            CGPoint::new(10.0, 0.0),
            CGPoint::new(50.0, 0.0),
            CGPoint::new(50.0, 40.0),
            CGPoint::new(10.0, 40.0),
        ];
        s.preview_corners(shape);
        s.commit_transform();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 1);
        let layer = &doc.layers[0];
        assert_eq!(layer.transform.origin, CGPoint::new(10.0, 0.0));
        assert_eq!(layer.transform.size, CGSize::new(40.0, 40.0));
        let image = layer.asset.as_ref().unwrap().image.as_ref().clone();
        assert!(image.width >= 38 && image.width <= 42, "{}", image.width);
        // One undo step restores the un-distorted layer.
        assert_eq!(s.history.undo_name(), "Distort");
        s.undo();
        let layer = s.document.as_ref().unwrap().layers[0].clone();
        assert_eq!(layer.transform.origin, CGPoint::ZERO);
        assert_eq!(layer.asset.as_ref().unwrap().image.width, 40);
    }

    #[test]
    fn distorting_a_linked_mask_warps_it_with_the_pixels() {
        let mut s = session();
        let id = s.document.as_ref().unwrap().layers[0].id;
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(red_image(40), "L"));
        {
            let doc = s.document.as_mut().unwrap();
            let layer = doc.layer_mut(id).unwrap();
            let mut mask = LayerMask::solid(true);
            mask.is_linked = true;
            mask.asset = ImportedImage::new(red_image(40), "Layer Mask");
            layer.mask = Some(mask);
        }
        s.begin_transform();
        s.begin_distort();
        let shape = [
            CGPoint::new(10.0, 0.0),
            CGPoint::new(50.0, 0.0),
            CGPoint::new(50.0, 40.0),
            CGPoint::new(10.0, 40.0),
        ];
        s.preview_corners(shape);
        s.commit_transform();
        let doc = s.document.as_ref().unwrap();
        let layer = &doc.layers[0];
        let mask = layer.mask.as_ref().unwrap();
        // The mask warped with the pixels: still linked, no placement, and it
        // picked up the pixels' new bounds.
        assert!(mask.is_linked);
        assert!(mask.placement.is_none());
        assert!(mask.asset.image.width >= 38, "{}", mask.asset.image.width);
        assert_eq!(s.history.undo_name(), "Distort");
    }

    #[test]
    fn nudge_layer_commits_immediately() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(50, 50), "L"));
        s.nudge_layer(1.0, 0.0);
        assert_eq!(
            s.document
                .as_ref()
                .unwrap()
                .layer(id)
                .unwrap()
                .transform
                .origin
                .x,
            1.0
        );
        assert!(s.transform_edit.is_none());
    }

    #[test]
    fn flip_layers_toggles_flip_x() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(50, 50), "L"));
        s.flip_layers(true);
        let layer = s.document.as_ref().unwrap().layer(id).unwrap();
        assert!(layer.transform.flip_x);
        // The flip is about the layer's own middle: the center is unchanged.
        assert!((layer.transform.center().x - 50.0).abs() < 1e-9);
        assert!((layer.transform.center().y - 50.0).abs() < 1e-9);
    }

    #[test]
    fn merge_down_bakes_pixels() {
        let mut s = session();
        s.add_blank_layer();
        let top_id = s.document.as_ref().unwrap().layers[1].id;
        {
            let doc = s.document.as_mut().unwrap();
            doc.layers[0].asset = Some(ImportedImage::new(red_image(50), "bottom"));
            let mut blue = Bitmap::new(50, 50);
            for i in (0..blue.data.len()).step_by(4) {
                blue.data[i + 2] = 255;
                blue.data[i + 3] = 255;
            }
            doc.layers[1].asset = Some(ImportedImage::new(blue, "top"));
        }
        s.select_layer(Some(top_id));
        s.merge_layers();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].id, s.active_layer_id.unwrap());
        let out = doc.layers[0].asset.as_ref().unwrap().image.clone();
        assert_eq!(out.pixel(10, 10), [0, 0, 255, 255]);
    }

    #[test]
    fn merge_down_refuses_a_folder_below() {
        // The layer beneath in the same folder is a folder: nothing to merge
        // into (mergePlan refuses, it never skips it).
        let mut s = session();
        let mut folder = ImageLayer::blank("Folder", CGSize::new(100.0, 100.0));
        folder.is_group = true;
        let mut child = ImageLayer::blank("Inner", CGSize::new(100.0, 100.0));
        child.parent_id = Some(folder.id);
        let top = ImageLayer::blank("Layer 2", CGSize::new(100.0, 100.0));
        let top_id = top.id;
        s.document.as_mut().unwrap().layers = vec![folder, child, top];
        s.select_layer(Some(top_id));
        assert!(!s.can_merge_layers());
        assert_eq!(s.merge_title(), "Merge Down");
        let count = s.history.undo_count();
        s.merge_layers();
        assert_eq!(s.document.as_ref().unwrap().layers.len(), 3);
        assert_eq!(s.history.undo_count(), count);
    }

    #[test]
    fn merging_an_empty_folder_does_nothing() {
        let mut s = session();
        let mut folder = ImageLayer::blank("Folder", CGSize::new(100.0, 100.0));
        folder.is_group = true;
        let folder_id = folder.id;
        s.document.as_mut().unwrap().layers = vec![folder];
        s.select_layer(Some(folder_id));
        assert!(!s.can_merge_layers());
        let count = s.history.undo_count();
        s.merge_layers();
        assert_eq!(s.history.undo_count(), count);
    }

    #[test]
    fn merging_only_folders_does_nothing() {
        // A multi-selection merges only when something with pixels is in it.
        let mut s = session();
        let mut first = ImageLayer::blank("Folder 1", CGSize::new(100.0, 100.0));
        first.is_group = true;
        let mut second = ImageLayer::blank("Folder 2", CGSize::new(100.0, 100.0));
        second.is_group = true;
        let (first_id, second_id) = (first.id, second.id);
        s.document.as_mut().unwrap().layers = vec![first, second];
        s.active_layer_id = Some(first_id);
        s.selected_layer_ids = vec![first_id, second_id];
        assert!(!s.can_merge_layers());
        let count = s.history.undo_count();
        s.merge_layers();
        assert_eq!(s.history.undo_count(), count);
        assert_eq!(s.document.as_ref().unwrap().layers.len(), 2);
    }

    #[test]
    fn merging_a_folder_with_its_own_contents_backs_out() {
        // The result would parent to the very folder the merge removes, so the
        // hierarchy check refuses and nothing changes (LayerMerge.swift:66).
        let mut s = session();
        let mut folder = ImageLayer::blank("Folder", CGSize::new(100.0, 100.0));
        folder.is_group = true;
        let mut child = ImageLayer::blank("Inner", CGSize::new(100.0, 100.0));
        child.parent_id = Some(folder.id);
        child.asset = Some(ImportedImage::new(red_image(50), "Inner"));
        let (folder_id, child_id) = (folder.id, child.id);
        s.document.as_mut().unwrap().layers = vec![folder, child];
        s.active_layer_id = Some(child_id);
        s.selected_layer_ids = vec![folder_id, child_id];
        assert!(s.can_merge_layers(), "the plan itself is fine");
        let count = s.history.undo_count();
        s.merge_layers();
        assert_eq!(s.history.undo_count(), count, "validate fails, no merge");
        assert_eq!(s.document.as_ref().unwrap().layers.len(), 2);
    }

    #[test]
    fn merging_a_selection_names_the_result_for_the_topmost() {
        let mut s = session();
        s.add_blank_layer();
        {
            let doc = s.document.as_mut().unwrap();
            doc.layers[0].asset = Some(ImportedImage::new(red_image(50), "bottom"));
            doc.layers[1].asset = Some(ImportedImage::new(red_image(50), "top"));
        }
        let ids: Vec<Uuid> = s
            .document
            .as_ref()
            .unwrap()
            .layers
            .iter()
            .map(|l| l.id)
            .collect();
        s.selected_layer_ids = ids;
        assert_eq!(s.merge_title(), "Merge Layers");
        s.merge_layers();
        assert_eq!(s.history.undo_name(), "Merge Layers");
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 1);
        // The topmost selected layer names the result and owns its slot.
        assert_eq!(doc.layers[0].name, "Layer 2");
        assert_eq!(Some(doc.layers[0].id), s.active_layer_id);
    }

    #[test]
    fn merge_waits_for_the_crop_draft() {
        let mut s = session();
        s.add_blank_layer();
        s.crop_rect = Some(CGRect::new(0.0, 0.0, 50.0, 50.0));
        assert!(!s.can_merge_layers());
        let count = s.history.undo_count();
        s.merge_layers();
        assert_eq!(s.history.undo_count(), count);
        s.cancel_crop();
        assert!(s.can_merge_layers());
        assert_eq!(s.merge_title(), "Merge Down");
    }

    #[test]
    fn preview_blend_mode_is_temporary() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(10, 10), "L"));
        s.mark_saved();
        let id = s.active_layer_id.unwrap();
        let count = s.history.undo_count();
        s.set_preview_blend_mode(Some(LayerBlendMode::Multiply), Some(id));
        assert_eq!(s.blend_preview, Some((id, LayerBlendMode::Multiply)));
        assert_eq!(
            s.displayed_blend_mode(s.active_layer().unwrap()),
            LayerBlendMode::Multiply
        );
        assert_eq!(s.active_layer().unwrap().blend_mode, LayerBlendMode::Normal);
        assert_eq!(s.history.undo_count(), count, "a preview is no edit");
        assert!(!s.is_modified(), "and never touches the saved state");
        // Clearing, or a foreign layer id, drops it.
        s.set_preview_blend_mode(None, None);
        assert_eq!(s.blend_preview, None);
        s.set_preview_blend_mode(Some(LayerBlendMode::Multiply), Some(Uuid::new_v4()));
        assert_eq!(s.blend_preview, None);
        // Committing the mode drops it too.
        s.set_preview_blend_mode(Some(LayerBlendMode::Multiply), Some(id));
        s.set_layer_blend_mode(LayerBlendMode::Multiply);
        assert_eq!(s.blend_preview, None);
        assert_eq!(s.history.undo_name(), "Layer Blend Mode");
        // A committed transform drops it as well.
        s.set_preview_blend_mode(Some(LayerBlendMode::Multiply), Some(id));
        s.begin_transform();
        s.commit_transform();
        assert_eq!(s.blend_preview, None);
        // Folders have no appearance to preview.
        s.add_group();
        let folder = s.active_layer_id.unwrap();
        s.set_preview_blend_mode(Some(LayerBlendMode::Screen), Some(folder));
        assert_eq!(s.blend_preview, None);
    }

    #[test]
    fn nudges_open_a_transient_transform() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(50, 50), "L"));
        s.begin_transform_persistent(false);
        assert!(!s.transform_edit.as_ref().unwrap().persistent);
        s.cancel_transform();
        s.begin_transform();
        assert!(s.transform_edit.as_ref().unwrap().persistent);
        s.cancel_transform();
        // The nudge itself opens, moves, and commits in one step.
        s.nudge_layer(3.0, 0.0);
        assert!(s.transform_edit.is_none());
        assert_eq!(
            s.active_layer().unwrap().transform.origin,
            CGPoint::new(3.0, 0.0)
        );
        assert_eq!(s.history.undo_name(), "Transform Layer");
    }

    #[test]
    fn guides_add_clear() {
        let mut s = session();
        s.add_guide(GuideAxis::Vertical, 50.0);
        assert_eq!(s.document.as_ref().unwrap().guides.len(), 1);
        s.clear_guides();
        assert!(s.document.as_ref().unwrap().guides.is_empty());
    }

    #[test]
    fn crop_shifts_content() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(100, 100), "L"));
        s.crop_rect = Some(CGRect::new(25.0, 25.0, 50.0, 50.0));
        s.commit_crop();
        let doc = s.document.as_ref().unwrap();
        assert_eq!((doc.width, doc.height), (50, 50));
        assert_eq!(doc.layers[0].transform.origin, CGPoint::new(-25.0, -25.0));
    }

    #[test]
    fn canvas_and_image_size() {
        let mut s = session();
        let mut options = CanvasSizeOptions::new(150, 150);
        options.anchor = 0;
        s.resize_canvas_to(&options);
        assert_eq!(s.document.as_ref().unwrap().width, 150);
        s.resize_image_to(75, 75, Some(144.0), crate::transform::LayerSampling::High);
        assert_eq!(s.document.as_ref().unwrap().width, 75);
        assert_eq!(s.document.as_ref().unwrap().resolution, 144.0);
        assert_eq!(s.history.undo_name(), "Image Size");
    }

    #[test]
    fn trim_reports_nothing_to_trim() {
        let mut s = session();
        // A fully transparent layer: nothing remains to trim.
        let transparent = Bitmap::new(100, 100);
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(transparent, "L"));
        assert!(!s.trim(&TrimOptions::default()));
        // Content in one corner: trim crops to it.
        let mut image = Bitmap::new(100, 100);
        for y in 0..20 {
            for x in 0..20 {
                image.set_pixel(x, y, [255, 0, 0, 255]);
            }
        }
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(image, "L"));
        assert!(s.trim(&TrimOptions::default()));
        assert_eq!(s.document.as_ref().unwrap().width, 20);
    }

    #[test]
    fn palette_mask_painting_is_black_and_white() {
        let mut s = session();
        assert_eq!(s.palette_color(false), PaletteColor::BLACK);
        s.is_mask_selected = true;
        s.mask_paint_white = true;
        assert_eq!(s.palette_color(false), PaletteColor::WHITE);
        assert_eq!(s.palette_color(true), PaletteColor::BLACK);
        s.swap_palette_colors();
        assert_eq!(s.palette_color(false), PaletteColor::BLACK);
    }

    #[test]
    fn wand_settings_default_matches_photoshop() {
        let w = WandSettings::default();
        assert_eq!(w.tolerance, 32);
        assert!(w.contiguous);
        assert_eq!(w.sample_radius, 0);
    }

    /// A session whose active layer is a fresh Levels adjustment layer.
    fn levels_adjustment_session() -> (EditorSession, Uuid) {
        let mut s = session();
        s.add_adjustment(crate::adjustment::AdjustmentKind::Levels);
        let id = s.active_layer_id.unwrap();
        (s, id)
    }

    fn adjustment_of(s: &EditorSession, id: Uuid) -> crate::adjustment::LayerAdjustment {
        s.document
            .as_ref()
            .unwrap()
            .layer(id)
            .and_then(|layer| layer.adjustment.clone())
            .unwrap()
    }

    #[test]
    fn adjustment_editing_previews_and_commits_as_one_undo_step() {
        let (mut s, id) = levels_adjustment_session();
        let original = adjustment_of(&s, id);

        assert!(s.begin_adjustment_editing(id));
        assert_eq!(s.adjustment_editing_id, Some(id));
        assert_eq!(s.adjustment_original.as_ref(), Some(&original));

        let mut settings = crate::adjust::LevelsSettings::default();
        settings.ranges[0].black = 20.0;
        s.levels = Some(settings);
        // Preview writes the editor's settings into the layer's metadata.
        assert!(s.preview_adjustment_editing(true));
        assert_eq!(adjustment_of(&s, id).levels, settings);
        // Unchecking preview restores the opening value.
        assert!(s.preview_adjustment_editing(false));
        assert_eq!(adjustment_of(&s, id), original);
        assert!(s.preview_adjustment_editing(true));

        // OK keeps the settings; the whole session folded into one step.
        assert!(s.finish_adjustment_editing(true));
        assert_eq!(adjustment_of(&s, id).levels, settings);
        assert_eq!(s.history.undo_name(), "Edit Levels Adjustment");
        assert_eq!(s.adjustment_editing_id, None);
        assert_eq!(s.adjustment_original, None);
        assert_eq!(s.levels, None);

        s.undo();
        assert_eq!(adjustment_of(&s, id), original, "undo reverts the edit");
    }

    #[test]
    fn adjustment_editing_cancel_restores_exactly() {
        let (mut s, id) = levels_adjustment_session();
        let original = adjustment_of(&s, id);
        assert!(s.begin_adjustment_editing(id));

        let mut settings = crate::adjust::LevelsSettings::default();
        settings.ranges[0].white = 200.0;
        s.levels = Some(settings);
        assert!(s.preview_adjustment_editing(true));
        assert_eq!(adjustment_of(&s, id).levels.ranges[0].white, 200.0);

        assert!(s.finish_adjustment_editing(false));
        assert_eq!(
            adjustment_of(&s, id),
            original,
            "Cancel restores the opening value"
        );
        // The preview and the cancel never became an undo step.
        assert_eq!(s.history.undo_name(), "New Levels Adjustment");
        assert_eq!(s.adjustment_editing_id, None);
        assert_eq!(s.adjustment_original, None);
        assert_eq!(s.levels, None);
    }

    #[test]
    fn adjustment_editing_stays_bound_across_layer_switches() {
        let (mut s, id) = levels_adjustment_session();
        let pixel = s.document.as_ref().unwrap().layers[0].id;
        assert!(s.begin_adjustment_editing(id));
        // The user clicks another layer while the panel is open.
        s.select_layer(Some(pixel));
        assert_eq!(s.active_layer_id, Some(pixel));

        let mut settings = crate::adjust::LevelsSettings::default();
        settings.ranges[0].gamma = 2.0;
        s.levels = Some(settings);
        assert!(s.preview_adjustment_editing(true));
        assert_eq!(
            adjustment_of(&s, id).levels,
            settings,
            "the edited layer follows"
        );
        assert!(
            s.document
                .as_ref()
                .unwrap()
                .layer(pixel)
                .and_then(|layer| layer.adjustment.clone())
                .is_none(),
            "the selected layer is untouched"
        );
        assert!(s.finish_adjustment_editing(true));
        assert_eq!(adjustment_of(&s, id).levels, settings);
    }

    #[test]
    fn adjustment_editing_refuses_conflicts() {
        let (mut s, id) = levels_adjustment_session();
        let blank = s.document.as_ref().unwrap().layers[0].id;

        // No adjustment on the layer, or the id unknown.
        assert!(!s.begin_adjustment_editing(blank));
        assert!(!s.begin_adjustment_editing(Uuid::new_v4()));

        // Open, and already open.
        assert!(s.begin_adjustment_editing(id));
        assert!(!s.begin_adjustment_editing(id));
        s.finish_adjustment_editing(true);

        // An open editor (Levels) blocks a new editing session.
        s.levels = Some(crate::adjust::LevelsSettings::default());
        assert!(!s.begin_adjustment_editing(id));
    }

    #[test]
    fn adjustment_editing_carries_filter_settings_back() {
        let mut s = session();
        s.add_adjustment(crate::adjustment::AdjustmentKind::Exposure);
        let id = s.active_layer_id.unwrap();
        assert!(s.begin_adjustment_editing(id));

        s.filter_edit = Some(crate::filters::FilterSettings {
            exposure: crate::adjust::ExposureSettings {
                exposure: 1.5,
                offset: 0.0,
                gamma: 1.0,
            },
            ..Default::default()
        });
        assert_eq!(
            s.edited_adjustment().unwrap().exposure().exposure,
            1.5,
            "the edited exposure rides on the opening value"
        );
        assert!(s.preview_adjustment_editing(true));
        assert_eq!(adjustment_of(&s, id).exposure().exposure, 1.5);
        assert!(s.finish_adjustment_editing(true));
        assert_eq!(adjustment_of(&s, id).exposure().exposure, 1.5);
        assert_eq!(s.history.undo_name(), "Edit Exposure Adjustment");
        assert_eq!(s.filter_edit, None);

        // A blur adjustment takes the radius; Hue/Saturation its settings.
        s.add_adjustment(crate::adjustment::AdjustmentKind::GaussianBlur);
        let blur = s.active_layer_id.unwrap();
        assert!(s.begin_adjustment_editing(blur));
        s.filter_edit = Some(crate::filters::FilterSettings {
            radius: 25.0,
            ..Default::default()
        });
        assert!(s.preview_adjustment_editing(true));
        assert_eq!(adjustment_of(&s, blur).gaussian_radius(), 25.0);
        assert!(s.finish_adjustment_editing(true));

        s.add_adjustment(crate::adjustment::AdjustmentKind::Hsv);
        let hsv = s.active_layer_id.unwrap();
        assert!(s.begin_adjustment_editing(hsv));
        s.hue_saturation = Some(crate::adjust::HueSaturationSettings::new(
            30.0, 10.0, 0.0, false,
        ));
        assert_eq!(s.edited_adjustment().unwrap().resolved_hsv().hue(), 30.0);
        assert!(s.preview_adjustment_editing(true));
        assert_eq!(adjustment_of(&s, hsv).resolved_hsv().hue(), 30.0);
        assert!(s.finish_adjustment_editing(true));
        assert_eq!(s.history.undo_name(), "Edit Hue/Saturation Adjustment");
    }

    #[test]
    fn adjustment_editing_needs_an_open_edit() {
        let mut s = session();
        assert!(!s.preview_adjustment_editing(true));
        assert!(!s.finish_adjustment_editing(true));
        assert!(s.edited_adjustment().is_none());

        // Invert has no settings: the opening value is all there is.
        s.add_adjustment(crate::adjustment::AdjustmentKind::Invert);
        let id = s.active_layer_id.unwrap();
        assert!(s.begin_adjustment_editing(id));
        let original = adjustment_of(&s, id);
        assert_eq!(s.edited_adjustment().as_ref(), Some(&original));
        assert!(s.preview_adjustment_editing(true));
        assert_eq!(adjustment_of(&s, id), original);
        assert!(s.finish_adjustment_editing(true));
    }
}

/// Tests for `installProject` / `clearProject`
/// (Document/EditorSession+Projects.swift:20-53).
#[cfg(test)]
mod project_install_tests {
    use super::*;
    use crate::raster::{Bitmap, ImportedImage};

    fn loaded_session() -> (EditorSession, Uuid, crate::project::ProjectData) {
        let mut session = EditorSession::new();
        session.create_document(64, 64, false);
        session.insert_image(
            ImportedImage::new(Bitmap::new_clearing(16, 16, [255, 0, 0, 255]), "a"),
            None,
        );
        let active = session.active_layer_id.unwrap();
        let mut data = crate::project::project_from_document(session.document.as_ref().unwrap());
        data.manifest.active_layer_id = Some(active);
        (session, active, data)
    }

    /// `installProject`: the loaded package replaces everything, the active
    /// layer is the manifest's, the URL is recorded, and the session comes out
    /// unmodified with an empty undo stack.
    #[test]
    fn install_project_replaces_state_and_marks_unmodified() {
        let (_source, active, data) = loaded_session();

        // A live session with history and a pending transform dies entirely.
        let mut target = EditorSession::new();
        target.create_document(10, 10, true);
        target.add_blank_layer();
        assert!(target.is_modified());
        let url = std::path::Path::new("Z:/tmp/Sample.comp");
        target.install_project(&data, url).unwrap();

        let document = target.document.as_ref().unwrap();
        assert_eq!(document.width, 64);
        assert_eq!(document.layers.len(), 1);
        assert_eq!(target.active_layer_id, Some(active));
        assert_eq!(target.selected_layer_ids, vec![active]);
        assert_eq!(target.project_url.as_deref(), Some(url));
        assert!(!target.is_modified());
        // `history.reset()`: open starts from a clean undo stack.
        assert_eq!(target.history.undo_count(), 0);
        assert!(!target.can_undo());
        assert!(!target.can_redo());

        // A damaged package leaves the live session untouched.
        let mut broken = data.clone();
        broken.manifest.width = 0;
        let before = target.document.clone();
        assert!(target.install_project(&broken, url).is_err());
        assert!(target.document == before);
        assert_eq!(target.project_url.as_deref(), Some(url));
    }

    /// `clearProject`: document, selection, URL and history all drop.
    #[test]
    fn clear_project_leaves_an_empty_unmodified_session() {
        let (_source, _active, data) = loaded_session();
        let mut target = EditorSession::new();
        target
            .install_project(&data, std::path::Path::new("Z:/tmp/Sample.comp"))
            .unwrap();
        assert!(!target.is_modified());

        target.clear_project();
        assert!(target.document.is_none());
        assert_eq!(target.active_layer_id, None);
        assert!(target.selected_layer_ids.is_empty());
        assert_eq!(target.project_url, None);
        assert!(!target.is_modified());
        assert_eq!(target.history.undo_count(), 0);
    }

    // MARK: Project-operation busy guards (stage-D ticket 34)

    /// `canStartProjectOperation` (EditorSession.swift:150-152), condition
    /// for condition where a flag is cheap to raise; plus
    /// `begin_project_access`/`end_project_access`
    /// (ProjectController.swift:16-22): guard, then the crop cancel +
    /// transform commit that open the busy hold.
    #[test]
    fn can_start_project_operation_and_busy_access() {
        let mut s = EditorSession::new();
        assert!(s.can_start_project_operation());

        s.is_project_busy = true;
        assert!(!s.can_start_project_operation());
        s.is_project_busy = false;
        s.is_importing = true;
        assert!(!s.can_start_project_operation());
        s.is_importing = false;
        s.has_brush_stroke = true;
        assert!(!s.can_start_project_operation());
        s.has_brush_stroke = false;
        s.shows_new_document = true;
        assert!(!s.can_start_project_operation());
        s.shows_new_document = false;
        s.shows_importer = true;
        assert!(!s.can_start_project_operation());
        s.shows_importer = false;
        s.renaming_layer_id = Some(Uuid::new_v4());
        assert!(!s.can_start_project_operation());
        s.renaming_layer_id = None;
        s.import_error = Some("x: the image could not be read.".into());
        assert!(!s.can_start_project_operation());
        s.import_error = None;
        s.levels = Some(crate::adjust::LevelsSettings::default());
        assert!(!s.can_start_project_operation());
        s.levels = None;
        s.shows_conversion_sheet = true;
        assert!(!s.can_start_project_operation());
        s.shows_conversion_sheet = false;
        assert!(s.can_start_project_operation());

        // `begin` (:16-22): the pending crop frame dies as the busy hold
        // starts, a second begin refuses, and the release restores the
        // guard.
        let mut s = EditorSession::new();
        s.create_document(100, 100, true);
        s.select_tool(NavigationTool::Crop);
        assert!(s.crop_rect.is_some());
        assert!(s.begin_project_access());
        assert!(s.is_project_busy);
        assert!(s.crop_rect.is_none());
        assert!(!s.can_start_project_operation());
        assert!(!s.begin_project_access());
        s.end_project_access();
        assert!(s.can_start_project_operation());
    }

    /// Port of `HistoryTests.historyBlockedDuringImportsAndDialogs`
    /// (:83-101) — complete since stage-D ticket 35 added the session's
    /// `shows_importer`.
    #[test]
    fn history_blocked_during_imports_and_dialogs() {
        let mut s = EditorSession::new();
        s.create_document(40, 40, false);

        s.is_importing = true;
        s.undo();
        assert!(s.document.is_some());
        s.is_importing = false;

        s.shows_new_document = true;
        s.undo();
        assert!(s.document.is_some());
        s.shows_new_document = false;

        s.undo();
        assert!(s.document.is_none());
        s.shows_importer = true;
        s.redo();
        assert!(s.document.is_none());
        s.shows_importer = false;
        s.redo();
        assert!(s.document.is_some());
    }

    /// Port of `ProjectTests.projectOperationsBlockEditsAndQueueImageImports`
    /// (:181-199), synchronous half: with the busy hold on, New and Undo
    /// refuse. Swift's `deleteActiveLayer` under busy and the queued-import
    /// wait (`waitForProjectAccess` :302-317) have no core counterpart —
    /// the layer-delete guards are a per-callsite batch, and the port's
    /// imports are synchronous with the app draining its queue when the
    /// guards clear (workspace deviation #5).
    #[test]
    fn project_operations_block_edits_while_busy() {
        let mut s = EditorSession::new();
        s.create_document(100, 100, true);
        s.add_blank_layer();

        s.is_project_busy = true;
        s.create_document(400, 400, true);
        assert_eq!(s.document.as_ref().unwrap().width, 100);
        s.undo();
        assert_eq!(s.document.as_ref().unwrap().layers.len(), 2);

        s.is_project_busy = false;
        s.create_document(400, 400, true);
        assert_eq!(s.document.as_ref().unwrap().width, 400);

        s.clear_project();
        assert!(s.document.is_none() && s.project_url.is_none());
    }
}
