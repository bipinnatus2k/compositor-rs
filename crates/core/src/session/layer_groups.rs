//! Port of the session's multi-select and bulk layer ops extension
//! (`LayerGroups.swift`).

use super::*;

impl EditorSession {
    // MARK: Multi-select and bulk layer ops (LayerGroups.swift)

    /// `selectLayers` — the selection is intersected with the document's
    /// layers; switching it commits a pending transform first.
    pub fn select_layers(&mut self, ids: &[Uuid], primary: Option<Uuid>) {
        // selectLayers (LayerGroups.swift:96-104): the effect selection dies,
        // a changed selection finishes pending text, and a changed valid
        // selection commits the transform and resolves the pending gradient.
        self.effect_selection = None;
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let known: std::collections::HashSet<Uuid> = document.layers.iter().map(|l| l.id).collect();
        let mut valid: Vec<Uuid> = Vec::new();
        for id in ids {
            if known.contains(id) && !valid.contains(id) {
                valid.push(*id);
            }
        }
        let same = valid.len() == self.selected_layer_ids.len()
            && valid.iter().all(|id| self.selected_layer_ids.contains(id));
        if !same && !self.finish_text() {
            return;
        }
        if !same {
            self.commit_transform();
            self.resolve_gradient();
        }
        self.active_layer_id = match primary {
            Some(primary) if valid.contains(&primary) => Some(primary),
            _ => valid.first().copied(),
        };
        self.selected_layer_ids = valid;
    }

    /// Cmd-Shift-click: adds a layer to the selection, or takes it out again
    /// when it is already in it (`extendSelection`).
    pub fn extend_selection(&mut self, id: Uuid) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        if !document.layers.iter().any(|l| l.id == id) {
            return;
        }
        let mut ids = self.selected_layer_ids.clone();
        if ids.contains(&id) && ids.len() > 1 {
            ids.retain(|i| *i != id);
            let primary = if self.active_layer_id == Some(id) {
                ids.first().copied()
            } else {
                self.active_layer_id
            };
            self.select_layers(&ids, primary);
        } else {
            ids.push(id);
            self.select_layers(&ids, Some(id));
        }
    }

    /// `groupSelectedLayers` — wraps the selection in a new "Folder N" at
    /// their common parent. A selected folder carries its subtree, and
    /// selected descendants are not pulled out of it.
    pub fn group_selected_layers(&mut self) {
        if !self.can_edit_layers() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        if document.layers.len() >= 10_000 {
            return;
        }
        let by_id: std::collections::HashMap<Uuid, &ImageLayer> =
            document.layers.iter().map(|l| (l.id, l)).collect();
        let selected: std::collections::HashSet<Uuid> =
            self.selected_layer_ids.iter().copied().collect();
        if selected.is_empty() {
            return;
        }
        let ancestors =
            |id: Uuid, by_id: &std::collections::HashMap<Uuid, &ImageLayer>| -> Vec<Option<Uuid>> {
                let mut result = Vec::new();
                let mut parent = by_id.get(&id).and_then(|l| l.parent_id);
                while let Some(current) = parent {
                    result.push(Some(current));
                    parent = by_id.get(&current).and_then(|l| l.parent_id);
                }
                result.push(None);
                result
            };
        let in_selected = |chain: &Vec<Option<Uuid>>| {
            chain
                .iter()
                .any(|a| a.map(|a| selected.contains(&a)).unwrap_or(false))
        };
        // The topmost selected branches: roots of the selected forest.
        let root_ids: Vec<Uuid> = selected
            .iter()
            .copied()
            .filter(|id| !in_selected(&ancestors(*id, &by_id)))
            .collect();
        let root_set: std::collections::HashSet<Uuid> = root_ids.iter().copied().collect();
        // Hierarchy order keeps the relative stacking of what moves.
        let ordered: Vec<Uuid> = document
            .hierarchy_entries(false, &std::collections::HashSet::new())
            .into_iter()
            .map(|e| e.layer.id)
            .filter(|id| root_set.contains(id))
            .collect();
        if ordered.is_empty() {
            return;
        }
        // The nearest ancestor chain every selected branch shares.
        let parent: Option<Uuid> = ordered
            .first()
            .and_then(|first| {
                let first_chain = ancestors(*first, &by_id);
                first_chain.iter().copied().find(|candidate| {
                    ordered
                        .iter()
                        .all(|id| ancestors(*id, &by_id).contains(candidate))
                })
            })
            .flatten();
        let names: std::collections::HashSet<String> =
            document.layers.iter().map(|l| l.name.clone()).collect();
        let mut number = 1;
        while names.contains(&format!("Folder {number}")) {
            number += 1;
        }
        let mut group = ImageLayer::blank(format!("Folder {number}"), document.size());
        group.is_group = true;
        group.parent_id = parent;
        // The wrapper sits where the topmost selected branch sits.
        let branches: Vec<Uuid> = ordered
            .iter()
            .map(|id| {
                let mut branch = *id;
                while let Some(next) = by_id.get(&branch).and_then(|l| l.parent_id) {
                    if Some(next) == parent {
                        break;
                    }
                    branch = next;
                }
                branch
            })
            .collect();
        let highest = document
            .layers
            .iter()
            .rposition(|l| branches.contains(&l.id));
        let insertion = highest
            .map(|h| {
                document.layers[..=h]
                    .iter()
                    .filter(|l| !root_set.contains(&l.id))
                    .count()
            })
            .unwrap_or(document.layers.len());
        let mut layers = document
            .layers
            .iter()
            .filter(|l| !root_set.contains(&l.id))
            .cloned()
            .collect::<Vec<ImageLayer>>();
        let group_id = group.id;
        layers.insert(insertion.min(layers.len()), group);
        for id in &ordered {
            let mut child = match by_id.get(id) {
                Some(child) => (*child).clone(),
                None => continue,
            };
            child.parent_id = Some(group_id);
            layers.push(child);
        }
        self.begin_edit("Group Layers");
        if let Some(doc) = self.document.as_mut() {
            doc.layers = layers;
        }
        self.active_layer_id = Some(group_id);
        self.selected_layer_ids = vec![group_id];
        if let Some(parent) = parent {
            self.collapsed_group_ids.remove(&parent);
        }
        self.end_edit();
    }

    /// `addGroup` — a new empty "Folder N" above the active layer, parented
    /// like a sibling of it (inside it when the active layer is a folder).
    pub fn add_group(&mut self) {
        if !self.can_edit_layers() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        if document.layers.len() >= 10_000 {
            return;
        }
        let names: std::collections::HashSet<String> =
            document.layers.iter().map(|l| l.name.clone()).collect();
        let mut number = 1;
        while names.contains(&format!("Folder {number}")) {
            number += 1;
        }
        let mut group = ImageLayer::blank(format!("Folder {number}"), document.size());
        group.is_group = true;
        group.parent_id = match self.active_layer() {
            Some(active) if active.is_group => self.active_layer_id,
            Some(active) => active.parent_id,
            None => None,
        };
        let insertion = self
            .active_layer_id
            .and_then(|id| document.layer_index(id))
            .map(|i| i + 1)
            .unwrap_or(document.layers.len());
        let group_id = group.id;
        let group_parent = group.parent_id;
        self.begin_edit("New Folder");
        if let Some(doc) = self.document.as_mut() {
            doc.layers.insert(insertion.min(doc.layers.len()), group);
            self.active_layer_id = Some(group_id);
            self.selected_layer_ids = vec![group_id];
        }
        if let Some(parent) = group_parent {
            self.collapsed_group_ids.remove(&parent);
        }
        self.end_edit();
    }

    /// `toggleGroupExpansion` — collapsing takes the active selection with it
    /// when it lives inside the folder.
    pub fn toggle_group_expansion(&mut self, id: Uuid) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        if !document.layer(id).map(|l| l.is_group).unwrap_or(false) {
            return;
        }
        if self.collapsed_group_ids.contains(&id) {
            self.collapsed_group_ids.remove(&id);
        } else {
            let active_inside = self
                .active_layer_id
                .map(|a| document.descendant_ids(id).contains(&a))
                .unwrap_or(false);
            if active_inside {
                self.select_layer(Some(id));
            }
            self.collapsed_group_ids.insert(id);
        }
    }

    /// `canPlaceLayer` — a folder never lands inside its own subtree.
    pub fn can_place_layer(&self, id: Uuid, parent: Option<Uuid>) -> bool {
        if !self.can_edit_layers() {
            return false;
        }
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        if !document.layers.iter().any(|l| l.id == id) {
            return false;
        }
        let Some(parent) = parent else {
            return true;
        };
        parent != id
            && !document.descendant_ids(id).contains(&parent)
            && document.layer(parent).map(|l| l.is_group).unwrap_or(false)
    }

    /// `placeLayer` — moves a layer (with its subtree) under `parent`, above
    /// `target`, or at the very bottom, as one "Move Layer" undo step.
    pub fn place_layer(
        &mut self,
        id: Uuid,
        parent: Option<Uuid>,
        above: Option<Uuid>,
        at_bottom: bool,
    ) -> bool {
        if above == Some(id) || !self.can_place_layer(id, parent) {
            return false;
        }
        let Some(document) = self.document.clone() else {
            return false;
        };
        let Some(index) = document.layer_index(id) else {
            return false;
        };
        let mut layers = document.layers.clone();
        let mut layer = layers.remove(index);
        layer.parent_id = parent;
        let mut insertion = if at_bottom { 0 } else { layers.len() };
        if let Some(target) = above {
            match layers
                .iter()
                .position(|l| l.id == target && l.parent_id == parent)
            {
                Some(target_index) => insertion = target_index + 1,
                None => return false,
            }
        }
        layers.insert(insertion.min(layers.len()), layer);
        adopt_clipping(id, &mut layers);
        release_detached_clipping(&mut layers);
        self.begin_edit("Move Layer");
        if let Some(doc) = self.document.as_mut() {
            doc.layers = layers;
        }
        self.active_layer_id = Some(id);
        self.selected_layer_ids = vec![id];
        if let Some(parent) = parent {
            self.collapsed_group_ids.remove(&parent);
        }
        self.end_edit();
        true
    }

    /// `moveActiveLayerOutOfGroup` — above the group, at the group's parent.
    pub fn move_active_layer_out_of_group(&mut self) {
        let Some(layer) = self.active_layer().cloned() else {
            return;
        };
        let Some(parent) = layer.parent_id else {
            return;
        };
        let Some(group) = self
            .document
            .as_ref()
            .and_then(|d| d.layer(parent))
            .cloned()
        else {
            return;
        };
        self.place_layer(layer.id, group.parent_id, Some(group.id), false);
    }

    /// `reorderLayers` — panel rows move top-to-bottom while the document
    /// stores bottom-to-top, so the panel order is the reversed array.
    pub fn reorder_layers(&mut self, from: &[usize], destination: usize) {
        if !self.can_edit_layers() {
            return;
        }
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let mut layers = document.layers.clone();
        layers.reverse();
        if from.is_empty() || from.iter().any(|i| *i >= layers.len()) || destination > layers.len()
        {
            return;
        }
        // Array.move(fromOffsets:toOffset:): remove (lowest first), then
        // insert before the destination, adjusted for rows removed above it.
        let mut moved = Vec::new();
        let mut removed_below = 0usize;
        let mut offsets: Vec<usize> = from.to_vec();
        offsets.sort_unstable();
        offsets.dedup();
        for (k, offset) in offsets.iter().enumerate() {
            if *offset < destination {
                removed_below += 1;
            }
            moved.push(layers.remove(offset - k));
        }
        moved.reverse();
        let insertion = (destination - removed_below).min(layers.len());
        for (k, layer) in moved.into_iter().enumerate() {
            layers.insert(insertion + k, layer);
        }
        layers.reverse();
        self.begin_edit("Reorder Layers");
        if let Some(doc) = self.document.as_mut() {
            doc.layers = layers;
        }
        self.end_edit();
    }

    /// Photoshop's eye swipe (`beginVisibilitySwipe`): opens one undo step
    /// that every following swipe joins until `end_visibility_swipe`.
    pub fn begin_visibility_swipe(&mut self, id: Uuid) -> Option<bool> {
        if !self.can_edit_layers() {
            return None;
        }
        let visible = !self.document.as_ref()?.layer(id)?.is_visible;
        self.begin_edit(if visible { "Show Layer" } else { "Hide Layer" });
        self.set_visibility_in_swipe(id, visible);
        Some(visible)
    }

    pub fn set_visibility_in_swipe(&mut self, id: Uuid, visible: bool) {
        if let Some(document) = self.document.as_mut()
            && let Some(layer) = document.layer_mut(id)
            && layer.is_visible != visible
        {
            layer.is_visible = visible;
        }
    }

    pub fn end_visibility_swipe(&mut self) {
        self.end_edit();
    }

    /// `deleteSelectedLayers` — every selected layer (a selected folder takes
    /// its contents) in one undo step; one selection just deletes it. When the
    /// deleted layers supply live masks to layers that stay, Swift asks
    /// bake-or-unlink (EditorSession.swift:652); the plain path bakes so the
    /// dependents keep their masked appearance (see `delete_layer`).
    pub fn delete_selected_layers(&mut self) {
        if !self.can_edit_layers() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let ids: Vec<Uuid> = document
            .layers
            .iter()
            .map(|l| l.id)
            .filter(|id| self.selected_layer_ids.contains(id))
            .collect();
        if ids.len() <= 1 {
            self.delete_active_layer();
            return;
        }
        if self.delete_with_live_mask_choice(&ids, true) {
            return;
        }
        let mut removed = std::collections::HashSet::new();
        for id in &ids {
            removed.extend(document.descendant_ids(*id));
        }
        self.begin_edit("Delete Layers");
        if let Some(doc) = self.document.as_mut() {
            doc.layers.retain(|l| !removed.contains(&l.id));
            for layer in doc.layers.iter_mut() {
                if layer
                    .mask_source_id
                    .map(|source| removed.contains(&source))
                    .unwrap_or(false)
                {
                    layer.mask_source_id = None;
                }
            }
        }
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
                let index = document.layer_index(ids[0]).unwrap_or(0).min(count - 1);
                let neighbor = self.document.as_ref().map(|d| d.layers[index].id);
                self.active_layer_id = neighbor;
                self.selected_layer_ids = neighbor.map(|n| vec![n]).unwrap_or_default();
            }
        }
        self.end_edit();
    }

    /// `layerRows` — the Layers-panel rows, top first, honoring collapse.
    pub fn layer_rows(&self) -> Vec<crate::document::HierarchyEntry<'_>> {
        match self.document.as_ref() {
            Some(document) => document.hierarchy_entries(true, &self.collapsed_group_ids),
            None => Vec::new(),
        }
    }
}
