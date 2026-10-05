//! Port of `Compositor/Document/DocumentHistory.swift`. Value snapshots share
//! immutable images through `Arc`, so layer edits copy no pixels.

use std::collections::HashSet;

use uuid::Uuid;

use crate::document::CanvasDocument;

/// A document snapshot: the whole document value (images shared), the active
/// layer, and the revision it was taken at.
#[derive(Clone)]
pub struct Snapshot {
    pub document: Option<CanvasDocument>,
    pub active_layer_id: Option<Uuid>,
    pub revision: Uuid,
}

struct Entry {
    name: String,
    before: Snapshot,
    after: Snapshot,
}

/// Undo/redo with an entry limit and a retained-byte limit.
pub struct DocumentHistory {
    past: Vec<Entry>,
    future: Vec<Entry>,
    revision: Uuid,
    saved_revision: Option<Uuid>,
    pending: Option<Snapshot>,
    pending_name: String,
    depth: usize,
    pub entry_limit: usize,
    pub retained_byte_limit: usize,
}

impl Default for DocumentHistory {
    fn default() -> Self {
        DocumentHistory::new(100, 256 * 1024 * 1024)
    }
}

impl DocumentHistory {
    pub fn new(entry_limit: usize, retained_byte_limit: usize) -> Self {
        let revision = Uuid::new_v4();
        DocumentHistory {
            past: Vec::new(),
            future: Vec::new(),
            saved_revision: Some(revision),
            revision,
            pending: None,
            pending_name: "Edit".to_string(),
            depth: 0,
            entry_limit,
            retained_byte_limit,
        }
    }

    pub fn can_undo(&self) -> bool {
        self.depth == 0 && !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        self.depth == 0 && !self.future.is_empty()
    }

    pub fn undo_name(&self) -> &str {
        self.past.last().map(|e| e.name.as_str()).unwrap_or("")
    }

    pub fn redo_name(&self) -> &str {
        self.future.last().map(|e| e.name.as_str()).unwrap_or("")
    }

    pub fn is_modified(&self) -> bool {
        self.revision != self.saved_revision.unwrap()
    }

    pub fn undo_count(&self) -> usize {
        self.past.len()
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = Some(self.revision);
    }

    pub fn reset(&mut self) {
        self.past.clear();
        self.future.clear();
        self.pending = None;
        self.depth = 0;
        self.revision = Uuid::new_v4();
        self.saved_revision = Some(self.revision);
    }

    /// Nestable transaction boundary.
    pub fn begin(
        &mut self,
        name: &str,
        document: &Option<CanvasDocument>,
        selection: Option<Uuid>,
    ) {
        if self.depth == 0 {
            self.pending = Some(Snapshot {
                document: document.clone(),
                active_layer_id: selection,
                revision: self.revision,
            });
            self.pending_name = name.to_string();
        }
        self.depth += 1;
    }

    pub fn end(&mut self, document: &Option<CanvasDocument>, selection: Option<Uuid>) {
        if self.depth == 0 {
            return;
        }
        self.depth -= 1;
        if self.depth != 0 {
            return;
        }
        let before = match self.pending.take() {
            Some(b) => b,
            None => return,
        };
        // Selecting, navigating, and no-op edits must preserve redo history.
        if before.document == *document {
            return;
        }
        self.revision = Uuid::new_v4();
        self.past.push(Entry {
            name: std::mem::take(&mut self.pending_name),
            after: Snapshot {
                document: document.clone(),
                active_layer_id: selection,
                revision: self.revision,
            },
            before,
        });
        self.future.clear();
        let current = document.clone();
        self.trim(current.as_ref());
    }

    pub fn undo(&mut self) -> Option<Snapshot> {
        if !self.can_undo() {
            return None;
        }
        let entry = self.past.pop()?;
        self.future.push(entry.clone_for_push());
        self.revision = entry.before.revision;
        let current = entry.before.document.clone();
        self.trim(current.as_ref());
        Some(entry.before)
    }

    pub fn redo(&mut self) -> Option<Snapshot> {
        if !self.can_redo() {
            return None;
        }
        let entry = self.future.pop()?;
        self.past.push(entry.clone_for_push());
        self.revision = entry.after.revision;
        let current = entry.after.document.clone();
        self.trim(current.as_ref());
        Some(entry.after)
    }

    /// Bytes retained only by history, excluding images in the live document.
    /// Layer bitmaps and masks are RGBA (4 bytes per pixel) in this port —
    /// unlike Swift, whose masks are DeviceGray at one byte per pixel
    /// (DocumentHistory.retainedBytes). Images compare by identity.
    pub fn retained_bytes(&self, current: Option<&CanvasDocument>) -> usize {
        let mut seen: HashSet<ArcPtr> = HashSet::new();
        if let Some(document) = current {
            for layer in &document.layers {
                for asset in layer
                    .asset
                    .iter()
                    .chain(layer.mask.as_ref().map(|m| &m.asset))
                {
                    seen.insert(arc_ptr(&asset.image));
                    seen.insert(arc_ptr(&asset.thumbnail));
                }
            }
        }
        let mut bytes = 0usize;
        for entry in self.past.iter().chain(self.future.iter()) {
            for snapshot in [&entry.before, &entry.after] {
                let Some(document) = &snapshot.document else {
                    continue;
                };
                for layer in &document.layers {
                    for image in layer.asset.iter().flat_map(|a| [&a.image, &a.thumbnail]) {
                        if seen.insert(arc_ptr(image)) {
                            bytes += image.width * image.height * 4;
                        }
                    }
                    if let Some(mask) = layer.mask.as_ref() {
                        for image in [&mask.asset.image, &mask.asset.thumbnail] {
                            if seen.insert(arc_ptr(image)) {
                                // RGBA here, 4x the Swift DeviceGray estimate.
                                bytes += image.width * image.height * 4;
                            }
                        }
                    }
                }
            }
        }
        bytes
    }

    fn trim(&mut self, current: Option<&CanvasDocument>) {
        while self.past.len() + self.future.len() > self.entry_limit
            || self.retained_bytes(current) > self.retained_byte_limit
        {
            if !self.past.is_empty() {
                self.past.remove(0);
            } else if !self.future.is_empty() {
                self.future.remove(0);
            } else {
                break;
            }
        }
    }
}

trait EntryClone {
    fn clone_for_push(&self) -> Self;
}

impl EntryClone for Entry {
    fn clone_for_push(&self) -> Self {
        Entry {
            name: self.name.clone(),
            before: self.before.clone(),
            after: self.after.clone(),
        }
    }
}

/// Helper to take data pointers of Arc contents for identity checks.
type ArcPtr = *const u8;

fn arc_ptr(image: &std::sync::Arc<crate::raster::Bitmap>) -> ArcPtr {
    std::sync::Arc::as_ptr(image) as *const u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{CGPoint, CGSize};
    use crate::raster::Bitmap;
    use crate::transform::LayerTransform;

    fn doc_with_color(r: u8) -> CanvasDocument {
        let mut doc = CanvasDocument::new(10, 10);
        let mut img = Bitmap::new(10, 10);
        for i in (0..img.data.len()).step_by(4) {
            img.data[i] = r;
            img.data[i + 3] = 255;
        }
        let mut layer = crate::document::ImageLayer::blank("L", CGSize::new(10.0, 10.0));
        layer.asset = Some(crate::raster::ImportedImage::new(img, "L"));
        doc.layers = vec![layer];
        doc
    }

    fn bump(doc: &CanvasDocument) -> CanvasDocument {
        let mut next = doc.clone();
        if let Some(layer) = next.layers.first_mut() {
            let mut img = layer.asset.as_ref().unwrap().image.as_ref().clone();
            for i in (0..img.data.len()).step_by(4) {
                img.data[i] = img.data[i].wrapping_add(1);
            }
            layer.asset = Some(crate::raster::ImportedImage::new(img, "L"));
        }
        next
    }

    #[test]
    fn masks_retain_four_bytes_per_pixel() {
        // Unlike Swift's DeviceGray masks, this port stores masks as RGBA
        // bitmaps (document.rs LayerMask::solid builds 4-channel pixels), so
        // a 20×30 mask costs four bytes per pixel — the estimate must match
        // reality or the retained-byte limit under-counts masks 4x.
        let mut history = DocumentHistory::default();
        let plain = CanvasDocument::new(10, 10);
        let mut masked = CanvasDocument::new(10, 10);
        let mut layer = crate::document::ImageLayer::blank("L", CGSize::new(10.0, 10.0));
        let mut mask_image = crate::raster::Bitmap::new(20, 30);
        mask_image.data.fill(255);
        let mut mask = crate::document::LayerMask {
            asset: crate::raster::ImportedImage::new(mask_image, "M"),
            is_enabled: true,
            placement: None,
            is_linked: true,
        };
        mask.asset.thumbnail = std::sync::Arc::new(crate::raster::Bitmap::new(20, 30));
        layer.mask = Some(mask);
        masked.layers = vec![layer];
        history.begin("Mask", &Some(plain.clone()), None);
        history.end(&Some(masked.clone()), None);
        // before contributes nothing (no mask); after contributes the RGBA
        // image and its thumbnail at four bytes per pixel.
        let expected = 20 * 30 * 4 + 20 * 30 * 4;
        assert!(
            (history.retained_bytes(None) as i64 - expected as i64).abs() <= 2,
            "retained={} expected≈{expected}",
            history.retained_bytes(None)
        );
    }

    #[test]
    fn identical_pixel_data_in_a_new_image_still_records() {
        // Swift compares images by identity: the same bytes in a new image
        // are a change and keep the entry.
        let mut history = DocumentHistory::default();
        let a = doc_with_color(0);
        let mut current = a.clone();
        history.begin("Paint", &Some(a.clone()), None);
        // "bump" rebuilds the bitmap into a new image even when the bytes
        // happen to match.
        current = bump(&current);
        history.end(&Some(current.clone()), None);
        assert!(history.can_undo());
    }

    #[test]
    fn identity_clones_do_not_record() {
        // Snapshots share Arcs: a cloned document equals the original and
        // the entry is dropped.
        let mut history = DocumentHistory::default();
        let a = doc_with_color(0);
        history.begin("Paint", &Some(a.clone()), None);
        history.end(&Some(a.clone()), None);
        assert!(!history.can_undo());
    }

    #[test]
    fn begin_end_pushes_entry() {
        let mut history = DocumentHistory::default();
        let a = doc_with_color(0);
        let mut current = a.clone();
        history.begin("Paint", &Some(a.clone()), None);
        current = bump(&current);
        history.end(&Some(current.clone()), None);
        assert!(history.can_undo());
        assert!(!history.can_redo());
        assert_eq!(history.undo_name(), "Paint");
        assert!(history.is_modified());
    }

    #[test]
    fn no_op_end_preserves_history() {
        let mut history = DocumentHistory::default();
        let a = doc_with_color(0);
        history.begin("Select", &Some(a.clone()), None);
        history.end(&Some(a.clone()), None);
        assert!(!history.can_undo());
    }

    #[test]
    fn undo_redo_roundtrip() {
        let mut history = DocumentHistory::default();
        let a = doc_with_color(0);
        let mut current = a.clone();
        history.begin("Paint", &Some(a.clone()), None);
        current = bump(&current);
        history.end(&Some(current.clone()), None);

        let snapshot = history.undo().unwrap();
        assert_eq!(
            snapshot.document.as_ref().unwrap().layers[0]
                .asset
                .as_ref()
                .unwrap()
                .image
                .data[0],
            0
        );
        assert!(history.can_redo());
        assert_eq!(history.redo_name(), "Paint");

        let snapshot = history.redo().unwrap();
        assert_eq!(
            snapshot.document.as_ref().unwrap().layers[0]
                .asset
                .as_ref()
                .unwrap()
                .image
                .data[0],
            1
        );
        assert!(history.can_undo());
        assert!(!history.can_redo());
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut history = DocumentHistory::default();
        let a = doc_with_color(0);
        let mut current = a.clone();
        history.begin("One", &Some(a.clone()), None);
        current = bump(&current);
        history.end(&Some(current.clone()), None);
        history.undo();

        let base = doc_with_color(0);
        let mut current = base.clone();
        history.begin("Two", &Some(base.clone()), None);
        current = bump(&current);
        current = bump(&current);
        history.end(&Some(current.clone()), None);
        assert!(!history.can_redo());
        assert_eq!(history.undo_name(), "Two");
    }

    #[test]
    fn nested_transactions_fold_into_one() {
        let mut history = DocumentHistory::default();
        let a = doc_with_color(0);
        let mut current = a.clone();
        history.begin("Outer", &Some(a.clone()), None);
        current = bump(&current);
        history.begin("Inner", &Some(current.clone()), None);
        current = bump(&current);
        history.end(&Some(current.clone()), None); // inner closes
        history.end(&Some(current.clone()), None); // outer closes
        assert_eq!(history.undo_count(), 1);
        assert_eq!(history.undo_name(), "Outer");
    }

    #[test]
    fn entry_limit_trims_oldest() {
        let mut history = DocumentHistory::new(2, 1024 * 1024 * 1024);
        let mut current = doc_with_color(0);
        for i in 0..5 {
            let base = current.clone();
            history.begin(&format!("Edit {i}"), &Some(base.clone()), None);
            current = bump(&current);
            history.end(&Some(current.clone()), None);
        }
        assert_eq!(history.undo_count(), 2);
        assert_eq!(history.undo_name(), "Edit 4");
        // Oldest two are gone: undoing twice reaches the third state, no further.
        history.undo();
        history.undo();
        assert!(!history.can_undo());
    }

    #[test]
    fn mark_saved_and_reset() {
        let mut history = DocumentHistory::default();
        let a = doc_with_color(0);
        let mut current = a.clone();
        history.begin("Paint", &Some(a.clone()), None);
        current = bump(&current);
        history.end(&Some(current.clone()), None);
        assert!(history.is_modified());
        history.mark_saved();
        assert!(!history.is_modified());
        history.undo();
        assert!(history.is_modified());
        history.reset();
        assert!(!history.can_undo() && !history.can_redo());
        assert!(!history.is_modified());
    }

    #[test]
    fn save_revision_tracks_the_modified_flag() {
        // mark_saved pins the revision: edits set the modified flag and
        // undoing back to the saved revision clears it again
        // (HistoryTests.navigationNoOpsAndSaveRevisionPreserveHistory).
        let mut history = DocumentHistory::default();
        let a = CanvasDocument::new(10, 10);
        history.begin("Paint", &Some(a.clone()), None);
        let mut current = a.clone();
        current.resolution = 144.0;
        history.end(&Some(current), None);
        assert!(history.is_modified());
        history.mark_saved();
        assert!(!history.is_modified());
        let mut changed = a.clone();
        changed.resolution = 300.0;
        history.begin("Paint", &Some(a.clone()), None);
        history.end(&Some(changed), None);
        assert!(history.is_modified());
        // Undoing past the edit returns to the saved revision.
        history.undo();
        assert!(!history.is_modified());
        assert!(history.can_redo());
        history.redo();
        assert!(history.is_modified());
    }
}
