//! Port of the session's `DocumentHistory` integration: undo/redo, edit
//! boundaries and the modified flag.

use super::*;

impl EditorSession {
    // MARK: Undo

    // `pub(crate)`: `project::copy_layers_between` runs one whole copy as a
    // single undoable edit, exactly as `ProjectWorkspace.copyLayer` does.
    pub(crate) fn begin_edit(&mut self, name: &str) {
        self.history
            .begin(name, &self.document, self.active_layer_id);
    }

    pub(crate) fn end_edit(&mut self) {
        self.history.end(&self.document, self.active_layer_id);
    }

    /// `canUseHistory` (EditorSession.swift:563-567), condition for
    /// condition: no editor may swap the document out under a busy gesture.
    /// The busy flags moved core-side (stage-D ticket 34) — the
    /// `!isProjectBusy && !isImporting && brushStroke == nil &&
    /// warpStroke == nil` clauses are the folded `has_brush_stroke` — and
    /// the last four flags with stage-D ticket 35. The core additionally
    /// guards `pixel_move` — lifted pixels must not land in a restored
    /// document.
    pub fn can_use_history(&self) -> bool {
        self.selection_amount_operation.is_none()
            && self.text_draft.is_none()
            && !self.is_project_busy
            && !self.is_importing
            && !self.has_brush_stroke
            && self.levels.is_none()
            && !self.shows_new_document
            && !self.shows_importer
            && self.renaming_layer_id.is_none()
            && self.import_error.is_none()
            && self.transform_edit.is_none()
            && !self.shows_conversion_sheet
            && self.pixel_move.is_none()
    }

    pub fn can_undo(&self) -> bool {
        // A pending gradient can still be applied or cancelled, so the menu
        // treats it as undoable too (EditorSession.swift:567's
        // `gradientEdit != nil` branch).
        self.can_use_history() && (self.history.can_undo() || self.gradient_edit.is_some())
    }

    pub fn can_redo(&self) -> bool {
        self.can_use_history() && self.history.can_redo()
    }

    pub fn undo(&mut self) {
        // Like Photoshop, the first Undo discards a pending gradient
        // (EditorSession.swift:569-571).
        if self.gradient_edit.is_some() {
            self.cancel_gradient();
            return;
        }
        if !self.can_undo() {
            return;
        }
        if let Some(snapshot) = self.history.undo() {
            self.restore(snapshot);
        }
    }

    pub fn redo(&mut self) {
        if !self.can_redo() {
            return;
        }
        if let Some(snapshot) = self.history.redo() {
            self.restore(snapshot);
        }
    }

    /// `restore(_:)` (EditorSession.swift:578-586): a pending crop and a
    /// pending gradient die with the undo, the snapshot swaps in, and the mask
    /// stays the edit target only when the active layer did not change and the
    /// restored layer still has a mask.
    fn restore(&mut self, snapshot: crate::history::Snapshot) {
        self.cancel_crop();
        self.cancel_gradient();
        // The mask target survives only an undo that keeps editing the same
        // layer (`keepMaskTarget`).
        let keep_mask_target =
            self.is_mask_selected && self.active_layer_id == snapshot.active_layer_id;
        self.document = snapshot.document;
        self.active_layer_id = snapshot.active_layer_id;
        if let Some(id) = self.active_layer_id {
            self.selected_layer_ids = vec![id];
        }
        self.is_mask_selected = keep_mask_target
            && self
                .active_layer()
                .map(|l| l.mask.is_some())
                .unwrap_or(false);
        // Swift then refits the viewport when the canvas itself changed
        // (`viewport.fit(documentSize:)`); the core session has no viewport,
        // so that stays with the app.
    }

    pub fn is_modified(&self) -> bool {
        self.history.is_modified()
    }

    pub fn mark_saved(&mut self) {
        self.history.mark_saved();
    }
}
