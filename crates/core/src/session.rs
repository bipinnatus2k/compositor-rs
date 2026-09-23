//! Port of the session layer: `EditorSession` and its extensions
//! (`EditorSession.swift`, `Selection.swift`, `LayerAppearance.swift`,
//! `LayerMask.swift`, `LayerGroups.swift`, `LayerMerge.swift`,
//! `LayerFlip.swift`, `DocumentHistory` integration, `Crop.swift`,
//! `CanvasSize`/`ImageSize`/`Trim` operations).

#![allow(dead_code)]

use crate::blend::LayerBlendMode;
use crate::canvas::{
    mirrored_transform, resize_canvas, resize_image, trim as trim_document, CanvasSizeOptions,
    CropDrag, CropDragMode, CropGeometry, TrimOptions,
};
use crate::document::{
    CanvasDocument, CanvasGuide, GuideAxis, ImageLayer, LayerMask, LayerTextStyle, NavigationTool,
    PaletteColor,
};
use crate::geometry::{CGPoint, CGRect, CGSize};
use crate::history::DocumentHistory;
use crate::raster::{Bitmap, ImportedImage};
use crate::selection::{
    drag_box_rect, DocumentSelection, LassoDraft, LassoKind, SelectionMode, SelectionPath,
};
use crate::transform::{LayerTransform, TransformEdit};
use uuid::Uuid;

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
/// `@Observable`; here the UI reads the fields directly and mutation methods
/// keep the same guards and undo names.
pub struct EditorSession {
    pub document: Option<CanvasDocument>,
    pub tool: NavigationTool,
    pub active_layer_id: Option<Uuid>,
    pub selected_layer_ids: Vec<Uuid>,
    pub is_mask_selected: bool,
    pub collapsed_group_ids: Vec<Uuid>,
    pub history: DocumentHistory,

    // Appearance preview
    pub blend_preview: Option<(Uuid, LayerBlendMode)>,
    pub opacity_edit_layer_id: Option<Uuid>,

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

    // Transform
    pub transform_edit: Option<TransformEdit>,
    pub locks_transform_ratio: bool,
    pub snapping_enabled: bool,

    // Crop
    pub crop_rect: Option<CGRect>,
    pub crop_ratio_choice: String,
    pub crop_drag: Option<CropDrag>,

    // Sheets (the UI reads these to present its dialogs)
    pub shows_new_document: bool,
    pub levels: Option<crate::adjust::LevelsSettings>,
    pub levels_layer_id: Option<Uuid>,
    pub hue_saturation: Option<crate::adjust::HueSaturationSettings>,
    pub hue_saturation_layer_id: Option<Uuid>,

    // Guides
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
            collapsed_group_ids: Vec::new(),
            history: DocumentHistory::default(),
            blend_preview: None,
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
            transform_edit: None,
            locks_transform_ratio: true,
            snapping_enabled: true,
            crop_rect: None,
            crop_ratio_choice: "Free".to_string(),
            crop_drag: None,
            shows_new_document: false,
            levels: None,
            levels_layer_id: None,
            hue_saturation: None,
            hue_saturation_layer_id: None,
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

    /// `createDocument` — File > New.
    pub fn create_document(&mut self, width: usize, height: usize, empty_layer: bool) {
        if !(1..=30_000).contains(&width) || !(1..=30_000).contains(&height) {
            return;
        }
        self.commit_transform();
        self.begin_edit("New Canvas");
        let mut document = CanvasDocument::new(width, height);
        let layer = if empty_layer {
            Some(ImageLayer::blank("Layer 1", document.size()))
        } else {
            None
        };
        let layer_id = layer.as_ref().map(|l| l.id);
        if let Some(layer) = layer {
            document.layers = vec![layer];
        }
        self.document = Some(document);
        self.active_layer_id = layer_id;
        self.selected_layer_ids = layer_id.map(|i| vec![i]).unwrap_or_default();
        self.history.end(&self.document, self.active_layer_id);
        self.shows_new_document = false;
    }

    pub fn document_size(&self) -> Option<CGSize> {
        self.document.as_ref().map(|d| d.size())
    }

    pub fn active_layer(&self) -> Option<&ImageLayer> {
        let id = self.active_layer_id?;
        self.document.as_ref()?.layer(id)
    }

    pub fn selection(&self) -> Option<&DocumentSelection> {
        self.document.as_ref()?.selection.as_ref()
    }

    // MARK: Undo

    fn begin_edit(&mut self, name: &str) {
        self.history.begin(name, &self.document, self.active_layer_id);
    }

    fn end_edit(&mut self) {
        self.history.end(&self.document, self.active_layer_id);
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn undo(&mut self) {
        if let Some(snapshot) = self.history.undo() {
            self.document = snapshot.document;
            self.active_layer_id = snapshot.active_layer_id;
            if let Some(id) = self.active_layer_id {
                self.selected_layer_ids = vec![id];
            }
        }
    }

    pub fn redo(&mut self) {
        if let Some(snapshot) = self.history.redo() {
            self.document = snapshot.document;
            self.active_layer_id = snapshot.active_layer_id;
            if let Some(id) = self.active_layer_id {
                self.selected_layer_ids = vec![id];
            }
        }
    }

    pub fn is_modified(&self) -> bool {
        self.history.is_modified()
    }

    pub fn mark_saved(&mut self) {
        self.history.mark_saved();
    }

    // MARK: Layers

    pub fn select_layer(&mut self, id: Option<Uuid>) {
        if id != self.active_layer_id {
            self.is_mask_selected = false;
        }
        self.active_layer_id = id;
        self.selected_layer_ids = id.map(|i| vec![i]).unwrap_or_default();
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
        let insertion = self
            .active_layer_id
            .and_then(|id| document.layer_index(id))
            .map(|i| i + 1)
            .unwrap_or(document.layers.len());
        self.begin_edit("New Blank Layer");
        let id = layer.id;
        if let Some(doc) = self.document.as_mut() {
            doc.layers.insert(insertion.min(doc.layers.len()), layer);
            self.active_layer_id = Some(id);
            self.selected_layer_ids = vec![id];
        }
        self.end_edit();
    }

    /// `deleteLayer` — a selected folder takes its contents.
    pub fn delete_layer(&mut self, id: Uuid) {
        let Some(document) = self.document.clone() else {
            return;
        };
        if document.layer(id).is_none() {
            return;
        }
        let removed = document.descendant_ids(id);
        self.begin_edit("Delete Layer");
        if let Some(doc) = self.document.as_mut() {
            doc.layers.retain(|l| !removed.contains(&l.id));
        }
        if self.active_layer_id == Some(id) {
            self.active_layer_id = None;
            self.selected_layer_ids.clear();
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
        if self.document.as_ref().and_then(|d| d.layer(id)).is_some() {
            self.begin_edit("Rename Layer");
            if let Some(doc) = self.document.as_mut() {
                doc.layer_mut(id).unwrap().name = name.to_string();
            }
            self.end_edit();
        }
    }

    pub fn toggle_layer_visibility(&mut self, id: Uuid) {
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

    /// `duplicateActiveLayer`: a copy above the original.
    pub fn duplicate_active_layer(&mut self) {
        let Some(active_id) = self.active_layer_id else {
            return;
        };
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(source) = document.layer(active_id).cloned() else {
            return;
        };
        let mut copy = source.clone();
        copy.id = Uuid::new_v4();
        copy.name = format!("{} copy", source.name);
        self.begin_edit("Duplicate Layer");
        if let Some(doc) = self.document.as_mut() {
            let index = doc.layer_index(active_id).unwrap_or(doc.layers.len() - 1);
            let copy_id = copy.id;
            doc.layers.insert(index + 1, copy);
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
        let raster = crate::render::render_shape_image(&shape, transform.size.width.round() as usize, transform.size.height.round() as usize);
        let mut layer = ImageLayer::with_asset(ImportedImage::new(raster, name.clone()), rect.origin);
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

    pub fn can_edit_appearance(&self) -> bool {
        self.selected_layer_ids.len() == 1
            && self.active_layer().map(|l| !l.is_group).unwrap_or(false)
    }

    pub fn displayed_blend_mode(&self, layer: &ImageLayer) -> LayerBlendMode {
        if let Some((id, mode)) = self.blend_preview {
            if id == layer.id && self.active_layer_id == Some(layer.id) {
                return mode;
            }
        }
        layer.blend_mode
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
        let index = modes.iter().position(|m| *m == layer.blend_mode).unwrap_or(0);
        let next = modes[(index + if forward { 1 } else { modes.len() - 1 }) % modes.len()];
        self.set_layer_blend_mode(next);
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
        if self.opacity_edit_layer_id.is_none() {
            if let Some(id) = self.active_layer_id {
                self.begin_edit("Layer Opacity");
                self.opacity_edit_layer_id = Some(id);
            }
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
        if let Some(doc) = self.document.as_mut() {
            if let Some(layer) = doc.layer_mut(id) {
                layer.opacity = opacity.clamp(0.0, 1.0);
            }
        }
        if standalone {
            self.end_edit();
        }
    }

    // MARK: Selections

    pub fn selection_mode(&self, shift: bool, option: bool) -> SelectionMode {
        if option {
            SelectionMode::Subtract
        } else if shift {
            SelectionMode::Add
        } else {
            self.selection_mode_choice
        }
    }

    pub fn begin_lasso(&mut self, at: CGPoint, mode: SelectionMode) {
        if !self.tool.is_selection_tool() || self.tool == NavigationTool::Wand {
            return;
        }
        if self.tool == NavigationTool::Marquee {
            let anchor = CGPoint::new(at.x.round(), at.y.round());
            self.lasso_draft = Some(LassoDraft {
                points: vec![anchor],
                cursor: None,
                mode,
                kind: self.marquee_kind,
                anchor: Some(anchor),
            });
        } else {
            self.lasso_draft = Some(LassoDraft {
                points: vec![at],
                cursor: None,
                mode,
                kind: self.lasso_kind,
                anchor: None,
            });
        }
    }

    /// Marquee drag, snapped to whole pixels.
    pub fn drag_marquee(&mut self, to: CGPoint, square: bool, from_center: bool) {
        let Some(draft) = self.lasso_draft.as_mut() else {
            return;
        };
        if !matches!(draft.kind, LassoKind::Rectangle | LassoKind::Ellipse) {
            return;
        }
        let Some(anchor) = draft.anchor else { return };
        let rect = drag_box_rect(anchor, to, square, from_center);
        draft.points = vec![
            CGPoint::new(rect.min_x(), rect.min_y()),
            CGPoint::new(rect.max_x(), rect.min_y()),
            CGPoint::new(rect.max_x(), rect.max_y()),
            CGPoint::new(rect.min_x(), rect.max_y()),
        ];
    }

    /// Adds an outline point; points closer than a quarter pixel are skipped.
    pub fn extend_lasso(&mut self, to: CGPoint) {
        let Some(draft) = self.lasso_draft.as_mut() else {
            return;
        };
        if let Some(last) = draft.points.last() {
            if last.distance_to(to) < 0.25 {
                return;
            }
        }
        draft.points.push(to);
    }

    pub fn move_lasso_cursor(&mut self, to: Option<CGPoint>) {
        if let Some(draft) = self.lasso_draft.as_mut() {
            draft.cursor = to;
        }
    }

    pub fn remove_last_lasso_point(&mut self) {
        if let Some(draft) = self.lasso_draft.as_mut() {
            draft.points.pop();
            if draft.points.is_empty() {
                self.lasso_draft = None;
            }
        }
    }

    pub fn cancel_lasso(&mut self) {
        self.lasso_draft = None;
    }

    pub fn toggle_marquee_kind(&mut self) {
        self.cancel_lasso();
        self.marquee_kind = if self.marquee_kind == LassoKind::Rectangle {
            LassoKind::Ellipse
        } else {
            LassoKind::Rectangle
        };
    }

    pub fn toggle_lasso_kind(&mut self) {
        self.cancel_lasso();
        self.lasso_kind = if self.lasso_kind == LassoKind::Freehand {
            LassoKind::Polygonal
        } else {
            LassoKind::Freehand
        };
    }

    /// Closes the outline and combines it with the current selection.
    pub fn finish_lasso(&mut self) {
        let Some(draft) = self.lasso_draft.take() else {
            return;
        };
        let outline = if draft.kind == LassoKind::Ellipse && draft.points.len() == 4 {
            let xs: Vec<f64> = draft.points.iter().map(|p| p.x).collect();
            let ys: Vec<f64> = draft.points.iter().map(|p| p.y).collect();
            let min_x = xs.iter().cloned().fold(f64::INFINITY, f64::min);
            let max_x = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let min_y = ys.iter().cloned().fold(f64::INFINITY, f64::min);
            let max_y = ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            SelectionPath::ellipse(CGRect::new(min_x, min_y, max_x - min_x, max_y - min_y))
        } else {
            if draft.points.len() < 3 {
                if draft.mode == SelectionMode::Replace {
                    self.deselect();
                }
                return;
            }
            SelectionPath::from_polygon(draft.points.clone())
        };
        let bounds_valid = outline
            .bounds()
            .map(|b| b.width() > 0.0 && b.height() > 0.0)
            .unwrap_or(false);
        if !bounds_valid {
            if draft.mode == SelectionMode::Replace {
                self.deselect();
            }
            return;
        }
        let name = match draft.kind {
            LassoKind::Freehand => "Lasso",
            LassoKind::Polygonal => "Polygonal Lasso",
            LassoKind::Ellipse => "Elliptical Marquee",
            LassoKind::Rectangle => "Rectangular Marquee",
        };
        self.apply_selection(outline, draft.mode, name);
    }

    pub fn apply_selection(&mut self, shape: SelectionPath, mode: SelectionMode, name: &str) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let canvas = SelectionPath::rect(CGRect::from_origin_size(CGPoint::ZERO, document.size()));
        let clipped = shape.intersection(&canvas);
        let result = match mode {
            SelectionMode::Replace => clipped,
            SelectionMode::Add => match &document.selection {
                Some(current) => current.path.union(&clipped),
                None => clipped,
            },
            SelectionMode::Subtract => {
                let Some(current) = &document.selection else {
                    return;
                };
                current.path.subtract(&clipped)
            }
        };
        self.set_selection(
            Some(DocumentSelection {
                path: result,
                antialiased: self.selection_antialiased,
                feather: 0.0,
            }),
            name,
        );
    }

    pub fn set_selection(&mut self, value: Option<DocumentSelection>, name: &str) {
        {
            let current = self.document.as_ref().and_then(|d| d.selection.clone());
            if current == value {
                return;
            }
        }
        self.begin_edit(name);
        if let Some(doc) = self.document.as_mut() {
            doc.selection = value;
        }
        self.end_edit();
    }

    pub fn select_all(&mut self) {
        let Some(document) = self.document.clone() else {
            return;
        };
        self.set_selection(
            Some(DocumentSelection::new(SelectionPath::rect(
                CGRect::from_origin_size(CGPoint::ZERO, document.size()),
            ))),
            "Select All",
        );
    }

    pub fn deselect(&mut self) {
        if self.selection().is_some() {
            self.set_selection(None, "Deselect");
        }
    }

    pub fn invert_selection(&mut self) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(current) = document.selection.clone() else {
            return;
        };
        let canvas = SelectionPath::rect(CGRect::from_origin_size(CGPoint::ZERO, document.size()));
        self.set_selection(
            Some(DocumentSelection {
                path: canvas.subtract(&current.path),
                antialiased: current.antialiased,
                feather: current.feather,
            }),
            "Inverse",
        );
    }

    /// Arrow-key nudge: 1 px, or 10 px with Shift. Each press is one undo step.
    pub fn nudge_selection(&mut self, dx: f64, dy: f64) {
        if self.selection().is_none() {
            return;
        }
        self.begin_edit("Move Selection");
        if let Some(doc) = self.document.as_mut() {
            if let Some(selection) = doc.selection.as_mut() {
                selection.path = selection.path.translated(dx, dy);
            }
        }
        self.end_edit();
    }

    pub fn can_modify_selection(&self) -> bool {
        self.selection().map(|s| !s.is_empty()).unwrap_or(false)
    }

    pub fn expand_selection(&mut self, amount: i64) {
        self.resize_selection(amount as f64, "Expand Selection");
    }

    pub fn contract_selection(&mut self, amount: i64) {
        self.resize_selection(-(amount as f64), "Contract Selection");
    }

    fn resize_selection(&mut self, delta: f64, name: &str) {
        if !self.can_modify_selection() || delta == 0.0 || delta.abs() > 500.0 {
            return;
        }
        let current = self.document.as_ref().unwrap().selection.clone().unwrap();
        // A stroke of width 2|δ| centered on the outline extends |δ| per side.
        let band = current.path.stroke_band(delta.abs() * 2.0);
        let result = if delta > 0.0 {
            let grown = current.path.union(&band);
            if let Some(document) = &self.document {
                let canvas =
                    SelectionPath::rect(CGRect::from_origin_size(CGPoint::ZERO, document.size()));
                grown.intersection(&canvas)
            } else {
                grown
            }
        } else {
            current.path.subtract(&band)
        };
        self.set_selection(
            Some(DocumentSelection {
                path: result,
                antialiased: current.antialiased,
                feather: current.feather,
            }),
            name,
        );
    }

    /// Softens the current selection's edge (`Feather`).
    pub fn feather_selection(&mut self, amount: i64) {
        if !self.can_modify_selection() || amount <= 0 {
            return;
        }
        let current = self.document.as_ref().unwrap().selection.clone().unwrap();
        let softened = (current.feather * current.feather + amount as f64 * amount as f64).sqrt();
        self.set_selection(
            Some(DocumentSelection {
                path: current.path.clone(),
                antialiased: current.antialiased,
                feather: softened.min(250.0),
            }),
            "Feather Selection",
        );
    }

    pub fn confirm_selection_amount(&mut self, amount: i64) {
        let Some(operation) = self.selection_amount_operation else {
            return;
        };
        let limit = if operation == SelectionAmountOperation::Feather {
            250
        } else {
            500
        };
        if !(1..=limit).contains(&amount) {
            return;
        }
        self.selection_amount_operation = None;
        match operation {
            SelectionAmountOperation::Expand => {
                self.selection_expand_amount = amount;
                self.expand_selection(amount);
            }
            SelectionAmountOperation::Contract => {
                self.selection_contract_amount = amount;
                self.contract_selection(amount);
            }
            SelectionAmountOperation::Feather => {
                self.selection_feather_amount = amount;
                self.feather_selection(amount);
            }
        }
    }

    /// The Magic Wand: selects pixels similar to the one at `point`, read from
    /// the active layer (or the composite), combined by `mode`.
    pub fn magic_wand(&mut self, point: CGPoint, mode: SelectionMode) {
        let Some(document) = self.document.clone() else {
            return;
        };
        if point.x < 0.0
            || point.y < 0.0
            || point.x >= document.width as f64
            || point.y >= document.height as f64
        {
            return;
        }
        let Some(sample) = self.selection_sample(&document) else {
            return;
        };
        let x = point.x.floor() as usize;
        let y = point.y.floor() as usize;
        let mut mask = vec![0u8; document.width * document.height];
        let count = crate::tools::wand::wand_mask(
            &sample,
            (x, y),
            self.wand_settings.sample_radius,
            self.wand_settings.tolerance.clamp(0, 255),
            self.wand_settings.contiguous,
            &mut mask,
        );
        let Some(count) = count else {
            return;
        };
        if count <= 0 {
            if mode == SelectionMode::Replace {
                self.deselect();
            }
            return;
        }
        let outline = match crate::tools::wand::wand_trace(&mask, document.width, document.height) {
            Ok((points, loops)) => {
                let mut path = SelectionPath::default();
                let mut index = 0usize;
                for length in loops {
                    let len = length as usize;
                    path.rings.push(
                        points[index..index + len]
                            .iter()
                            .map(|(x, y)| CGPoint::new(*x as f64, *y as f64))
                            .collect(),
                    );
                    index += len;
                }
                path
            }
            Err(_) => return,
        };
        if mode == SelectionMode::Replace {
            self.set_selection(
                Some(DocumentSelection {
                    path: outline,
                    antialiased: self.selection_antialiased,
                    feather: 0.0,
                }),
                "Magic Wand",
            );
        } else {
            self.apply_selection(outline, mode, "Magic Wand");
        }
    }

    /// What selection-from-image tools read, at document size.
    pub fn selection_sample(&self, document: &CanvasDocument) -> Option<Bitmap> {
        if self.wand_settings.sample_all_layers {
            Some(crate::render::draw_live_composite(document))
        } else {
            let layer = self.active_layer()?;
            let asset = layer.asset.as_ref()?;
            let mut out = Bitmap::new(document.width, document.height);
            crate::render::draw_layer_pixels(asset.image.as_ref(), &layer.transform, &mut out);
            Some(out)
        }
    }

    // MARK: Masks

    pub fn can_edit_mask(&self) -> bool {
        self.selected_layer_ids.len() == 1 && self.active_layer().is_some()
    }

    pub fn add_layer_mask(&mut self, revealing: bool) {
        if !self.can_edit_mask() || self.active_layer().map(|l| l.mask.is_some()).unwrap_or(true) {
            return;
        }
        let Some(id) = self.active_layer_id else {
            return;
        };
        self.finish_opacity_edit();
        self.begin_edit(if revealing {
            "Add Reveal-All Mask"
        } else {
            "Add Hide-All Mask"
        });
        if let Some(doc) = self.document.as_mut() {
            doc.layer_mut(id).unwrap().mask = Some(LayerMask::solid(revealing));
        }
        self.is_mask_selected = true;
        self.end_edit();
    }

    pub fn toggle_layer_mask(&mut self) {
        let Some(id) = self.active_layer_id else {
            return;
        };
        let enabled = self
            .active_layer()
            .and_then(|l| l.mask.as_ref())
            .map(|m| m.is_enabled);
        if let Some(enabled) = enabled {
            self.finish_opacity_edit();
            self.begin_edit(if enabled { "Disable Layer Mask" } else { "Enable Layer Mask" });
            if let Some(doc) = self.document.as_mut() {
                doc.layer_mut(id).unwrap().mask.as_mut().unwrap().is_enabled = !enabled;
            }
            self.end_edit();
        }
    }

    pub fn delete_layer_mask(&mut self) {
        let Some(id) = self.active_layer_id else {
            return;
        };
        if self.active_layer().map(|l| l.mask.is_some()).unwrap_or(false) {
            self.finish_opacity_edit();
            self.begin_edit("Delete Layer Mask");
            if let Some(doc) = self.document.as_mut() {
                doc.layer_mut(id).unwrap().mask = None;
            }
            self.is_mask_selected = false;
            self.end_edit();
        }
    }

    // MARK: Transforms

    pub fn commit_transform(&mut self) {
        self.finish_opacity_edit();
        let Some(edit) = self.transform_edit.take() else {
            return;
        };
        let Some(document) = self.document.as_mut() else {
            return;
        };
        let Some(index) = document.layer_index(edit.layer_id) else {
            return;
        };
        if !edit.draft.is_valid() {
            return;
        }
        self.begin_edit("Transform Layer");
        let document = self.document.as_mut().unwrap();
        let layer = &mut document.layers[index];
        if let Some(mask) = layer.mask.as_mut() {
            let old = layer.transform;
            mask.placement = mask.placement_moving_layer(&old, &edit.draft);
        }
        layer.transform = edit.draft;
        self.end_edit();
    }

    pub fn cancel_transform(&mut self) {
        self.transform_edit = None;
    }

    pub fn begin_transform(&mut self) {
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
        let mask_alone = self
            .is_mask_selected
            && layer.mask.as_ref().map(|m| !m.is_linked).unwrap_or(false);
        let draft = if mask_alone {
            layer.mask_transform()
        } else {
            layer.transform
        };
        self.transform_edit = Some(TransformEdit {
            layer_id: layer.id,
            draft,
            persistent: true,
            floating: None,
            corners: None,
            mask: mask_alone,
            group: None,
        });
    }

    pub fn preview_transform(&mut self, value: LayerTransform) {
        if value.is_valid() {
            if let Some(edit) = self.transform_edit.as_mut() {
                edit.draft = value;
            }
        }
    }

    pub fn can_transform(&self) -> bool {
        let Some(layer) = self.active_layer() else {
            return false;
        };
        layer.asset.is_some() && !layer.is_group
    }

    pub fn nudge_layer(&mut self, dx: f64, dy: f64) {
        let already = self.transform_edit.is_some();
        if !already {
            self.begin_transform();
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
        let Some(id) = self.active_layer_id else {
            return;
        };
        let axis = self
            .active_layer()
            .map(|l| if horizontally { l.transform.center().x } else { l.transform.center().y });
        let Some(axis) = axis else {
            return;
        };
        self.finish_opacity_edit();
        self.begin_edit(if horizontally { "Flip Horizontal" } else { "Flip Vertical" });
        if let Some(doc) = self.document.as_mut() {
            if let Some(layer) = doc.layer_mut(id) {
                let flipped = mirrored_transform(&layer.transform, horizontally, axis);
                if let Some(mask) = layer.mask.as_mut() {
                    let old = layer.transform;
                    mask.placement = mask.placement_moving_layer(&old, &flipped);
                }
                layer.transform = flipped;
            }
        }
        self.end_edit();
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
                if let Some(mask) = layer.mask.as_mut() {
                    if let Some(placement) = mask.placement.as_mut() {
                        *placement = mirrored_transform(placement, horizontally, axis);
                    }
                }
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
    pub fn merge_layers(&mut self) {
        self.commit_transform();
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(active) = self.active_layer().cloned() else {
            return;
        };
        let (ids, action, name): (Vec<Uuid>, &'static str, String) =
            if self.selected_layer_ids.len() > 1 {
                let mut picked: Vec<Uuid> = Vec::new();
                for id in &self.selected_layer_ids {
                    picked.extend(document.descendant_ids(*id));
                }
                let ordered: Vec<Uuid> = document
                    .layers
                    .iter()
                    .filter(|l| picked.contains(&l.id))
                    .map(|l| l.id)
                    .collect();
                let name = document
                    .layers
                    .iter()
                    .rev()
                    .find(|l| self.selected_layer_ids.contains(&l.id))
                    .map(|l| l.name.clone())
                    .unwrap_or_default();
                (ordered, "Merge Layers", name)
            } else if active.is_group {
                let inside = document.descendant_ids(active.id);
                let mut ids: Vec<Uuid> = document
                    .layers
                    .iter()
                    .filter(|l| inside.contains(&l.id))
                    .map(|l| l.id)
                    .collect();
                ids.push(active.id);
                let name = active.name.clone();
                (ids, "Merge Group", name)
            } else {
                let Some(index) = document.layer_index(active.id) else {
                    return;
                };
                let below = document.layers[..index]
                    .iter()
                    .rev()
                    .find(|l| l.parent_id == active.parent_id && !l.is_group)
                    .cloned();
                let Some(below) = below else {
                    return;
                };
                (vec![below.id, active.id], "Merge Down", below.name.clone())
            };
        let kept: std::collections::HashSet<Uuid> = ids.iter().copied().collect();

        // Composite the subset onto a canvas.
        let mut subset_doc = document.clone();
        subset_doc.layers = document
            .layers
            .iter()
            .filter(|l| kept.contains(&l.id))
            .cloned()
            .collect();
        for layer in subset_doc.layers.iter_mut() {
            if let Some(parent) = layer.parent_id {
                if !kept.contains(&parent) {
                    layer.parent_id = None;
                }
            }
            if let Some(source) = layer.mask_source_id {
                if !kept.contains(&source) {
                    layer.mask_source_id = None;
                }
            }
        }
        let full = crate::render::draw_live_composite(&subset_doc);
        let canvas_transform = LayerTransform::new(
            CGPoint::ZERO,
            CGSize::new(document.width as f64, document.height as f64),
        );
        let (trimmed_image, trimmed_transform) = crate::filters::trimmed(&full, canvas_transform);
        let anchor = *ids.last().unwrap_or(&active.id);
        let parent = document.layer(anchor).and_then(|l| l.parent_id);
        let mut merged = ImageLayer::with_asset(
            ImportedImage::new(trimmed_image, name.clone()),
            trimmed_transform.origin,
        );
        merged.transform = trimmed_transform;
        merged.parent_id = parent;

        let mut next: Vec<ImageLayer> = document
            .layers
            .iter()
            .filter(|l| !kept.contains(&l.id))
            .cloned()
            .collect();
        for layer in next.iter_mut() {
            if let Some(source) = layer.mask_source_id {
                if kept.contains(&source) {
                    layer.mask_source_id = Some(merged.id);
                }
            }
        }
        let slot = document.layer_index(anchor).unwrap_or(document.layers.len());
        let insertion =
            slot - document.layers[..slot].iter().filter(|l| kept.contains(&l.id)).count();
        let insertion = insertion.min(next.len());
        let merged_id = merged.id;
        next.insert(insertion, merged);

        self.begin_edit(action);
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

    // MARK: Crop

    pub fn cancel_crop(&mut self) {
        self.crop_rect = None;
        self.crop_drag = None;
    }

    pub fn begin_crop_drag(&mut self, start: CGPoint, mode: CropDragMode) {
        let original = self.crop_rect.unwrap_or_else(|| {
            CGRect::from_origin_size(
                CGPoint::ZERO,
                self.document_size().unwrap_or(CGSize::ZERO),
            )
        });
        self.crop_drag = Some(CropDrag {
            start,
            original,
            mode,
        });
        if self.crop_rect.is_none() {
            self.crop_rect = Some(original);
        }
    }

    pub fn drag_crop(&mut self, point: CGPoint, ratio: Option<f64>, symmetric: bool) {
        let Some(drag) = &self.crop_drag else {
            return;
        };
        let updated = drag.updated(point, ratio, symmetric);
        self.crop_rect = Some(updated);
    }

    pub fn end_crop_drag(&mut self) {
        self.crop_drag = None;
        if let Some(rect) = self.crop_rect {
            if !CropGeometry::valid(rect) {
                self.crop_rect = None;
            }
        }
    }

    pub fn crop_ratio(&self) -> Option<f64> {
        match self.crop_ratio_choice.as_str() {
            "Original" => self
                .document
                .as_ref()
                .map(|d| d.width as f64 / d.height as f64),
            "1:1" => Some(1.0),
            "4:3" => Some(4.0 / 3.0),
            "16:9" => Some(16.0 / 9.0),
            _ => None,
        }
    }

    pub fn commit_crop(&mut self) {
        let Some(rect) = self.crop_rect else {
            return;
        };
        if !CropGeometry::valid(rect) {
            return;
        }
        let mut options = CanvasSizeOptions::new(rect.width() as usize, rect.height() as usize);
        options.content_offset = Some(CGPoint::new(-rect.min_x(), -rect.min_y()));
        self.begin_edit("Crop");
        if let Some(doc) = self.document.take() {
            self.document = Some(resize_canvas(&doc, &options));
        }
        self.end_edit();
        self.crop_rect = None;
    }

    // MARK: Canvas / Image size, trim

    pub fn resize_canvas_to(&mut self, options: &CanvasSizeOptions) {
        self.begin_edit("Canvas Size");
        if let Some(doc) = self.document.take() {
            self.document = Some(resize_canvas(&doc, options));
        }
        self.end_edit();
    }

    pub fn resize_image_to(&mut self, width: usize, height: usize, resolution: Option<f64>) {
        if !(1..=30_000).contains(&width) || !(1..=30_000).contains(&height) {
            return;
        }
        self.begin_edit("Image Size");
        if let Some(doc) = self.document.take() {
            self.document = Some(resize_image(&doc, width, height, resolution));
        }
        self.end_edit();
    }

    /// Returns false when nothing remained to trim.
    pub fn trim(&mut self, options: &TrimOptions) -> bool {
        if let Some(doc) = self.document.clone() {
            if let Some(trimmed) = trim_document(&doc, options) {
                self.begin_edit("Trim");
                self.document = Some(trimmed);
                self.end_edit();
                return true;
            }
        }
        false
    }

    // MARK: Fills & palette

    pub fn palette_color(&self, background: bool) -> PaletteColor {
        if self.is_mask_selected {
            let white = if background {
                !self.mask_paint_white
            } else {
                self.mask_paint_white
            };
            if white {
                PaletteColor::WHITE
            } else {
                PaletteColor::BLACK
            }
        } else if background {
            self.background_color
        } else {
            PaletteColor::new(
                self.brush_settings.red,
                self.brush_settings.green,
                self.brush_settings.blue,
            )
        }
    }

    pub fn swap_palette_colors(&mut self) {
        if self.is_mask_selected {
            self.mask_paint_white = !self.mask_paint_white;
        } else {
            let old = self.palette_color(false);
            let background = self.background_color;
            self.brush_settings.red = background.red;
            self.brush_settings.green = background.green;
            self.brush_settings.blue = background.blue;
            self.background_color = old;
        }
    }

    pub fn reset_palette_colors(&mut self) {
        if self.is_mask_selected {
            self.mask_paint_white = false;
        } else {
            self.brush_settings.red = 0.0;
            self.brush_settings.green = 0.0;
            self.brush_settings.blue = 0.0;
            self.background_color = PaletteColor::WHITE;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        s.resize_image_to(75, 75, Some(144.0));
        assert_eq!(s.document.as_ref().unwrap().width, 75);
        assert_eq!(s.document.as_ref().unwrap().resolution, 144.0);
        assert_eq!(s.history.undo_name(), "Image Size");
    }

    #[test]
    fn trim_reports_nothing_to_trim() {
        let mut s = session();
        // A fully transparent layer: nothing remains to trim.
        let transparent = Bitmap::new(100, 100);
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(transparent, "L"));
        assert!(!s.trim(&TrimOptions::default()));
        // Content in one corner: trim crops to it.
        let mut image = Bitmap::new(100, 100);
        for y in 0..20 {
            for x in 0..20 {
                image.set_pixel(x, y, [255, 0, 0, 255]);
            }
        }
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(image, "L"));
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
}
