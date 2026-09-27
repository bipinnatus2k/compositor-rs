//! Port of `ProjectWorkspace.copyLayer(_:into:at:)`'s model half
//! (Document/ProjectWorkspace.swift:157-209): the tab guards, dialogs and
//! drag-and-drop plumbing stay in the app layer; everything that manipulates
//! two sessions is here so it can be tested without a window.

use std::collections::HashMap;

use uuid::Uuid;

use crate::document::ImageLayer;
use crate::geometry::CGPoint;
use crate::raster::ImportedImage;
use crate::session::EditorSession;

/// The per-project pixel budget (`ProjectWorkspace.swift:172`): the pixels
/// already used by the target plus the pixels being copied may not exceed
/// 100 megapixels. Only layer pixel assets count — masks, being derived
/// pixels, are not budgeted.
const PIXEL_BUDGET: usize = 100_000_000;

/// `ProjectWorkspace.copyLayer`'s core: copies `root_id` and its descendants
/// from `source` into `target` as one undoable edit named
/// "Copy Layers from Project".
///
/// `at` is the document point the root layer's center lands on; None centers
/// the copy on the target document. The caller is responsible for the tab
/// guards (`canSwitch`, `canEditLayers`, the source-is-not-the-target check).
pub fn copy_layers_between(
    source: &EditorSession,
    target: &mut EditorSession,
    root_id: Uuid,
    at: Option<CGPoint>,
) -> Result<(), String> {
    let Some(source_document) = source.document.as_ref() else {
        return Err("There is no project to copy layers from.".to_string());
    };
    if source_document.layer(root_id).is_none() {
        return Err("The layer to copy is missing.".to_string());
    }
    // `included = descendantIDs(of:).union([id])` (:168); Rust's
    // `descendant_ids` includes the root itself.
    let included = source_document.descendant_ids(root_id);
    let mut copied: Vec<ImageLayer> = source_document
        .layers
        .iter()
        .filter(|layer| included.contains(&layer.id))
        .cloned()
        .collect();

    // The budget (:170-172), counted from the source assets before any bake.
    // Adjustment layers and blank layers have no pixels and count nothing.
    let pixels = |layer: &ImageLayer| {
        layer
            .asset
            .as_ref()
            .map(|asset| asset.image.width * asset.image.height)
            .unwrap_or(0)
    };
    let used: usize = target
        .document
        .as_ref()
        .map(|document| document.layers.iter().map(pixels).sum())
        .unwrap_or(0);
    let added: usize = copied.iter().map(pixels).sum();
    if used + added > PIXEL_BUDGET {
        return Err("The copied layers exceed this project’s 100-megapixel limit."
            .to_string());
    }

    // A layer fed by a live mask outside the copied set loses the feed: an
    // adjustment layer just drops the link, a pixel layer bakes the live
    // dependency into its own pixels first (:182-187).
    for layer in &mut copied {
        if !layer
            .mask_source_id
            .map(|id| !included.contains(&id))
            .unwrap_or(false)
        {
            continue;
        }
        if layer.adjustment.is_some() {
            layer.mask_source_id = None;
            continue;
        }
        layer.asset = source.bake_live_mask(layer.id);
        layer.mask_source_id = None;
    }

    // Every copied layer gets a fresh identity; parents and mask sources that
    // stayed inside the set follow the mapping, the rest drop to None (:188,
    // :198-199).
    let mapping: HashMap<Uuid, Uuid> =
        copied.iter().map(|layer| (layer.id, Uuid::new_v4())).collect();

    // `size` (:189): the target document's size when it has one — which is
    // when no document is created below — otherwise the source's.
    let size = target
        .document
        .as_ref()
        .map(|document| document.size())
        .unwrap_or_else(|| source_document.size());
    // `anchor` (:190): the root layer's center, else the source's center.
    let anchor = copied
        .iter()
        .find(|layer| layer.id == root_id)
        .map(|layer| layer.transform.center())
        .unwrap_or(CGPoint {
            x: source_document.size().width / 2.0,
            y: source_document.size().height / 2.0,
        });
    let center = at.unwrap_or(CGPoint {
        x: size.width / 2.0,
        y: size.height / 2.0,
    });
    let offset_x = center.x - anchor.x;
    let offset_y = center.y - anchor.y;

    let layers: Vec<ImageLayer> = copied
        .into_iter()
        .map(|mut layer| {
            layer.id = mapping[&layer.id];
            layer.parent_id = layer.parent_id.and_then(|parent| mapping.get(&parent).copied());
            layer.mask_source_id = layer
                .mask_source_id
                .and_then(|source| mapping.get(&source).copied());
            layer.transform.origin.x += offset_x;
            layer.transform.origin.y += offset_y;
            // Only an explicit placement shifts with the layer; None means the
            // mask covers the layer's own pixel grid and follows the transform.
            if let Some(placement) = layer.mask.as_mut().and_then(|mask| mask.placement.as_mut()) {
                placement.origin.x += offset_x;
                placement.origin.y += offset_y;
            }
            layer
        })
        .collect();

    // One undoable edit (:202-206). The nested "New Canvas" edit folds into it.
    target.begin_edit("Copy Layers from Project");
    if target.document.is_none() {
        target.create_document(size.width as usize, size.height as usize, false);
    }
    if let Some(document) = target.document.as_mut() {
        document.layers.extend(layers);
    }
    target.active_layer_id = mapping.get(&root_id).copied();
    target.selected_layer_ids = target
        .active_layer_id
        .map(|id| vec![id])
        .unwrap_or_default();
    target.end_edit();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{CanvasDocument, LayerMask};
    use crate::raster::Bitmap;
    use crate::transform::LayerTransform;
    use crate::adjustment::{AdjustmentKind, LayerAdjustment};
    use crate::geometry::{CGFloat, CGSize};

    fn session_with_document(width: usize, height: usize) -> EditorSession {
        let mut session = EditorSession::new();
        session.create_document(width, height, false);
        session
    }

    fn solid_asset(width: usize, height: usize, rgba: [u8; 4], name: &str) -> ImportedImage {
        ImportedImage::new(Bitmap::new_clearing(width, height, rgba), name)
    }

    fn pixel_layer(asset: ImportedImage, origin: (CGFloat, CGFloat)) -> ImageLayer {
        ImageLayer::with_asset(
            asset,
            CGPoint {
                x: origin.0,
                y: origin.1,
            },
        )
    }

    fn center_of(session: &EditorSession, id: Uuid) -> CGPoint {
        session.document.as_ref().unwrap().layer(id).unwrap().transform.center()
    }

    fn append_layer(session: &mut EditorSession, layer: ImageLayer) -> Uuid {
        let id = layer.id;
        session.document.as_mut().unwrap().layers.push(layer);
        id
    }

    /// `crossProjectCopyRemapsIdentityAndHasIndependentUndo`
    /// (ProjectWorkspaceTests.swift:58-76): the copy gets a new identity,
    /// lands centered on the target, and the two projects keep separate undo
    /// stacks.
    #[test]
    fn copy_remaps_identity_and_has_independent_undo() {
        let mut source = session_with_document(100, 100);
        source.insert_image(solid_asset(20, 20, [255, 255, 255, 255], "a"), None);
        let original = source.active_layer_id.unwrap();

        let mut target = session_with_document(200, 200);
        copy_layers_between(&source, &mut target, original, None).unwrap();

        let copied = &target.document.as_ref().unwrap().layers[0];
        assert_ne!(copied.id, original);
        assert_eq!(copied.transform.center(), CGPoint { x: 100.0, y: 100.0 });
        assert_eq!(
            target.history.undo_name(),
            "Copy Layers from Project"
        );
        // The source is untouched, document and undo stack alike.
        assert_eq!(source.document.as_ref().unwrap().layers.len(), 1);
        let source_undo_count = source.history.undo_count();
        target.undo();
        assert!(target.document.as_ref().unwrap().layers.is_empty());
        assert_eq!(source.history.undo_count(), source_undo_count);
        target.redo();
        let copied = &target.document.as_ref().unwrap().layers[0];
        assert_eq!(copied.id, target.active_layer_id.unwrap());
    }

    /// The budget (:170-172): target pixels plus copied pixels may reach but
    /// not exceed 100,000,000, with the Swift error string.
    #[test]
    fn budget_rejects_with_the_swift_message() {
        let mut source = session_with_document(100, 100);
        source.insert_image(solid_asset(100, 100, [255, 255, 255, 255], "a"), None);
        let root = source.active_layer_id.unwrap();

        // The shared-asset trick keeps memory small while 10 000 layers of a
        // 100×100 image count exactly 100,000,000 used pixels.
        let shared = solid_asset(100, 100, [0, 0, 0, 255], "bulk");
        let make_target = |count: usize| {
            let mut target = session_with_document(100, 100);
            let document = target.document.as_mut().unwrap();
            for _ in 0..count {
                document.layers.push(pixel_layer(shared.clone(), (0.0, 0.0)));
            }
            target
        };

        // 99,990,000 used + 10,000 added = exactly the budget: allowed.
        let mut target = make_target(9_999);
        copy_layers_between(&source, &mut target, root, None).unwrap();
        assert_eq!(target.document.as_ref().unwrap().layers.len(), 10_000);

        // One layer over: 100,010,000.
        let mut target = make_target(10_000);
        let error = copy_layers_between(&source, &mut target, root, None).unwrap_err();
        assert_eq!(
            error,
            "The copied layers exceed this project’s 100-megapixel limit."
        );
        // Nothing was copied.
        assert_eq!(target.document.as_ref().unwrap().layers.len(), 10_000);
    }

    /// A copied pixel layer fed by a live mask outside the set bakes the feed
    /// into its own pixels and drops the link (:183-187); an adjustment layer
    /// just drops the link.
    #[test]
    fn out_of_set_live_mask_bakes_into_pixels() {
        let mut source = session_with_document(100, 100);
        let feeder = append_layer(
            &mut source,
            pixel_layer(solid_asset(8, 8, [255, 0, 0, 255], "feeder"), (0.0, 0.0)),
        );
        let mut masked = pixel_layer(solid_asset(4, 4, [0, 0, 255, 255], "masked"), (20.0, 20.0));
        masked.mask_source_id = Some(feeder);
        let masked_id = append_layer(&mut source, masked);

        let mut adjustment = ImageLayer::blank("adjust", source.document_size().unwrap());
        adjustment.adjustment = Some(LayerAdjustment::new(AdjustmentKind::Invert));
        adjustment.mask_source_id = Some(feeder);
        let adjustment_id = append_layer(&mut source, adjustment);

        let mut target = session_with_document(100, 100);
        copy_layers_between(&source, &mut target, masked_id, None).unwrap();
        let copied = &target.document.as_ref().unwrap().layers[0];
        assert_eq!(copied.name, "masked");
        assert_eq!(copied.mask_source_id, None);
        // The feeder's red covers the copied layer's whole 4×4 grid.
        let baked = copied.asset.as_ref().unwrap();
        assert!(baked.image.data.chunks_exact(4).all(|p| p == [255, 0, 0, 255]));

        // The adjustment layer keeps its metadata and loses only the link.
        let mut target = session_with_document(100, 100);
        copy_layers_between(&source, &mut target, adjustment_id, None).unwrap();
        let copied = &target.document.as_ref().unwrap().layers[0];
        assert_eq!(copied.mask_source_id, None);
        assert!(copied.adjustment.is_some());
        assert!(copied.asset.is_none());
    }

    /// Fresh identities for every copied layer; parent and mask-source links
    /// inside the set follow the mapping, links out of the set drop to None.
    #[test]
    fn identity_remap_follows_parents_and_mask_sources() {
        let mut source = session_with_document(100, 100);
        let mut group = ImageLayer::blank("Folder", source.document_size().unwrap());
        group.is_group = true;
        let group_id = append_layer(&mut source, group);

        let outside = append_layer(
            &mut source,
            pixel_layer(solid_asset(4, 4, [255, 0, 0, 255], "outside"), (0.0, 0.0)),
        );
        let mut child_a = pixel_layer(solid_asset(4, 4, [0, 255, 0, 255], "a"), (10.0, 10.0));
        child_a.parent_id = Some(group_id);
        child_a.mask_source_id = Some(outside);
        let child_a_id = append_layer(&mut source, child_a);
        let mut child_b = pixel_layer(solid_asset(4, 4, [0, 0, 255, 255], "b"), (20.0, 10.0));
        child_b.parent_id = Some(group_id);
        child_b.mask_source_id = Some(child_a_id);
        let child_b_id = append_layer(&mut source, child_b);

        let mut target = session_with_document(100, 100);
        copy_layers_between(&source, &mut target, group_id, None).unwrap();

        let document = target.document.as_ref().unwrap();
        let copied_group = document.layers.iter().find(|l| l.name == "Folder").unwrap();
        let copied_a = document.layers.iter().find(|l| l.name == "a").unwrap();
        let copied_b = document.layers.iter().find(|l| l.name == "b").unwrap();
        assert_eq!(copied_group.id, target.active_layer_id.unwrap());
        assert_ne!(copied_group.id, group_id);
        assert_ne!(copied_a.id, child_a_id);
        assert_ne!(copied_b.id, child_b_id);
        assert_eq!(copied_a.parent_id, Some(copied_group.id));
        assert_eq!(copied_b.parent_id, Some(copied_group.id));
        // `a`'s feeder stayed behind; `b`'s came along under a new id.
        assert_eq!(copied_a.mask_source_id, None);
        assert_ne!(copied_b.mask_source_id, Some(child_a_id));
        assert_eq!(
            copied_b.mask_source_id,
            document.layers.iter().find(|l| l.name == "a").map(|l| l.id)
        );
    }

    /// The copy lands so the root's center sits on `at` (or the target's
    /// center); children shift by the same amount and an explicit mask
    /// placement shifts with its layer (:190-196).
    #[test]
    fn translation_moves_layers_and_mask_placements() {
        let mut source = session_with_document(100, 100);
        let mut root = pixel_layer(solid_asset(20, 20, [255, 255, 255, 255], "root"), (20.0, 30.0));
        root.mask = Some(LayerMask {
            asset: solid_asset(40, 40, [255, 255, 255, 255], "mask"),
            is_enabled: true,
            placement: Some(LayerTransform::new(
                CGPoint { x: 50.0, y: 60.0 },
                CGSize { width: 40.0, height: 40.0 },
            )),
            is_linked: true,
        });
        let root_id = append_layer(&mut source, root);
        let mut child = pixel_layer(solid_asset(4, 4, [0, 0, 255, 255], "child"), (40.0, 40.0));
        child.parent_id = Some(root_id);
        let child_id = append_layer(&mut source, child);
        let source_root_center = center_of(&source, root_id);
        let source_mask_origin = source
            .document
            .as_ref()
            .unwrap()
            .layer(root_id)
            .unwrap()
            .mask
            .as_ref()
            .unwrap()
            .placement
            .unwrap()
            .origin;

        let mut target = session_with_document(200, 200);
        copy_layers_between(&source, &mut target, root_id, None).unwrap();
        let root = target.active_layer().unwrap();
        let expected = CGPoint { x: 100.0, y: 100.0 };
        assert_eq!(root.transform.center(), expected);
        let mask_offset = root.transform.center() - source_root_center;
        let mask_origin = root.mask.as_ref().unwrap().placement.unwrap().origin;
        assert_eq!(
            (mask_origin.x - source_mask_origin.x, mask_origin.y - source_mask_origin.y),
            (mask_offset.x, mask_offset.y)
        );
        let child = target
            .document
            .as_ref()
            .unwrap()
            .layers
            .iter()
            .find(|l| l.name == "child")
            .unwrap();
        assert_eq!(
            child.transform.center(),
            CGPoint {
                x: center_of(&source, child_id).x + mask_offset.x,
                y: center_of(&source, child_id).y + mask_offset.y,
            }
        );

        // An explicit point anchors the root's center there.
        let mut target = session_with_document(200, 200);
        let at = CGPoint { x: 37.5, y: 42.0 };
        copy_layers_between(&source, &mut target, root_id, Some(at)).unwrap();
        assert_eq!(target.active_layer().unwrap().transform.center(), at);
    }

    /// A target with no document gets one sized like the source (`:189`,
    /// `:203`), and the whole copy is a single undo step that removes the
    /// document again.
    #[test]
    fn empty_target_creates_document_with_one_undo() {
        let mut source = session_with_document(320, 240);
        source.insert_image(solid_asset(20, 20, [255, 255, 255, 255], "a"), None);
        let root = source.active_layer_id.unwrap();

        let mut target = EditorSession::new();
        assert!(target.document.is_none());
        copy_layers_between(&source, &mut target, root, None).unwrap();

        let document = target.document.as_ref().unwrap();
        assert_eq!(document.width, 320);
        assert_eq!(document.height, 240);
        assert_eq!(document.layers.len(), 1);
        assert_eq!(target.active_layer_id, Some(document.layers[0].id));
        // One entry, named after Swift's beginEdit.
        assert_eq!(target.history.undo_count(), 1);
        assert_eq!(target.history.undo_name(), "Copy Layers from Project");
        target.undo();
        assert!(target.document.is_none());
        assert_eq!(target.active_layer_id, None);
        // Redo restores the created document and the copied layer's identity.
        assert!(target.can_redo());
        target.redo();
        let document = target.document.as_ref().unwrap();
        assert_eq!(document.layers.len(), 1);
        assert_eq!(target.active_layer_id, Some(document.layers[0].id));
    }
}
