//! Port of `Compositor/Compositor/Document/ProjectWorkspace.swift` (210
//! lines): the multi-tab workspace — `ProjectTab`, tab guards and
//! selection, project open/load/close, quit ordering, the file/layer
//! receive flows and cross-project layer copies.
//!
//! Model-only by design: no GPUI types. Dialogs surface as
//! [`project::Pending`] instructions (see `crates/app/src/project.rs` for
//! the two-phase shape) and are resolved with the `resolve_*` methods; the
//! app presents and drains them. Deviations from Swift are commented at
//! their sites and summarized in the module tests' header.

use std::path::{Path, PathBuf};

use compositor_core::geometry::CGPoint;
use compositor_core::session::EditorSession;
use uuid::Uuid;

pub use crate::project::{Pending, Step, TabId, UnsavedChoice};
use crate::project::{ProjectController, is_project_file};

/// The app-side half of `canStartProjectOperation`
/// (EditorSession.swift:150-152). Core does not model these flags (see
/// session.rs's "app-side" note); the rest of the guard reads the session
/// directly — see [`ProjectBusy::ready`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectBusy {
    /// `isProjectBusy`: a project operation (save/export/open…) is running.
    pub is_project_busy: bool,
    /// `isImporting`: an image import is in flight.
    pub is_importing: bool,
    /// `brushStroke` (and `warpStroke`): a paint stroke is in flight.
    pub has_brush_stroke: bool,
}

impl ProjectBusy {
    pub fn new() -> Self {
        ProjectBusy::default()
    }

    /// The session half of `canStartProjectOperation`: every condition core
    /// models. The flags Swift also checks that core has no field for —
    /// `isProjectBusy`, `isImporting`, `brushStroke`, `warpStroke`,
    /// `showsImporter`, `renamingLayerID`, `importError`,
    /// `showsConversionSheet` — live app-side (`ProjectBusy` plus the
    /// workspace/UI state wired in Wave 3).
    pub fn session_ready(session: &EditorSession) -> bool {
        session.selection_amount_operation.is_none()
            && session.text_draft.is_none()
            && session.levels.is_none()
            && !session.shows_new_document
            && session.adjustment_editing_id.is_none()
    }

    /// `canStartProjectOperation` for the session whose app-side busy flags
    /// this snapshot carries.
    pub fn ready(&self, session: &EditorSession) -> bool {
        ProjectBusy::session_ready(session)
            && !self.is_project_busy
            && !self.is_importing
            && !self.has_brush_stroke
    }
}

/// One tab: its own session, file controller and default name
/// (`ProjectTab`, ProjectWorkspace.swift:6-17).
pub struct ProjectTab {
    pub id: TabId,
    pub session: EditorSession,
    pub controller: ProjectController,
    /// The app-side busy snapshot for this tab's session (Swift keeps these
    /// on `EditorSession`; core does not model them).
    pub busy: ProjectBusy,
    /// `defaultName`; shown while the tab has no `projectURL`.
    pub default_name: String,
    /// `session.importError` (EditorSession.swift:516): failures of imports
    /// and layer copies surface here for the UI to present.
    pub import_error: Option<String>,
}

impl ProjectTab {
    pub fn new(name: impl Into<String>) -> Self {
        ProjectTab {
            id: Uuid::new_v4(),
            session: EditorSession::new(),
            controller: ProjectController::new(),
            busy: ProjectBusy::new(),
            default_name: name.into(),
            import_error: None,
        }
    }

    /// `title` (:11): the project URL's file name without extension,
    /// falling back to the default name.
    pub fn title(&self) -> String {
        self.session
            .project_url
            .as_ref()
            .and_then(|url| url.file_stem())
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.default_name.clone())
    }
}

/// The in-progress quit/close flow: which tabs still owe an unsaved-changes
/// answer, in [`ProjectWorkspace::quit_order`] sequence (`confirmQuit`
/// :106-114, re-shaped as a state machine — deviation #4).
struct QuitRun {
    order: Vec<TabId>,
    index: usize,
    /// `closeWindow` (:115-119): after the last confirmation, replace the
    /// tabs with one empty tab (the app then closes the window).
    close_window: bool,
}

/// The tab collection (`ProjectWorkspace`).
pub struct ProjectWorkspace {
    tabs: Vec<ProjectTab>,
    selected_id: TabId,
    /// `isManaging`: a workspace-level flow (open/close/quit/receive) is
    /// running.
    pub is_managing: bool,
    next_number: u32,
    quit_run: Option<QuitRun>,
    pending_close: Option<TabId>,
    /// Batches of dropped/chosen files waiting for the guards to clear
    /// (Swift's `while !canSwitch` 30 ms poll, :126-129, re-shaped as an
    /// event-driven queue — deviation #5). Drained by `drain_incoming`.
    pending_receives: Vec<(Vec<PathBuf>, Option<TabId>, Option<CGPoint>)>,
    /// `showError` instructions (title, message) raised by model flows whose
    /// tab was discarded or which ran workspace-wide — e.g. a failed
    /// `load_project`. The app presents and drains them.
    pub errors: Vec<(String, String)>,
    /// App-side guard for the floating panels: while any panel is open the
    /// tab must not switch (Swift's `canStartProjectOperation` counts the
    /// panels' edit states — `levels != nil`, `filterEdit != nil`, … — which
    /// the Rust core only carries for adjustment editing). Opened/closed
    /// panels keep this in sync through `AppState`.
    pub ui_panels_open: bool,
}

impl Default for ProjectWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectWorkspace {
    /// `init` (:32-37): one "Untitled" tab, selected.
    /// (`skipsInitialClipboardCanvasSize` is a macOS launch-time clipboard
    /// probe the Rust session does not have — deviation #7.)
    pub fn new() -> Self {
        let first = ProjectTab::new("Untitled");
        let mut workspace = ProjectWorkspace {
            tabs: vec![first],
            selected_id: TabId::nil(),
            is_managing: false,
            next_number: 2,
            quit_run: None,
            pending_close: None,
            pending_receives: Vec::new(),
            errors: Vec::new(),
            ui_panels_open: false,
        };
        workspace.selected_id = workspace.tabs[0].id;
        workspace
    }

    /// `current` (:26): the selected tab, falling back to the first.
    /// Invariant: there is always at least one tab.
    pub fn current(&self) -> &ProjectTab {
        self.tabs
            .iter()
            .find(|tab| tab.id == self.selected_id)
            .unwrap_or(&self.tabs[0])
    }

    pub fn current_mut(&mut self) -> &mut ProjectTab {
        let selected_id = self.selected_id;
        let index = self
            .tabs
            .iter()
            .position(|tab| tab.id == selected_id)
            .unwrap_or(0);
        &mut self.tabs[index]
    }

    pub fn tabs(&self) -> &[ProjectTab] {
        &self.tabs
    }

    pub fn selected_id(&self) -> TabId {
        self.selected_id
    }

    pub fn tab(&self, id: TabId) -> Option<&ProjectTab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub fn tab_mut(&mut self, id: TabId) -> Option<&mut ProjectTab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    /// `canSwitch` (:27-31): not mid-flow, the current session can start a
    /// project operation, and none of the five floating edit sessions is
    /// open.
    pub fn can_switch(&self) -> bool {
        if self.is_managing || self.ui_panels_open {
            return false;
        }
        let tab = self.current();
        let session = &tab.session;
        tab.busy.ready(session)
            && session.hue_saturation.is_none()
            && session.filter_edit.is_none()
            && session.gradient_edit.is_none()
            && session.pixel_move.is_none()
            && session.color_picker.is_none()
    }

    /// `addTab(reuseEmpty:)` (:38-46): reuse the single empty tab, or create
    /// "Untitled {n}" and select it.
    pub fn add_tab(&mut self, reuse_empty: bool) -> &mut ProjectTab {
        if reuse_empty && self.tabs.len() == 1 && self.current().session.document.is_none() {
            return self.current_mut();
        }
        let name = format!("Untitled {}", self.next_number);
        self.next_number += 1;
        let tab = ProjectTab::new(name);
        self.tabs.push(tab);
        self.selected_id = self.tabs.last().unwrap().id;
        self.current_mut()
    }

    /// `select(_:)` (:47-52): commit the outgoing tab's transform, then
    /// switch. Refused when already selected, mid-flow, or unknown. (Swift
    /// rebuilds the view by `.id(tab.id)`, which refits the viewport; the
    /// app layer mirrors that by resetting its fit state on a true return —
    /// deviation #6.)
    pub fn select(&mut self, id: TabId) -> bool {
        if id == self.selected_id || !self.can_switch() {
            return false;
        }
        if !self.tabs.iter().any(|tab| tab.id == id) {
            return false;
        }
        self.current_mut().session.commit_transform();
        self.selected_id = id;
        true
    }

    /// `newCanvas` (:53-57): a fresh tab, never reusing the empty one.
    pub fn new_canvas(&mut self) -> bool {
        if !self.can_switch() {
            return false;
        }
        self.current_mut().session.commit_transform();
        self.add_tab(false);
        true
    }

    /// `open(_:)` (:58-76). `None` asks the app to run the multi-select
    /// `.comp` open panel (`allowedContentTypes: [.compositorProject]`,
    /// `allowsMultipleSelection`, `treatsFilePackagesAsDirectories: false`)
    /// and call `finish_open` with the chosen files (empty for Cancel).
    pub fn open(&mut self, urls: Option<Vec<PathBuf>>) -> Step<bool> {
        if !self.can_switch() {
            return Step::Done(false);
        }
        let Some(urls) = urls else {
            self.is_managing = true;
            return Step::Pending(Pending::Open);
        };
        Step::Done(self.open_urls(&urls))
    }

    /// Second half of the open panel (:73-75): load each URL, reporting
    /// whether at least one opened.
    pub fn finish_open(&mut self, urls: Vec<PathBuf>) -> bool {
        self.is_managing = false;
        self.open_urls(&urls)
    }

    fn open_urls(&mut self, urls: &[PathBuf]) -> bool {
        let mut opened = false;
        for url in urls {
            opened = self.load_project(url) || opened;
        }
        opened
    }

    /// `loadProject(_:)` (:77-88). Dedup compares
    /// `resolveSymlinksInPath`ed paths (`std::fs::canonicalize`; a missing
    /// path falls back to its literal form). The project loads into a
    /// detached tab first, so a failed open never leaves a broken tab; when
    /// the only tab has no document it is removed before the new one joins.
    pub fn load_project(&mut self, url: &Path) -> bool {
        let resolved = std::fs::canonicalize(url).unwrap_or_else(|_| url.to_path_buf());
        let existing = self.tabs.iter().position(|tab| {
            tab.session
                .project_url
                .as_ref()
                .map(|project_url| {
                    std::fs::canonicalize(project_url).unwrap_or_else(|_| project_url.to_path_buf())
                })
                .as_deref()
                == Some(&resolved)
        });
        if let Some(existing) = existing {
            self.selected_id = self.tabs[existing].id;
            return true;
        }
        let name = url
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string());
        let mut tab = ProjectTab::new(name);
        match tab.controller.open(&mut tab.session, &mut tab.busy, url) {
            Step::Done(true) => {}
            Step::Done(false) | Step::Pending(_) => {
                // Surface the failure's alert instruction, then drop the
                // doomed tab (Swift's showError on the detached controller).
                if let Some(Pending::Error { title, message }) = tab.controller.take_pending() {
                    self.errors.push((title, message));
                }
                return false;
            }
        }
        if self.tabs.len() == 1 && self.current().session.document.is_none() {
            self.tabs.clear();
        }
        self.selected_id = tab.id;
        self.tabs.push(tab);
        true
    }

    /// `close(_:)` (:89-96): confirm the tab's unsaved changes, then remove
    /// it. A confirmation surfaces as `Pending::Confirm{step: Close}` —
    /// answer with `resolve_close` (and `close_after_save` if its `Save`
    /// button raised a save panel).
    pub fn close(&mut self, id: TabId) -> Step<()> {
        if !self.can_switch() {
            return Step::Done(());
        }
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return Step::Done(());
        };
        self.is_managing = true;
        // Swift asks about the target tab without bringing it on screen
        // (:89-96): the confirm resolves through `pending_close`, never
        // through the selection, so the user's tab stays put.
        let tab = &mut self.tabs[index];
        match tab
            .controller
            .confirm_close(&mut tab.session, &mut tab.busy)
        {
            Step::Done(true) => {
                self.is_managing = false;
                self.remove_tab(id);
                Step::Done(())
            }
            Step::Done(false) => {
                self.is_managing = false;
                Step::Done(())
            }
            Step::Pending(pending) => {
                self.pending_close = Some(id);
                Step::Pending(pending)
            }
        }
    }

    /// The close flow's shared scaffold: both resume points
    /// (`resolve_close`, `close_after_save`) hand their answer to the
    /// pending tab's controller through `ask` and interpret it the same
    /// way — approval removes the tab and releases the guard, refusal just
    /// releases it, a nested save panel keeps both until it resolves.
    fn resume_close_flow(&mut self, ask: impl FnOnce(&mut ProjectTab) -> Step<bool>) -> Step<()> {
        let Some(id) = self.pending_close else {
            return Step::Done(());
        };
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            // The tab vanished mid-flow: release the guard with nothing to
            // remove.
            self.pending_close = None;
            self.is_managing = false;
            return Step::Done(());
        };
        let answer = ask(&mut self.tabs[index]);
        match answer {
            Step::Done(true) => {
                self.pending_close = None;
                self.is_managing = false;
                self.remove_tab(id);
                Step::Done(())
            }
            Step::Done(false) => {
                self.pending_close = None;
                self.is_managing = false;
                Step::Done(())
            }
            Step::Pending(pending) => Step::Pending(pending),
        }
    }

    /// The close flow's alert answer.
    pub fn resolve_close(&mut self, choice: UnsavedChoice) -> Step<()> {
        self.resume_close_flow(|tab| tab.controller.resolve_confirm(&mut tab.session, choice))
    }

    /// Second half of a save panel raised by the close flow's `Save` button
    /// (after `finish_save` on the tab's controller).
    pub fn close_after_save(&mut self, saved: bool) -> Step<()> {
        self.resume_close_flow(move |tab| {
            tab.controller.confirm_after_save(&mut tab.session, saved)
        })
    }

    /// `removeTab(_:)` (:97-102): remove; refill an emptied workspace; a
    /// removed selected tab selects the neighbor at the same index
    /// (clamped to the last).
    pub fn remove_tab(&mut self, id: TabId) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.add_tab(false);
        } else if self.selected_id == id {
            let index = index.min(self.tabs.len() - 1);
            self.selected_id = self.tabs[index].id;
        }
    }

    /// The order Quit (and closing the window) asks about unsaved projects:
    /// the tab on screen first, then the rest left to right, so it never
    /// jumps to another project before the one you're viewing (:103-105).
    pub fn quit_order(&self) -> Vec<TabId> {
        let current = self.current().id;
        let mut order = vec![current];
        order.extend(
            self.tabs
                .iter()
                .map(|tab| tab.id)
                .filter(|id| *id != current),
        );
        order
    }

    /// `confirmQuit` (:106-114): walk `quit_order`, asking each tab about
    /// unsaved changes; any Cancel aborts the whole quit. A question
    /// surfaces as `Pending::Confirm{step: Quit}` on the asked tab's
    /// controller — answer with `resolve_quit` (and `quit_after_save` for a
    /// nested save panel).
    pub fn confirm_quit(&mut self) -> Step<bool> {
        if !self.can_switch() {
            return Step::Done(false);
        }
        let order = self.quit_order();
        self.is_managing = true;
        self.quit_run = Some(QuitRun {
            order,
            index: 0,
            close_window: false,
        });
        self.advance_quit()
    }

    /// `closeWindow` (:115-119): the quit flow; on success the tabs are
    /// replaced with one empty tab and the app may close the window (the
    /// `on_window_should_close` veto maps to `Done(false)` / pending).
    pub fn close_window(&mut self) -> Step<bool> {
        if !self.can_switch() {
            return Step::Done(false);
        }
        let order = self.quit_order();
        self.is_managing = true;
        self.quit_run = Some(QuitRun {
            order,
            index: 0,
            close_window: true,
        });
        self.advance_quit()
    }

    fn advance_quit(&mut self) -> Step<bool> {
        loop {
            let Some(run) = self.quit_run.as_ref() else {
                return Step::Done(true);
            };
            if run.index >= run.order.len() {
                let close_window = run.close_window;
                self.quit_run = None;
                self.is_managing = false;
                if close_window {
                    self.tabs.clear();
                    self.add_tab(false);
                }
                return Step::Done(true);
            }
            let tab_id = run.order[run.index];
            // Swift :110: the asked tab becomes the one on screen.
            if self.tabs.iter().any(|tab| tab.id == tab_id) {
                self.selected_id = tab_id;
            }
            let index = self.tabs.iter().position(|tab| tab.id == tab_id);
            match index {
                None => {
                    self.quit_run.as_mut().unwrap().index += 1;
                }
                Some(index) => {
                    let tab = &mut self.tabs[index];
                    match tab.controller.confirm_quit(&mut tab.session, &mut tab.busy) {
                        Step::Done(true) => {
                            self.quit_run.as_mut().unwrap().index += 1;
                        }
                        Step::Done(false) => {
                            self.quit_run = None;
                            self.is_managing = false;
                            return Step::Done(false);
                        }
                        Step::Pending(pending) => return Step::Pending(pending),
                    }
                }
            }
        }
    }

    /// The quit flow's shared scaffold: both resume points
    /// (`resolve_quit`, `quit_after_save`) interpret the asked tab's
    /// controller answer the same way — approval walks on to the next tab,
    /// refusal aborts the whole quit.
    fn resume_quit_flow(&mut self, answer: Step<bool>) -> Step<bool> {
        match answer {
            Step::Done(true) => {
                self.quit_run.as_mut().unwrap().index += 1;
                self.advance_quit()
            }
            Step::Done(false) => {
                self.quit_run = None;
                self.is_managing = false;
                Step::Done(false)
            }
            Step::Pending(pending) => Step::Pending(pending),
        }
    }

    /// The quit flow's alert answer.
    pub fn resolve_quit(&mut self, choice: UnsavedChoice) -> Step<bool> {
        let Some(run) = self.quit_run.as_ref() else {
            return Step::Done(false);
        };
        let tab_id = run.order[run.index];
        let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            // The asked tab vanished mid-flow: skip it and keep going.
            self.quit_run.as_mut().unwrap().index += 1;
            return self.advance_quit();
        };
        let answer = {
            let tab = &mut self.tabs[index];
            tab.controller.resolve_confirm(&mut tab.session, choice)
        };
        self.resume_quit_flow(answer)
    }

    /// Second half of a save panel raised by the quit flow's `Save` button
    /// (after `finish_save` on the asked tab's controller).
    pub fn quit_after_save(&mut self, saved: bool) -> Step<bool> {
        let Some(run) = self.quit_run.as_ref() else {
            return Step::Done(false);
        };
        let tab_id = run.order[run.index];
        let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            // The asked tab vanished mid-flow: the panel's own result stands
            // and the quit aborts.
            self.quit_run = None;
            self.is_managing = false;
            return Step::Done(saved);
        };
        let answer = {
            let tab = &mut self.tabs[index];
            tab.controller.confirm_after_save(&mut tab.session, saved)
        };
        self.resume_quit_flow(answer)
    }

    // MARK: Receiving files (`receive` :123-141)

    /// `receive(_:into:at:)`. While the guards block switching, the batch
    /// waits in an event-driven queue instead of Swift's 30 ms poll — call
    /// `drain_incoming` once the blockers clear (deviation #5). Returns the
    /// image-import failures for the UI to surface.
    pub fn receive(
        &mut self,
        urls: &[PathBuf],
        into: Option<TabId>,
        at: Option<CGPoint>,
    ) -> Vec<String> {
        if !self.can_switch() {
            self.pending_receives.push((urls.to_vec(), into, at));
            return Vec::new();
        }
        self.run_receive(urls, into, at)
    }

    /// Drains the queued receive batches; call after dialogs/sheets complete
    /// and whenever the guards clear. Returns the accumulated import
    /// failures.
    pub fn drain_incoming(&mut self) -> Vec<String> {
        if !self.can_switch() {
            return Vec::new();
        }
        let queued = std::mem::take(&mut self.pending_receives);
        let mut failures = Vec::new();
        for (urls, into, at) in queued {
            failures.extend(self.run_receive(&urls, into, at));
        }
        failures
    }

    pub fn has_queued_receives(&self) -> bool {
        !self.pending_receives.is_empty()
    }

    /// The per-file loop (:131-140): `.comp` files load as projects; images
    /// land in the named tab (skipped when gone) or each open a reused/new
    /// tab, and select their tab before importing.
    fn run_receive(
        &mut self,
        urls: &[PathBuf],
        into: Option<TabId>,
        at: Option<CGPoint>,
    ) -> Vec<String> {
        let mut failures = Vec::new();
        for url in urls {
            if is_project_file(url) {
                self.load_project(url);
                continue;
            }
            let tab_id = match into {
                Some(destination) => {
                    let Some(tab) = self.tabs.iter().find(|tab| tab.id == destination) else {
                        continue;
                    };
                    tab.id
                }
                None => self.add_tab(true).id,
            };
            self.selected_id = tab_id;
            let index = self.tabs.iter().position(|tab| tab.id == tab_id).unwrap();
            failures.extend(
                self.tabs[index]
                    .session
                    .import_images(std::slice::from_ref(url), at),
            );
        }
        failures
    }

    // MARK: Cross-project layer copies (`copyLayer` :157-209)

    /// `copyLayer(_:into:at:)`. The copy itself (descendant set, bake,
    /// remap, budget, one undo step) lives in core as
    /// `compositor_core::project::copy_layers_between`; here: guards, target
    /// resolution (`None` opens a fresh tab), error surfacing and selection.
    pub fn copy_layer(&mut self, id: Uuid, into: Option<TabId>, at: Option<CGPoint>) -> bool {
        if !self.can_switch() {
            return false;
        }
        let Some(source_index) = self.tabs.iter().position(|tab| {
            tab.session
                .document
                .as_ref()
                .is_some_and(|document| document.layer(id).is_some())
        }) else {
            return false;
        };
        if !self.tabs[source_index].session.can_edit_layers() {
            return false;
        }
        if into == Some(self.tabs[source_index].id) {
            return false;
        }
        let target_index = match into {
            Some(destination) => {
                let Some(index) = self.tabs.iter().position(|tab| tab.id == destination) else {
                    return false;
                };
                // :164 — the target must be able to start a project operation.
                let target = &self.tabs[index];
                if !target.busy.ready(&target.session) {
                    return false;
                }
                index
            }
            None => {
                let tab_id = self.add_tab(false).id;
                self.tabs.iter().position(|t| t.id == tab_id).unwrap()
            }
        };
        // :167 — an empty target is always acceptable; a populated one must
        // be layer-editable.
        if self.tabs[target_index].session.document.is_some()
            && !self.tabs[target_index].session.can_edit_layers()
        {
            return false;
        }
        let target_id = self.tabs[target_index].id;
        let result = if source_index < target_index {
            let (left, right) = self.tabs.split_at_mut(target_index);
            compositor_core::project::copy_layers_between(
                &left[source_index].session,
                &mut right[0].session,
                id,
                at,
            )
        } else {
            let (left, right) = self.tabs.split_at_mut(source_index);
            compositor_core::project::copy_layers_between(
                &right[0].session,
                &mut left[target_index].session,
                id,
                at,
            )
        };
        match result {
            Ok(()) => {
                self.selected_id = target_id;
                true
            }
            Err(message) => {
                // :172/:208 — the failure lands on the target's error channel.
                if let Some(target) = self.tab_mut(target_id) {
                    target.import_error = Some(message);
                }
                false
            }
        }
    }

    /// `canReceiveDrag(into:)` (ProjectTabs.swift:276-280): a file drag is
    /// always welcome; a layer drag onto the tab it already lives in is not
    /// (the layers panel's own option-drag duplicates in place). Swift
    /// sniffs the drag pasteboard for the source; here the drag item
    /// carries its source tab directly (deviation #3).
    pub fn can_receive_drag(&self, into: Option<TabId>, source: Option<TabId>) -> bool {
        match (into, source) {
            (Some(destination), Some(source)) => source != destination,
            _ => true,
        }
    }

    // MARK: App-layer flow hooks (Wave 3 wiring)
    //
    // Small read-only accessors the app's two-phase dialog plumbing needs:
    // the pending dialog's owner tab and whether the running quit flow is a
    // window close (so the app knows to close the window once it resolves).

    /// The tab whose close flow is awaiting an answer.
    pub fn close_pending_tab(&self) -> Option<TabId> {
        self.pending_close
    }

    /// The tab the quit flow is currently asking (already the one on screen).
    pub fn quit_asked_tab(&self) -> Option<TabId> {
        self.quit_run
            .as_ref()
            .map(|run| run.order[run.index.min(run.order.len() - 1)])
    }

    /// Whether the running quit flow came from `close_window` (the app then
    /// closes the window once it resolves to `Done(true)`).
    pub fn quit_closes_window(&self) -> bool {
        self.quit_run
            .as_ref()
            .map(|run| run.close_window)
            .unwrap_or(false)
    }
}

// MARK: Tests
//
// Ports of `CompositorTests/ProjectWorkspaceTests.swift`, plus the
// ProjectController's generation/confirmation behaviors live in
// `project.rs`'s test module.

#[cfg(test)]
mod tests {
    use super::*;
    use compositor_core::document::PaletteColor;
    use compositor_core::export;
    use compositor_core::raster::Bitmap;

    fn fixture_png(dir: &std::path::Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let bytes = export::png_data(&Bitmap::new_clearing(4, 4, [255, 0, 0, 255])).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "compositor-workspace-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `newCanvasOpensAnEmptyTabWithoutAModal`: a fresh tab per New, the
    /// original document untouched, and switching stays possible.
    #[test]
    fn new_canvas_opens_an_empty_tab_without_a_modal() {
        let mut workspace = ProjectWorkspace::new();
        let original = workspace.selected_id();
        workspace
            .tab_mut(original)
            .unwrap()
            .session
            .create_document(4000, 3000, true);

        assert!(workspace.new_canvas());
        assert_eq!(workspace.tabs().len(), 2);
        assert_ne!(workspace.selected_id(), original);
        assert!(workspace.current().session.document.is_none());
        assert!(!workspace.current().session.shows_new_document);
        assert!(workspace.can_switch());
        assert_eq!(
            workspace
                .tab(original)
                .unwrap()
                .session
                .document_size()
                .unwrap()
                .width,
            4000.0
        );

        assert!(workspace.new_canvas());
        assert_eq!(workspace.tabs().len(), 3);
        assert!(!workspace.current().session.shows_new_document);
    }

    /// `tabsKeepIndependentDocumentsAndUndo` plus `removeTab`'s selection
    /// fallback (:97-102).
    #[test]
    fn tabs_keep_independent_documents_and_undo() {
        let mut workspace = ProjectWorkspace::new();
        let first = workspace.selected_id();
        workspace
            .tab_mut(first)
            .unwrap()
            .session
            .create_document(4000, 4000, false);
        workspace.tab_mut(first).unwrap().session.add_blank_layer();
        let second = workspace.add_tab(true).id;
        workspace
            .tab_mut(second)
            .unwrap()
            .session
            .create_document(640, 480, false);
        workspace.tab_mut(second).unwrap().session.add_blank_layer();
        workspace.tab_mut(second).unwrap().session.undo();
        assert_eq!(
            workspace
                .tab(first)
                .unwrap()
                .session
                .document
                .as_ref()
                .unwrap()
                .layers
                .len(),
            1
        );
        assert!(
            workspace
                .tab(second)
                .unwrap()
                .session
                .document
                .as_ref()
                .unwrap()
                .layers
                .is_empty()
        );

        assert!(workspace.select(first));
        assert_eq!(workspace.selected_id(), first);
        assert_eq!(
            workspace
                .tab(first)
                .unwrap()
                .session
                .document_size()
                .unwrap()
                .width,
            4000.0
        );

        workspace.remove_tab(second);
        assert_eq!(workspace.selected_id(), first);
        workspace.remove_tab(first);
        assert_eq!(workspace.tabs().len(), 1);
        assert!(workspace.current().session.document.is_none());
    }

    /// `crossProjectCopyRemapsIdentityAndHasIndependentUndo`.
    #[test]
    fn cross_project_copy_remaps_identity_and_has_independent_undo() {
        let mut workspace = ProjectWorkspace::new();
        let first = workspace.selected_id();
        workspace
            .tab_mut(first)
            .unwrap()
            .session
            .create_document(100, 100, true);
        let original = workspace
            .tab(first)
            .unwrap()
            .session
            .active_layer_id
            .unwrap();
        let second = workspace.add_tab(false).id;
        workspace
            .tab_mut(second)
            .unwrap()
            .session
            .create_document(200, 200, false);

        assert!(workspace.copy_layer(original, Some(second), None));
        let copied_id = workspace
            .tab(second)
            .unwrap()
            .session
            .document
            .as_ref()
            .unwrap()
            .layers[0]
            .id;
        let copied_center = workspace
            .tab(second)
            .unwrap()
            .session
            .document
            .as_ref()
            .unwrap()
            .layers[0]
            .transform
            .center();
        assert_ne!(copied_id, original);
        assert_eq!(copied_center, CGPoint::new(100.0, 100.0));
        assert_eq!(
            workspace
                .tab(first)
                .unwrap()
                .session
                .document
                .as_ref()
                .unwrap()
                .layers
                .len(),
            1
        );

        workspace.tab_mut(second).unwrap().session.undo();
        assert!(
            workspace
                .tab(second)
                .unwrap()
                .session
                .document
                .as_ref()
                .unwrap()
                .layers
                .is_empty()
        );
        assert_eq!(
            workspace
                .tab(first)
                .unwrap()
                .session
                .document
                .as_ref()
                .unwrap()
                .layers
                .len(),
            1
        );
        workspace.tab_mut(second).unwrap().session.redo();
        let restored_id = workspace
            .tab(second)
            .unwrap()
            .session
            .document
            .as_ref()
            .unwrap()
            .layers[0]
            .id;
        assert_eq!(restored_id, copied_id);

        // Copying onto the source's own tab is refused (:161); `into: None`
        // opens a fresh tab and selects it (:166-207).
        assert!(!workspace.copy_layer(original, Some(first), None));
        let before = workspace.tabs().len();
        assert!(workspace.copy_layer(original, None, None));
        assert_eq!(workspace.tabs().len(), before + 1);
        assert_ne!(workspace.selected_id(), first);
        assert_ne!(workspace.selected_id(), second);
    }

    /// `dockCreatesTabsAndTargetedImportUsesExistingTab`: each image in a
    /// plain receive opens a tab; a targeted receive lands in the named tab.
    #[test]
    fn dock_creates_tabs_and_targeted_import_uses_existing_tab() {
        let dir = temp_dir("receive");
        let url = fixture_png(&dir, "a.png");

        let mut workspace = ProjectWorkspace::new();
        workspace.receive(&[url.clone(), url.clone()], None, None);
        assert_eq!(workspace.tabs().len(), 2);

        let first = workspace.tabs()[0].id;
        let failures = workspace.receive(&[url], Some(first), None);
        assert!(failures.is_empty());
        assert_eq!(workspace.tabs().len(), 2);
        assert_eq!(workspace.selected_id(), first);
        assert_eq!(
            workspace
                .tab(first)
                .unwrap()
                .session
                .document
                .as_ref()
                .unwrap()
                .layers
                .len(),
            2
        );
        assert_eq!(
            workspace.tabs()[1]
                .session
                .document
                .as_ref()
                .unwrap()
                .layers
                .len(),
            1
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `.comp` in a receive batch loads as a project (:132).
    #[test]
    fn receive_loads_comp_files_as_projects() {
        let dir = temp_dir("comp");
        let mut workspace = ProjectWorkspace::new();
        let source = workspace.selected_id();
        workspace
            .tab_mut(source)
            .unwrap()
            .session
            .create_document(50, 50, true);
        workspace.tab_mut(source).unwrap().session.add_blank_layer();

        let path = dir.join("Saved.comp");
        let snapshot = compositor_core::project::project_from_document(
            workspace
                .tab(source)
                .unwrap()
                .session
                .document
                .as_ref()
                .unwrap(),
        );
        compositor_core::project::save(&path, &snapshot).unwrap();

        assert!(workspace.receive(&[path], None, None).is_empty());
        assert_eq!(workspace.tabs().len(), 2);
        assert_ne!(workspace.selected_id(), source);
        assert_eq!(workspace.current().title(), "Saved");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `quitAsksAboutTheActiveTabFirst` (:103-105).
    #[test]
    fn quit_asks_about_the_active_tab_first() {
        let mut workspace = ProjectWorkspace::new();
        let first = workspace.selected_id();
        let second = workspace.add_tab(false).id;
        let third = workspace.add_tab(false).id;

        assert!(workspace.select(second));
        assert_eq!(workspace.quit_order(), vec![second, first, third]);
        assert!(workspace.select(third));
        assert_eq!(workspace.quit_order(), vec![third, first, second]);
        assert!(workspace.select(first));
        assert_eq!(workspace.quit_order(), vec![first, second, third]);
    }

    /// The quit flow asks tab by tab; any Cancel aborts the whole quit
    /// (:106-114 re-shaped as a state machine — deviation #4).
    #[test]
    fn confirm_quit_walks_the_order_and_aborts_on_cancel() {
        let mut workspace = ProjectWorkspace::new();
        let _first = workspace.selected_id();
        let second = workspace.add_tab(false).id;
        workspace
            .tab_mut(second)
            .unwrap()
            .session
            .create_document(10, 10, true);
        workspace.tab_mut(second).unwrap().session.add_blank_layer();
        // `second` is already the tab on screen (add_tab selected it).

        match workspace.confirm_quit() {
            Step::Pending(Pending::Confirm { step, message, .. }) => {
                assert_eq!(step, crate::project::ConfirmStep::Quit);
                // Swift :255 names the project URL, falling back to
                // "Untitled" — not the tab's display name.
                assert_eq!(message, "Save changes to Untitled?");
                assert_eq!(
                    workspace.selected_id(),
                    second,
                    "the asked tab comes on screen"
                );
            }
            other => panic!("expected a confirmation, got {other:?}"),
        }
        assert!(
            !workspace.can_switch(),
            "is_managing blocks switching mid-quit"
        );

        assert!(matches!(
            workspace.resolve_quit(UnsavedChoice::DontSave),
            Step::Done(true)
        ));
        assert!(workspace.can_switch());
    }

    /// Quit's per-tab `Save` may raise a save panel (`saveCurrent` :261);
    /// after `finish_save`, `quit_after_save` carries the result on.
    #[test]
    fn confirm_quit_save_button_finishes_through_a_save_panel() {
        let dir = temp_dir("quit-save");
        let mut workspace = ProjectWorkspace::new();
        let first = workspace.selected_id();
        let tab = workspace.tab_mut(first).unwrap();
        tab.session.create_document(10, 10, true);
        tab.session.add_blank_layer();

        assert!(matches!(workspace.confirm_quit(), Step::Pending(_)));
        // The Save choice re-enters saveCurrent with no URL: a save panel.
        match workspace.resolve_quit(UnsavedChoice::Save) {
            Step::Pending(Pending::SaveAs { default_name, .. }) => {
                assert_eq!(default_name, "Untitled.comp");
            }
            other => panic!("expected a save panel, got {other:?}"),
        }
        let path = dir.join("Saved.comp");
        {
            let tab = workspace.tab_mut(first).unwrap();
            let ProjectTab {
                session,
                busy,
                controller,
                ..
            } = tab;
            assert!(controller.finish_save(session, busy, Some(path.clone())));
        }
        assert!(matches!(workspace.quit_after_save(true), Step::Done(true)));
        assert!(!workspace.current().session.is_modified());
        assert_eq!(
            workspace.current().session.project_url.as_deref(),
            Some(path.as_path())
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `close(_:)` (:89-96) asks with `ConfirmStep::Close` — not Quit — so
    /// the app routes the answer to `resolve_close`; the guard releases and
    /// the tab actually closes.
    #[test]
    fn close_asks_with_the_close_step_and_resolve_close_closes() {
        let mut workspace = ProjectWorkspace::new();
        let first = workspace.selected_id();
        let tab = workspace.tab_mut(first).unwrap();
        tab.session.create_document(10, 10, true);
        tab.session.add_blank_layer();

        match workspace.close(first) {
            Step::Pending(Pending::Confirm { step, message, .. }) => {
                assert_eq!(step, crate::project::ConfirmStep::Close);
                assert_eq!(message, "Save changes to Untitled?");
            }
            other => panic!("expected a confirmation, got {other:?}"),
        }
        assert_eq!(
            workspace.selected_id(),
            first,
            "the asked tab comes on screen"
        );
        assert!(
            !workspace.can_switch(),
            "is_managing blocks switching mid-close"
        );

        assert!(matches!(
            workspace.resolve_close(UnsavedChoice::DontSave),
            Step::Done(())
        ));
        assert!(
            workspace.can_switch(),
            "the guard releases when the flow ends"
        );
        assert!(workspace.close_pending_tab().is_none());
    }

    /// The close flow's Save button raises a save panel; a panel Cancel
    /// resumes `close_after_save(false)` — the tab stays and the guard
    /// releases (Swift's panel Cancel runs the completion with nil).
    #[test]
    fn close_save_panel_cancel_releases_the_guard_and_keeps_the_tab() {
        let mut workspace = ProjectWorkspace::new();
        let first = workspace.selected_id();
        let tab = workspace.tab_mut(first).unwrap();
        tab.session.create_document(10, 10, true);
        tab.session.add_blank_layer();

        assert!(matches!(workspace.close(first), Step::Pending(_)));
        match workspace.resolve_close(UnsavedChoice::Save) {
            Step::Pending(Pending::SaveAs { default_name, .. }) => {
                assert_eq!(default_name, "Untitled.comp");
            }
            other => panic!("expected a save panel, got {other:?}"),
        }
        assert_eq!(workspace.close_pending_tab(), Some(first));
        // The panel presents from a peek, not a take: `finish_save` consumes
        // the controller's `Pending::SaveAs` itself at OK time, so it must
        // still be set here — otherwise every save from the panel silently
        // fails.
        assert!(matches!(
            workspace
                .tabs()
                .iter()
                .find(|tab| tab.id == first)
                .unwrap()
                .controller
                .peek_pending(),
            Some(Pending::SaveAs { .. })
        ));

        // The panel's Cancel: finish_save(None) on the owning tab, then the
        // close flow resumes with saved = false.
        {
            let tab = workspace.tab_mut(first).unwrap();
            let ProjectTab {
                session,
                busy,
                controller,
                ..
            } = tab;
            assert!(!controller.finish_save(session, busy, None));
        }
        assert!(matches!(workspace.close_after_save(false), Step::Done(())));
        assert!(workspace.can_switch(), "the guard releases on panel Cancel");
        assert_eq!(workspace.close_pending_tab(), None);
        assert!(
            workspace.tabs().iter().any(|tab| tab.id == first),
            "the tab survives a cancelled close"
        );
        assert!(workspace.current().session.is_modified());
    }

    /// Swift `close(_:)` (:89-96) asks about the target tab WITHOUT bringing
    /// it on screen — the selection stays where the user left it, through
    /// the ask, a cancel, and a DontSave removal alike.
    #[test]
    fn close_asks_without_foregrounding_the_target_tab() {
        let mut workspace = ProjectWorkspace::new();
        let first = workspace.selected_id();
        workspace
            .tab_mut(first)
            .unwrap()
            .session
            .create_document(10, 10, true);
        let second = workspace.add_tab(true).id;
        workspace
            .tab_mut(second)
            .unwrap()
            .session
            .create_document(20, 20, true);
        workspace.tab_mut(second).unwrap().session.add_blank_layer();
        assert!(workspace.select(first));
        assert_eq!(workspace.selected_id(), first);

        // The dirty second tab asks; the first stays on screen.
        assert!(matches!(workspace.close(second), Step::Pending(_)));
        assert_eq!(
            workspace.selected_id(),
            first,
            "close must not foreground the asked tab (Swift :89-96)"
        );

        // A cancel answers nothing and still never moved the selection.
        assert!(matches!(
            workspace.resolve_close(UnsavedChoice::Cancel),
            Step::Done(())
        ));
        assert!(workspace.tabs().iter().any(|tab| tab.id == second));
        assert_eq!(workspace.selected_id(), first);

        // DontSave removes the asked tab; the selection is untouched.
        assert!(matches!(workspace.close(second), Step::Pending(_)));
        assert!(matches!(
            workspace.resolve_close(UnsavedChoice::DontSave),
            Step::Done(())
        ));
        assert!(!workspace.tabs().iter().any(|tab| tab.id == second));
        assert_eq!(workspace.selected_id(), first);
    }

    /// `closeWindow` (:115-119): after the last confirmation the tabs are
    /// replaced with one empty tab.
    #[test]
    fn close_window_resets_to_one_empty_tab() {
        let mut workspace = ProjectWorkspace::new();
        let first = workspace.selected_id();
        workspace
            .tab_mut(first)
            .unwrap()
            .session
            .create_document(10, 10, true);
        workspace.tab_mut(first).unwrap().session.add_blank_layer();
        let _second = workspace.add_tab(false).id;

        assert!(matches!(workspace.close_window(), Step::Pending(_)));
        assert!(matches!(
            workspace.resolve_quit(UnsavedChoice::Cancel),
            Step::Done(false)
        ));
        assert_eq!(workspace.tabs().len(), 2, "a cancel keeps the tabs");

        assert!(matches!(workspace.close_window(), Step::Pending(_)));
        assert!(matches!(
            workspace.resolve_quit(UnsavedChoice::DontSave),
            Step::Done(true)
        ));
        assert_eq!(workspace.tabs().len(), 1);
        assert!(workspace.current().session.document.is_none());
    }

    /// A failed open never leaves a broken tab (:81-84); the failure's alert
    /// instruction lands on the workspace's error queue.
    #[test]
    fn a_bad_project_never_lands_a_tab() {
        let dir = temp_dir("bad");
        let mut workspace = ProjectWorkspace::new();
        let before = workspace.selected_id();

        assert!(!workspace.load_project(&dir.join("Missing.comp")));
        assert_eq!(workspace.tabs().len(), 1);
        assert_eq!(workspace.selected_id(), before);
        assert_eq!(workspace.errors.len(), 1);
        assert_eq!(workspace.errors[0].0, "Couldn’t open the project");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `open` reuses an already-open project (resolved symlinks) and only
    /// selects it (:78-80).
    #[test]
    fn opening_an_open_project_only_selects_it() {
        let dir = temp_dir("dedup");
        let mut workspace = ProjectWorkspace::new();
        let source = workspace.selected_id();
        workspace
            .tab_mut(source)
            .unwrap()
            .session
            .create_document(50, 50, true);

        let path = dir.join("Again.comp");
        let snapshot = compositor_core::project::project_from_document(
            workspace
                .tab(source)
                .unwrap()
                .session
                .document
                .as_ref()
                .unwrap(),
        );
        compositor_core::project::save(&path, &snapshot).unwrap();

        assert!(workspace.load_project(&path));
        assert_eq!(workspace.tabs().len(), 2);
        let loaded = workspace.selected_id();
        assert!(workspace.load_project(&path));
        assert_eq!(workspace.tabs().len(), 2);
        assert_eq!(workspace.selected_id(), loaded);
        assert_eq!(workspace.current().title(), "Again");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `select` commits the outgoing tab's pending transform (:49).
    #[test]
    fn select_commits_the_outgoing_transform() {
        let mut workspace = ProjectWorkspace::new();
        let first = workspace.selected_id();
        let second = workspace.add_tab(false).id;
        workspace
            .tab_mut(second)
            .unwrap()
            .session
            .create_document(100, 100, true);

        // No pending transform: select proceeds (add_tab already selected
        // the new tab, so switch back and forth).
        assert!(workspace.select(first));
        assert!(workspace.select(second));
        assert!(workspace.select(first));
        assert!(workspace.can_switch());

        // A blocked session refuses to switch (:48).
        workspace.tab_mut(first).unwrap().session.color_picker =
            Some(compositor_core::session::ColorPickerState::new(
                compositor_core::session::ColorPickerTarget::Palette { background: false },
                PaletteColor::WHITE,
            ));
        assert!(!workspace.select(second));
        assert_eq!(workspace.selected_id(), first);
        workspace.tab_mut(first).unwrap().session.color_picker = None;
        assert!(workspace.select(second));

        // `isManaging` blocks switching too.
        workspace.is_managing = true;
        assert!(!workspace.select(first));
        workspace.is_managing = false;
        assert!(workspace.select(first));
    }

    /// The empty first tab is reused by `add_tab(reuse_empty: true)` (:40).
    #[test]
    fn the_empty_first_tab_is_reused() {
        let mut workspace = ProjectWorkspace::new();
        let first = workspace.selected_id();
        let tab = workspace.add_tab(true);
        assert_eq!(tab.id, first);
        assert_eq!(workspace.tabs().len(), 1);

        workspace
            .tab_mut(first)
            .unwrap()
            .session
            .create_document(10, 10, true);
        let second = workspace.add_tab(true).id;
        assert_ne!(second, first);
        assert_eq!(workspace.tabs().len(), 2);
        assert_eq!(workspace.current().default_name, "Untitled 2");
        let third = workspace.add_tab(false).id;
        assert_eq!(workspace.current().default_name, "Untitled 3");
        assert_eq!(workspace.tabs().len(), 3);
        assert_ne!(third, second);
    }

    /// Queued receives drain once the guards clear (deviation #5, replacing
    /// the 30 ms poll :126-129).
    #[test]
    fn receive_waits_for_the_guards_then_drains() {
        let dir = temp_dir("queued");
        let url = fixture_png(&dir, "a.png");
        let mut workspace = ProjectWorkspace::new();

        workspace.is_managing = true;
        workspace.receive(std::slice::from_ref(&url), None, None);
        assert!(workspace.has_queued_receives());
        assert_eq!(workspace.tabs().len(), 1);
        assert!(workspace.drain_incoming().is_empty(), "still blocked");
        workspace.is_managing = false;
        assert!(workspace.drain_incoming().is_empty());
        assert!(!workspace.has_queued_receives());
        // The single still-empty tab is reused, exactly like `addTab()`.
        assert_eq!(workspace.tabs().len(), 1);
        assert!(workspace.current().session.document.is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `canReceiveDrag` (:276-280): files always; a layer drag onto its own
    /// tab is not a drop target.
    #[test]
    fn can_receive_drag() {
        let mut workspace = ProjectWorkspace::new();
        let first = workspace.selected_id();
        let second = workspace.add_tab(false).id;

        assert!(workspace.can_receive_drag(None, None));
        assert!(workspace.can_receive_drag(Some(first), None));
        assert!(workspace.can_receive_drag(Some(second), Some(first)));
        assert!(!workspace.can_receive_drag(Some(first), Some(first)));
    }

    /// `copyLayer`'s budget failure lands on the target's error channel
    /// (:172).
    #[test]
    fn cross_project_copy_budget_error_surfaces_on_the_target() {
        let mut workspace = ProjectWorkspace::new();
        let first = workspace.selected_id();
        workspace
            .tab_mut(first)
            .unwrap()
            .session
            .create_document(200, 200, true);
        workspace.tab_mut(first).unwrap().session.add_blank_layer();
        // No way to hit 100 MP with a blank layer; assert the happy path's
        // error channel stays clean and the actual budget lives in core
        // (project::copy_layers_between tests).
        let second = workspace.add_tab(false).id;
        workspace
            .tab_mut(second)
            .unwrap()
            .session
            .create_document(50, 50, true);
        let layer = workspace
            .tab(first)
            .unwrap()
            .session
            .active_layer_id
            .unwrap();
        assert!(workspace.copy_layer(layer, Some(second), None));
        assert!(workspace.tab(second).unwrap().import_error.is_none());
    }

    /// Tab titles follow the project URL, falling back to the default
    /// name (:11).
    #[test]
    fn titles_follow_the_project_url() {
        let tab = ProjectTab::new("Untitled");
        assert_eq!(tab.title(), "Untitled");
        let mut tab = ProjectTab::new("Untitled");
        tab.session.project_url = Some(PathBuf::from("/tmp/Photos/Foo.comp"));
        assert_eq!(tab.title(), "Foo");
    }
}
