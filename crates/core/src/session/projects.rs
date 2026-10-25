//! Port of the project lifecycle: `EditorSession+Projects.swift`'s
//! `createNewProject`/`installProject`/`clearProject` plus the project-
//! operation guards (`canStartProjectOperation` and the busy hold that
//! ProjectController's flows take around save/open/export).

use crate::document::{CanvasDocument, ImageLayer};

use super::EditorSession;

impl EditorSession {
    /// `createDocument` — File > New. `isProjectBusy`/`isImporting` join
    /// the size guard (EditorSession.swift:893-895).
    pub fn create_document(&mut self, width: usize, height: usize, empty_layer: bool) {
        if self.is_project_busy
            || self.is_importing
            || !(1..=30_000).contains(&width)
            || !(1..=30_000).contains(&height)
        {
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
        // `viewport.fit(documentSize: document.size)` (EditorSession.swift:904).
        if let Some(size) = self.document_size() {
            self.viewport.fit(size);
        }
        self.shows_new_document = false;
    }

    /// `EditorSession.installProject(_:from:)`
    /// (Document/EditorSession+Projects.swift:20-40). Called only after the
    /// whole package has validated and loaded; replaces the document and all
    /// project state in one go. The live session is untouched on error.
    pub fn install_project(
        &mut self,
        data: &crate::project::ProjectData,
        url: &std::path::Path,
    ) -> Result<(), crate::project::ProjectError> {
        // `document_from_project` revalidates, so a hand-built `ProjectData`
        // cannot smuggle in damaged metadata.
        let (document, active_layer_id) = crate::project::document_from_project(data)?;
        self.collapsed_group_ids.clear();
        self.is_mask_selected = false;
        self.cancel_crop();
        // A pending gradient recorded against the old document must not leak
        // into the freshly installed one (apply_gradient commits by draft
        // layer id; dropping the draft keeps that safe).
        self.gradient_edit = None;
        self.guide_drag = None;
        self.transform_edit = None;
        self.active_layer_id = active_layer_id;
        self.selected_layer_ids = active_layer_id.map(|id| vec![id]).unwrap_or_default();
        self.document = Some(document);
        self.project_url = Some(url.to_path_buf());
        self.history.reset();
        // `viewport.fit(documentSize: document!.size)` — installProject's
        // last line (EditorSession+Projects.swift:39).
        if let Some(size) = self.document_size() {
            self.viewport.fit(size);
        }
        Ok(())
    }

    /// `EditorSession.clearProject` (Document/EditorSession+Projects.swift:42-53):
    /// File > New / close without a workspace. Everything the open project
    /// owned is dropped and the session is left as a fresh, empty canvas.
    pub fn clear_project(&mut self) {
        self.collapsed_group_ids.clear();
        self.is_mask_selected = false;
        self.cancel_crop();
        self.gradient_edit = None;
        self.transform_edit = None;
        self.guide_drag = None;
        self.document = None;
        self.active_layer_id = None;
        self.selected_layer_ids = Vec::new();
        self.project_url = None;
        self.history.reset();
    }

    /// `canStartProjectOperation` (EditorSession.swift:150-152), condition
    /// for condition — complete since stage-D ticket 35 moved the last four
    /// flags (`showsImporter`, `renamingLayerID`, `importError`,
    /// `showsConversionSheet`) in.
    pub fn can_start_project_operation(&self) -> bool {
        self.selection_amount_operation.is_none() // selectionAmountOperation == nil
            && self.text_draft.is_none() // textDraft == nil
            && !self.is_project_busy // !isProjectBusy
            && !self.is_importing // !isImporting
            && !self.has_brush_stroke // brushStroke == nil && warpStroke == nil
            && self.levels.is_none() // levels == nil
            && !self.shows_new_document // !showsNewDocument
            && !self.shows_importer // !showsImporter
            && self.renaming_layer_id.is_none() // renamingLayerID == nil
            && self.import_error.is_none() // importError == nil
            && self.adjustment_editing_id.is_none() // adjustmentEditingID == nil
            && !self.shows_conversion_sheet // !showsConversionSheet
    }

    /// `ProjectController.begin` (ProjectController.swift:16-22), moved onto
    /// the session with the busy flag it guards: refuse unless
    /// `can_start_project_operation`, then cancel a pending crop, commit a
    /// pending transform, and hold `is_project_busy` until the matching
    /// [`EditorSession::end_project_access`] (Swift's per-flow `defer` /
    /// the app's two-phase `finish_*`).
    pub fn begin_project_access(&mut self) -> bool {
        if !self.can_start_project_operation() {
            return false;
        }
        self.cancel_crop();
        self.commit_transform();
        self.is_project_busy = true;
        true
    }

    /// Releases the project-busy hold taken by
    /// [`EditorSession::begin_project_access`].
    pub fn end_project_access(&mut self) {
        self.is_project_busy = false;
    }
}
