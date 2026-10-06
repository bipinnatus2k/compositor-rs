//! Port of `Compositor/Compositor/IO/ProjectController.swift` (323 lines):
//! the per-tab file lifecycle — open / save / export / canvas size / trim /
//! quit confirmation / the incoming-file import queue.
//!
//! Swift drives these flows with `async` modal panels and sheets. GPUI has no
//! synchronous dialogs, so every awaiting step is a two-phase state machine:
//! the entry method returns [`Step::Pending`] with a [`Pending`] describing
//! the dialog to present (the app owns presentation), stores it on the
//! controller, and the matching `finish_*` / resolution method carries the
//! user's answer into the second half. Guards, the save generation, the
//! import queue and every state transition stay here and are unit tested.
//!
//! The controller never holds its `EditorSession` (Swift keeps a strong
//! reference); session and busy flags are passed in per call so a
//! `ProjectTab` can own both halves without aliasing.

use std::path::{Path, PathBuf};

use compositor_core::canvas::{CanvasSizeOptions, TrimOptions};
use compositor_core::geometry::CGPoint;
use compositor_core::session::EditorSession;

use crate::tabs::ProjectBusy;

/// A tab's identity (`ProjectTab.id`, a `UUID` in Swift). Declared here so
/// `Pending::Confirm` can name the tab being asked about.
pub type TabId = uuid::Uuid;

/// The informative text of the unsaved-changes alert
/// (ProjectController.swift:256).
pub const UNSAVED_CHANGES_INFO: &str = "Your changes will be lost if you don’t save them.";

/// The user's answer to the unsaved-changes alert
/// (`confirmReplacement`'s three buttons, :257-259, in that order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsavedChoice {
    Save,
    Cancel,
    DontSave,
}

/// Which flow a `Pending::Confirm` belongs to, so its resolution resumes the
/// right second half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmStep {
    /// `open(_:)` :210 — confirming may replace the document being opened.
    Open,
    /// `confirmQuit()` :246-250.
    Quit,
    /// `close(_:)` :233-244 (the workspace path forwards here per tab).
    Close,
}

/// A dialog the model asks the app to present (`showError`/panels/sheets of
/// ProjectController.swift). The app presents it and calls the matching
/// `finish_*` method.
#[derive(Debug, Clone)]
pub enum Pending {
    /// The `.comp` save panel of `saveCurrent` (:156-166). `title` is
    /// "Save Project" / "Save Project As"; `default_name` is the panel's
    /// `nameFieldStringValue`.
    SaveAs {
        as_new: bool,
        title: String,
        default_name: String,
    },
    /// The PNG save panel of `exportPNG` (:35-44). The snapshot is captured
    /// before the panel, exactly as Swift captures it before awaiting.
    ExportPng {
        default_name: String,
        snapshot: Option<Box<compositor_core::project::ProjectData>>,
    },
    /// The JPEG save panel of `exportJPEG` (:139-145). The app renders and
    /// encodes through its JPEG sheet first and hands the bytes to
    /// `finish_export_jpeg`.
    ExportJpeg { default_name: String },
    /// An options sheet (`canvasSize` :54-65 / `imageSize` :76-87 /
    /// `trim` :98-109); the app owns the sheet UI.
    Options { kind: OptionsSheet },
    /// The workspace-level multi-select `.comp` open panel (`open(_:)`
    /// :65-71 in ProjectWorkspace.swift); the app calls
    /// `ProjectWorkspace::finish_open` with the chosen files.
    Open,
    /// The unsaved-changes alert of `confirmReplacement` (:252-263).
    Confirm {
        tab: Option<TabId>,
        step: ConfirmStep,
        /// The alert's `messageText`: "Save changes to {name}?".
        message: String,
    },
    /// A warning alert (`showError` :265-272): messageText + informativeText
    /// + OK.
    Error { title: String, message: String },
}

impl Pending {
    /// The save panel's `title` for `saveCurrent` (:161).
    fn save_title(as_new: bool) -> &'static str {
        if as_new {
            "Save Project As"
        } else {
            "Save Project"
        }
    }

    /// `session.projectURL?.lastPathComponent ?? "Untitled.comp"` (:160).
    fn save_default_name(session: &EditorSession) -> String {
        session
            .project_url
            .as_ref()
            .and_then(|url| url.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled.comp".to_string())
    }

    /// `(session.projectURL?.deletingPathExtension().lastPathComponent ??
    /// "Untitled") + extension` (:40, :144).
    fn export_default_name(session: &EditorSession, extension: &str) -> String {
        let stem = session
            .project_url
            .as_ref()
            .and_then(|url| url.file_stem())
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string());
        format!("{stem}.{extension}")
    }
}

/// Which options sheet a `Pending::Options` stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionsSheet {
    CanvasSize,
    ImageSize,
    Trim,
}

/// One step of a two-phase flow: either finished with a value, or asking the
/// user for input. The pending is also stored on the controller so the
/// resolution methods can find it.
#[derive(Debug)]
pub enum Step<T> {
    Done(T),
    Pending(Pending),
}

impl<T> Step<T> {
    /// Unwraps the done value; a pending step counts as the flow having not
    /// proceeded.
    pub fn done(self) -> Option<T> {
        match self {
            Step::Done(value) => Some(value),
            Step::Pending(_) => None,
        }
    }
}

/// One queued batch of dropped/chosen files (`Incoming`, :279-283).
#[derive(Debug, Clone)]
pub struct Incoming {
    pub files: Vec<PathBuf>,
    pub point: Option<CGPoint>,
}

/// What a drain step wants from the app. The queue advances on every call
/// until it needs input or empties (the event-driven shape of
/// `drainIncoming` :302-322; see the workspace's deviation note on polling).
#[derive(Debug)]
pub enum Drain {
    /// The queue is empty; `processing` has been reset (:321).
    Idle,
    /// The session cannot start a file request yet (`waitForFileRequest`
    /// :305); retry when the app's guards clear.
    Wait,
    /// A batch of images imported straight into the session (:314-315).
    Imported { failures: Vec<String> },
    /// Open this `.comp` first, then call `advance(proceed)` with whether the
    /// open succeeded (:311-312).
    OpenProject { url: PathBuf },
    /// A warning alert instruction (:308-309).
    Error { title: String, message: String },
}

/// `ProjectError.invalid`'s user-facing text, the error body of "Open one
/// project at a time" (:309).
const INVALID_PROJECT_MESSAGE: &str = "The file is not a Compositor project, or it is damaged.";

/// The per-tab file lifecycle (`ProjectController`).
pub struct ProjectController {
    save_generation: u32,
    incoming: Vec<Incoming>,
    processing: bool,
    pending: Option<Pending>,
    /// `open(_:)`'s validated snapshot held across the replacement
    /// confirmation (:207-216).
    open_stash: Option<OpenStash>,
    /// The `.comp` a drain step is opening, plus the trailing image files
    /// (:311-317).
    opening: Option<OpenRequest>,
}

struct OpenStash {
    source: PathBuf,
    snapshot: compositor_core::project::ProjectData,
    previous_save: u32,
}

struct OpenRequest {
    images: Vec<PathBuf>,
}

impl Default for ProjectController {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectController {
    pub fn new() -> Self {
        ProjectController {
            save_generation: 0,
            incoming: Vec::new(),
            processing: false,
            pending: None,
            open_stash: None,
            opening: None,
        }
    }

    /// `saveGeneration` (:10): bumped by every successful `saveCurrent`.
    pub fn save_generation(&self) -> u32 {
        self.save_generation
    }

    /// The dialog currently awaiting an answer, if any.
    pub fn pending(&self) -> Option<&Pending> {
        self.pending.as_ref()
    }

    /// Takes the dialog awaiting an answer (the app presents and drains it).
    pub fn take_pending(&mut self) -> Option<Pending> {
        self.pending.take()
    }

    /// The pending without consuming it. The save panel must be presented
    /// from a *peek*: `finish_save` consumes `Pending::SaveAs` itself at
    /// OK time, so a flow that `take_pending`s first makes every save from
    /// its panel silently fail.
    pub fn peek_pending(&self) -> Option<Pending> {
        self.pending.clone()
    }

    /// `canStart` (:11-13): the session half lives in
    /// [`ProjectBusy::ready`]; `is_managing` is the workspace's flag, passed
    /// in because the controller does not hold a workspace reference.
    pub fn can_start(
        &self,
        session: &EditorSession,
        busy: &ProjectBusy,
        is_managing: bool,
    ) -> bool {
        busy.ready(session) && !is_managing
    }

    /// `begin()` (:16-22): guard, cancel a pending crop, commit a pending
    /// transform, and mark the session project-busy. The caller (or the
    /// matching `finish_*`) clears `is_project_busy` when its flow ends.
    pub fn begin(&self, session: &mut EditorSession, busy: &mut ProjectBusy) -> bool {
        if !busy.ready(session) {
            return false;
        }
        session.cancel_crop();
        session.commit_transform();
        busy.is_project_busy = true;
        true
    }

    // MARK: Save (`save(asNew:)` :25-29 + `saveCurrent` :152-182)

    /// `save(asNew:)`. A known destination (existing project URL, not
    /// save-as-new) writes straight to disk without a panel (:154); anything
    /// else surfaces the save panel.
    pub fn save(
        &mut self,
        session: &mut EditorSession,
        busy: &mut ProjectBusy,
        as_new: bool,
    ) -> Step<bool> {
        if session.document.is_none() || !self.begin(session, busy) {
            return Step::Done(false);
        }
        busy.is_project_busy = false;
        self.save_current(session, as_new)
    }

    /// Second half of the save panel. `None` is the panel's Cancel.
    pub fn finish_save(
        &mut self,
        session: &mut EditorSession,
        busy: &mut ProjectBusy,
        url: Option<PathBuf>,
    ) -> bool {
        busy.is_project_busy = false;
        match self.pending.take() {
            Some(Pending::SaveAs { .. }) => {}
            _ => return false,
        }
        let Some(url) = url else { return false };
        let Some(snapshot) = session.document.as_ref().map(project_snapshot) else {
            // Swift :153: no snapshot is not a failure.
            return true;
        };
        self.write_project(session, snapshot, &url)
    }

    /// `saveCurrent(asNew:)` (:152-182).
    fn save_current(&mut self, session: &mut EditorSession, as_new: bool) -> Step<bool> {
        let Some(snapshot) = session.document.as_ref().map(project_snapshot) else {
            return Step::Done(true);
        };
        let destination = if as_new {
            None
        } else {
            session.project_url.clone()
        };
        match destination {
            Some(destination) => Step::Done(self.write_project(session, snapshot, &destination)),
            None => {
                let pending = Pending::SaveAs {
                    as_new,
                    title: Pending::save_title(as_new).to_string(),
                    default_name: Pending::save_default_name(session),
                };
                self.pending = Some(pending.clone());
                Step::Pending(pending)
            }
        }
    }

    /// The write half of `saveCurrent` (:171-181): save the package, update
    /// `projectURL`, `markSaved`, bump the generation. (The recent-documents
    /// note `NSDocumentController.noteNewRecentDocumentURL` is platform-only
    /// and not ported — the workspace design's deviation #8.)
    fn write_project(
        &mut self,
        session: &mut EditorSession,
        snapshot: compositor_core::project::ProjectData,
        destination: &Path,
    ) -> bool {
        match compositor_core::project::save(destination, &snapshot) {
            Ok(()) => {
                session.project_url = Some(destination.to_path_buf());
                session.mark_saved();
                self.save_generation += 1;
                true
            }
            Err(error) => {
                self.pending = Some(Pending::Error {
                    title: "Couldn’t save the project".to_string(),
                    message: error.to_string(),
                });
                false
            }
        }
    }

    // MARK: Export (`exportPNG` :31-49, `exportJPEG` :119-151)

    /// `exportPNG`: capture the snapshot, then surface the `.png` save panel.
    /// The session stays project-busy until `finish_export_png` (Swift's
    /// `defer` runs after the panel completes).
    pub fn export_png(&mut self, session: &mut EditorSession, busy: &mut ProjectBusy) -> Step<()> {
        if session.document.is_none() || !self.begin(session, busy) {
            return Step::Done(());
        }
        let snapshot = session
            .document
            .as_ref()
            .map(|d| Box::new(project_snapshot(d)));
        let pending = Pending::ExportPng {
            default_name: Pending::export_default_name(session, "png"),
            snapshot,
        };
        self.pending = Some(pending.clone());
        Step::Pending(pending)
    }

    /// Second half of the PNG panel. `None` is Cancel. The snapshot captured
    /// at `export_png` time is rendered and written (:47).
    pub fn finish_export_png(&mut self, busy: &mut ProjectBusy, url: Option<&Path>) -> bool {
        busy.is_project_busy = false;
        let Some(Pending::ExportPng { snapshot, .. }) = self.pending.take() else {
            return false;
        };
        let (Some(url), Some(snapshot)) = (url, snapshot) else {
            return false;
        };
        match export_png_bytes(&snapshot) {
            Ok(bytes) => match std::fs::write(url, bytes) {
                Ok(()) => true,
                Err(error) => {
                    self.pending = Some(Pending::Error {
                        title: "Couldn’t export PNG".to_string(),
                        message: error.to_string(),
                    });
                    false
                }
            },
            Err(message) => {
                self.pending = Some(Pending::Error {
                    title: "Couldn’t export PNG".to_string(),
                    message,
                });
                false
            }
        }
    }

    /// `exportJPEG` (:119-138): the app's JPEG sheet renders, offers the
    /// quality/matte controls, and produces the encoded bytes; the model
    /// supplies the save panel step.
    pub fn export_jpeg(&mut self, session: &mut EditorSession, busy: &mut ProjectBusy) -> Step<()> {
        if session.document.is_none() || !self.begin(session, busy) {
            return Step::Done(());
        }
        let pending = Pending::ExportJpeg {
            default_name: Pending::export_default_name(session, "jpg"),
        };
        self.pending = Some(pending.clone());
        Step::Pending(pending)
    }

    /// Second half of the JPEG save panel: write the sheet's bytes (:148).
    /// `None` is Cancel.
    pub fn finish_export_jpeg(
        &mut self,
        busy: &mut ProjectBusy,
        url: Option<&Path>,
        data: &[u8],
    ) -> bool {
        busy.is_project_busy = false;
        let Some(Pending::ExportJpeg { .. }) = self.pending.take() else {
            return false;
        };
        let Some(url) = url else { return false };
        match std::fs::write(url, data) {
            Ok(()) => true,
            Err(error) => {
                self.pending = Some(Pending::Error {
                    title: "Couldn’t export JPEG".to_string(),
                    message: error.to_string(),
                });
                false
            }
        }
    }

    // MARK: Canvas size / image size / trim (:51-117)

    /// `canvasSize()`: guard + begin, then surface the Canvas Size sheet. The
    /// session stays project-busy until `finish_canvas_size`.
    pub fn canvas_size(&mut self, session: &mut EditorSession, busy: &mut ProjectBusy) -> Step<()> {
        if session.document.is_none() || !self.begin(session, busy) {
            return Step::Done(());
        }
        let pending = Pending::Options {
            kind: OptionsSheet::CanvasSize,
        };
        self.pending = Some(pending.clone());
        Step::Pending(pending)
    }

    /// Second half of the Canvas Size sheet. `None` is Cancel (:66).
    pub fn finish_canvas_size(
        &mut self,
        session: &mut EditorSession,
        busy: &mut ProjectBusy,
        options: Option<&CanvasSizeOptions>,
    ) {
        busy.is_project_busy = false;
        if !matches!(self.pending.take(), Some(Pending::Options { .. })) {
            return;
        }
        let Some(options) = options else { return };
        if !session.resize_canvas_to(options) {
            self.pending = Some(Pending::Error {
                title: "Couldn’t change canvas size".to_string(),
                message: "The canvas could not be resized.".to_string(),
            });
        }
    }

    /// `imageSize()`: guard + begin, then surface the Image Size sheet.
    pub fn image_size(&mut self, session: &mut EditorSession, busy: &mut ProjectBusy) -> Step<()> {
        if session.document.is_none() || !self.begin(session, busy) {
            return Step::Done(());
        }
        let pending = Pending::Options {
            kind: OptionsSheet::ImageSize,
        };
        self.pending = Some(pending.clone());
        Step::Pending(pending)
    }

    /// Second half of the Image Size sheet. `None` is Cancel (:88).
    pub fn finish_image_size(
        &mut self,
        session: &mut EditorSession,
        busy: &mut ProjectBusy,
        width: Option<usize>,
        height: Option<usize>,
        resolution: Option<f64>,
        sampling: compositor_core::transform::LayerSampling,
    ) {
        busy.is_project_busy = false;
        if !matches!(self.pending.take(), Some(Pending::Options { .. })) {
            return;
        }
        let (Some(width), Some(height)) = (width, height) else {
            return;
        };
        if !session.resize_image_to(width, height, resolution, sampling) {
            self.pending = Some(Pending::Error {
                title: "Couldn’t resize the image".to_string(),
                message: "The image could not be resized.".to_string(),
            });
        }
    }

    /// `trim()`: guard + begin, then surface the Trim sheet.
    pub fn trim(&mut self, session: &mut EditorSession, busy: &mut ProjectBusy) -> Step<()> {
        if session.document.is_none() || !self.begin(session, busy) {
            return Step::Done(());
        }
        let pending = Pending::Options {
            kind: OptionsSheet::Trim,
        };
        self.pending = Some(pending.clone());
        Step::Pending(pending)
    }

    /// Second half of the Trim sheet. `None` is Cancel (:110); trimming
    /// nothing stays silent (:112-114).
    pub fn finish_trim(
        &mut self,
        session: &mut EditorSession,
        busy: &mut ProjectBusy,
        options: Option<&TrimOptions>,
    ) {
        busy.is_project_busy = false;
        if !matches!(self.pending.take(), Some(Pending::Options { .. })) {
            return;
        }
        let Some(options) = options else { return };
        session.trim(options);
    }

    // MARK: Open (`open(_:)` :185-223)

    /// `open(_:)`'s direct path, which `ProjectWorkspace::load_project`
    /// drives for a fresh tab. Validation happens first (:207-208): a corrupt
    /// project never discards the live document. The replacement
    /// confirmation may need an alert — see `resolve_confirm`.
    pub fn open(
        &mut self,
        session: &mut EditorSession,
        busy: &mut ProjectBusy,
        source: &Path,
    ) -> Step<bool> {
        if !self.begin(session, busy) {
            return Step::Done(false);
        }
        busy.is_project_busy = false;
        let snapshot = match compositor_core::project::load(source) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.pending = Some(Pending::Error {
                    title: "Couldn’t open the project".to_string(),
                    message: error.to_string(),
                });
                return Step::Done(false);
            }
        };
        let previous_save = self.save_generation;
        match Self::request_unsaved_choice(session) {
            None => Step::Done(self.install_open(session, snapshot, source, previous_save)),
            Some(message) => {
                self.open_stash = Some(OpenStash {
                    source: source.to_path_buf(),
                    snapshot,
                    previous_save,
                });
                let pending = Pending::Confirm {
                    tab: None,
                    step: ConfirmStep::Open,
                    message,
                };
                self.pending = Some(pending.clone());
                Step::Pending(pending)
            }
        }
    }

    /// The tail of `open(_:)` (:211-217): saving inside the confirmation can
    /// replace the very file being opened, so a bumped generation on the same
    /// path reloads the snapshot before installing.
    fn install_open(
        &mut self,
        session: &mut EditorSession,
        snapshot: compositor_core::project::ProjectData,
        source: &Path,
        previous_save: u32,
    ) -> bool {
        let snapshot = if self.save_generation != previous_save && same_path(session, source) {
            match compositor_core::project::load(source) {
                Ok(reloaded) => reloaded,
                Err(error) => {
                    self.pending = Some(Pending::Error {
                        title: "Couldn’t open the project".to_string(),
                        message: error.to_string(),
                    });
                    return false;
                }
            }
        } else {
            snapshot
        };
        match session.install_project(&snapshot, source) {
            Ok(()) => true,
            Err(error) => {
                self.pending = Some(Pending::Error {
                    title: "Couldn’t open the project".to_string(),
                    message: error.to_string(),
                });
                false
            }
        }
    }

    // MARK: Quit / close / replacement confirmation (:246-272)

    /// `confirmQuit()` (:246-250): begin, then the unsaved-changes question.
    pub fn confirm_quit(
        &mut self,
        session: &mut EditorSession,
        busy: &mut ProjectBusy,
    ) -> Step<bool> {
        self.confirm_step(session, busy, ConfirmStep::Quit)
    }

    /// The close-tab flow's half of `confirmQuit` — the same question, but
    /// the alert carries `ConfirmStep::Close` so the app routes the answer
    /// back into the close flow (`close(_:)` :233-244 forwards here per tab).
    pub fn confirm_close(
        &mut self,
        session: &mut EditorSession,
        busy: &mut ProjectBusy,
    ) -> Step<bool> {
        self.confirm_step(session, busy, ConfirmStep::Close)
    }

    fn confirm_step(
        &mut self,
        session: &mut EditorSession,
        busy: &mut ProjectBusy,
        step: ConfirmStep,
    ) -> Step<bool> {
        if !self.begin(session, busy) {
            return Step::Done(false);
        }
        busy.is_project_busy = false;
        match Self::request_unsaved_choice(session) {
            None => Step::Done(true),
            Some(message) => {
                let pending = Pending::Confirm {
                    tab: None,
                    step,
                    message,
                };
                self.pending = Some(pending.clone());
                Step::Pending(pending)
            }
        }
    }

    /// `confirmReplacement` (:252-263): `None` proceeds without asking;
    /// `Some` is the alert's `messageText`
    /// ("Save changes to {projectURL lastPathComponent ?? Untitled}?").
    pub fn request_unsaved_choice(session: &EditorSession) -> Option<String> {
        if !session.is_modified() || session.document.is_none() {
            return None;
        }
        let name = session
            .project_url
            .as_ref()
            .and_then(|url| url.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string());
        Some(format!("Save changes to {name}?"))
    }

    /// The alert's answer. `Save` re-enters `saveCurrent` (:261), which may
    /// itself surface a save panel — after `finish_save` completes that
    /// panel, call [`ProjectController::confirm_after_save`] to carry the
    /// result through. `Cancel` declines; `Don't Save` proceeds (:262).
    pub fn resolve_confirm(
        &mut self,
        session: &mut EditorSession,
        choice: UnsavedChoice,
    ) -> Step<bool> {
        let step = match self.pending.take() {
            Some(Pending::Confirm { step, .. }) => step,
            _ => return Step::Done(false),
        };
        match choice {
            UnsavedChoice::Cancel => {
                self.open_stash = None;
                Step::Done(false)
            }
            UnsavedChoice::DontSave => {
                // The open flow continues: the validated snapshot installs
                // (:210 + :216).
                match step {
                    ConfirmStep::Open => {
                        let Some(stash) = self.open_stash.take() else {
                            return Step::Done(false);
                        };
                        Step::Done(self.install_open(
                            session,
                            stash.snapshot,
                            &stash.source,
                            stash.previous_save,
                        ))
                    }
                    _ => Step::Done(true),
                }
            }
            UnsavedChoice::Save => match self.save_current(session, false) {
                Step::Done(true) => {
                    if step == ConfirmStep::Open {
                        let Some(stash) = self.open_stash.take() else {
                            return Step::Done(false);
                        };
                        Step::Done(self.install_open(
                            session,
                            stash.snapshot,
                            &stash.source,
                            stash.previous_save,
                        ))
                    } else {
                        Step::Done(true)
                    }
                }
                Step::Done(false) => Step::Done(false),
                Step::Pending(pending) => Step::Pending(pending),
            },
        }
    }

    /// After a `finish_save` that came out of a confirmation's `Save`
    /// button: its result is the confirmation's result. The `Open` step still
    /// owes the generation check + install.
    pub fn confirm_after_save(&mut self, session: &mut EditorSession, saved: bool) -> Step<bool> {
        // The confirmation's pending slot was consumed by `finish_save`; the
        // `Open` stash (if any) says a reload + install is still owed.
        if !saved {
            self.open_stash = None;
            return Step::Done(false);
        }
        match self.open_stash.take() {
            Some(stash) => Step::Done(self.install_open(
                session,
                stash.snapshot,
                &stash.source,
                stash.previous_save,
            )),
            None => Step::Done(true),
        }
    }

    /// `showError(_:error:)` (:265-272): a warning alert instruction the app
    /// presents (messageText + informativeText + OK).
    pub fn show_error(&mut self, title: impl Into<String>, message: impl Into<String>) {
        self.pending = Some(Pending::Error {
            title: title.into(),
            message: message.into(),
        });
    }

    // MARK: Incoming queue (`receive` :287-300, `drainIncoming` :302-322)

    /// `receive(_:at:)`: enqueue a batch. Draining is event-driven (the
    /// workspace design's deviation #5): the app calls `drain` whenever the
    /// session can accept a file request.
    pub fn receive(&mut self, urls: Vec<PathBuf>, at: Option<CGPoint>) {
        if urls.is_empty() {
            return;
        }
        self.incoming.push(Incoming {
            files: urls,
            point: at,
        });
        self.processing = true;
    }

    pub fn has_incoming(&self) -> bool {
        !self.incoming.is_empty()
    }

    pub fn is_processing(&self) -> bool {
        self.processing
    }

    /// `drainIncoming`, one step at a time. Image batches import straight
    /// into the session; a `.comp` becomes [`Drain::OpenProject`] — the
    /// caller performs the open (the workspace routes it to
    /// `load_project`) and calls `advance`. More than one `.comp` in a batch
    /// is an error (:307-309).
    pub fn drain(&mut self, session: &mut EditorSession, busy: &ProjectBusy) -> Drain {
        if self.incoming.is_empty() {
            self.processing = false;
            return Drain::Idle;
        }
        // `waitForFileRequest` (:305): nothing proceeds while the session is
        // mid-operation.
        if !busy.ready(session) {
            return Drain::Wait;
        }
        let request = self.incoming.remove(0);
        let projects: Vec<&PathBuf> = request
            .files
            .iter()
            .filter(|url| is_project_file(url))
            .collect();
        if projects.len() > 1 {
            return Drain::Error {
                title: "Open one project at a time".to_string(),
                message: INVALID_PROJECT_MESSAGE.to_string(),
            };
        }
        if let Some(project) = projects.first() {
            self.opening = Some(OpenRequest {
                images: request
                    .files
                    .iter()
                    .filter(|url| !is_project_file(url))
                    .cloned()
                    .collect(),
            });
            return Drain::OpenProject {
                url: (*project).clone(),
            };
        }
        let failures = session.import_images(&request.files, request.point);
        if self.incoming.is_empty() {
            self.processing = false;
        }
        Drain::Imported { failures }
    }

    /// Continues after a `Drain::OpenProject`: when the open proceeded, the
    /// batch's image files import with no drop point (:313-315 — a project is
    /// present, so `at` is dropped). Then draining continues.
    pub fn advance(
        &mut self,
        session: &mut EditorSession,
        busy: &ProjectBusy,
        proceed: bool,
    ) -> Drain {
        if let Some(request) = self.opening.take()
            && proceed
        {
            let failures = session.import_images(&request.images, None);
            if self.incoming.is_empty() {
                self.processing = false;
            }
            // The import is its own reportable step; the next `drain`
            // continues with the rest of the queue.
            return Drain::Imported { failures };
        }
        self.drain(session, busy)
    }
}

/// `session.projectSnapshot()` (EditorSession+Projects.swift:4): a validated
/// snapshot of the live document.
fn project_snapshot(
    document: &compositor_core::document::CanvasDocument,
) -> compositor_core::project::ProjectData {
    compositor_core::project::project_from_document(document)
}

/// Renders a snapshot and encodes PNG bytes (`ImageExporter.exportPNG`).
fn export_png_bytes(snapshot: &compositor_core::project::ProjectData) -> Result<Vec<u8>, String> {
    let (document, _) = compositor_core::project::document_from_project(snapshot)
        .map_err(|error| error.to_string())?;
    let composite = compositor_core::render::draw_live_composite(&document);
    compositor_core::export::png_data(&composite)
        .ok_or_else(|| "The image could not be encoded as PNG.".to_string())
}

/// `$0.pathExtension.lowercased() == "comp"` (:132, :307).
pub fn is_project_file(url: &Path) -> bool {
    url.extension()
        .map(|extension| extension.to_string_lossy().to_lowercase() == "comp")
        .unwrap_or(false)
}

/// `resolvingSymlinksInPath` equality for the open-reload check (:212-213).
fn same_path(session: &EditorSession, source: &Path) -> bool {
    let canonical =
        |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    match session.project_url.as_ref() {
        Some(project_url) => canonical(project_url) == canonical(source),
        None => false,
    }
}

// MARK: Tests

#[cfg(test)]
mod tests {
    use super::*;
    use compositor_core::raster::Bitmap;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "compositor-controller-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_document(session: &mut EditorSession) {
        session.create_document(10, 10, true);
    }

    /// `save` without a project URL raises the save panel; its completion
    /// writes the package, sets `projectURL`, marks saved, bumps the
    /// generation (:152-182).
    #[test]
    fn save_without_url_asks_then_writes_and_bumps_generation() {
        let dir = temp_dir("save");
        let path = dir.join("Foo.comp");
        let mut session = EditorSession::new();
        make_document(&mut session);
        let mut busy = ProjectBusy::new();
        let mut controller = ProjectController::new();

        match controller.save(&mut session, &mut busy, false) {
            Step::Pending(Pending::SaveAs {
                title,
                default_name,
                ..
            }) => {
                assert_eq!(title, "Save Project");
                assert_eq!(default_name, "Untitled.comp");
            }
            other => panic!("expected a save panel, got {other:?}"),
        }

        assert!(controller.finish_save(&mut session, &mut busy, Some(path.clone())));
        assert_eq!(controller.save_generation(), 1);
        assert_eq!(session.project_url.as_deref(), Some(path.as_path()));
        assert!(!session.is_modified());

        // Saving again goes straight to disk (:154): no panel, generation +1.
        assert!(matches!(
            controller.save(&mut session, &mut busy, false),
            Step::Done(true)
        ));
        assert_eq!(controller.save_generation(), 2);

        // Save As raises the panel again, seeded with the current file name
        // (:160-161), and Cancel writes nothing.
        match controller.save(&mut session, &mut busy, true) {
            Step::Pending(Pending::SaveAs {
                title,
                default_name,
                ..
            }) => {
                assert_eq!(title, "Save Project As");
                assert_eq!(default_name, "Foo.comp");
            }
            other => panic!("expected a save panel, got {other:?}"),
        }
        assert!(!controller.finish_save(&mut session, &mut busy, None));
        assert_eq!(controller.save_generation(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `begin` refuses while the app-side busy flags block the session
    /// (`canStartProjectOperation`).
    #[test]
    fn begin_refuses_while_busy() {
        let mut session = EditorSession::new();
        make_document(&mut session);
        let mut busy = ProjectBusy::new();
        let mut controller = ProjectController::new();

        busy.has_brush_stroke = true;
        assert!(matches!(
            controller.save(&mut session, &mut busy, false),
            Step::Done(false)
        ));
        busy.has_brush_stroke = false;

        busy.is_importing = true;
        assert!(!controller.begin(&mut session, &mut busy));
        busy.is_importing = false;

        session.shows_new_document = true;
        assert!(!controller.begin(&mut session, &mut busy));
        session.shows_new_document = false;

        assert!(controller.begin(&mut session, &mut busy));
        assert!(busy.is_project_busy);
    }

    /// `confirmReplacement` (:252-263): unmodified proceeds; modified asks
    /// with the exact text; Save/Cancel/Don't-Save resolve accordingly.
    #[test]
    fn confirm_quit_flow_and_texts() {
        let dir = temp_dir("quit");
        let path = dir.join("Bar.comp");
        let mut session = EditorSession::new();
        let mut busy = ProjectBusy::new();
        let mut controller = ProjectController::new();

        // No document: no question (:253).
        assert!(matches!(
            controller.confirm_quit(&mut session, &mut busy),
            Step::Done(true)
        ));

        // A fresh document counts as modified; ask with the exact text.
        make_document(&mut session);
        assert!(session.is_modified());
        session.project_url = Some(path.clone());

        match controller.confirm_quit(&mut session, &mut busy) {
            Step::Pending(Pending::Confirm { message, .. }) => {
                assert_eq!(message, "Save changes to Bar.comp?");
            }
            other => panic!("expected a confirmation, got {other:?}"),
        }
        assert!(matches!(
            controller.resolve_confirm(&mut session, UnsavedChoice::Cancel),
            Step::Done(false)
        ));
        assert!(session.is_modified());

        assert!(matches!(
            controller.confirm_quit(&mut session, &mut busy),
            Step::Pending(_)
        ));
        assert!(matches!(
            controller.resolve_confirm(&mut session, UnsavedChoice::DontSave),
            Step::Done(true)
        ));

        // The Save button writes through `saveCurrent` — direct, since the
        // URL is known (:261).
        session.add_blank_layer();
        assert!(matches!(
            controller.confirm_quit(&mut session, &mut busy),
            Step::Pending(_)
        ));
        assert!(matches!(
            controller.resolve_confirm(&mut session, UnsavedChoice::Save),
            Step::Done(true)
        ));
        assert!(!session.is_modified());
        assert_eq!(controller.save_generation(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `open(_:)` validates before asking (:207-210), and saving inside the
    /// confirmation — which can replace the very file being opened —
    /// reloads before installing (:211-216).
    #[test]
    fn open_reloads_when_confirmation_saves_over_the_file() {
        let dir = temp_dir("open");
        let path = dir.join("Baz.comp");
        let mut session = EditorSession::new();
        make_document(&mut session);
        let mut busy = ProjectBusy::new();
        let mut controller = ProjectController::new();

        // Establish the on-disk project with one layer.
        assert!(matches!(
            controller.save(&mut session, &mut busy, false),
            Step::Pending(_)
        ));
        assert!(controller.finish_save(&mut session, &mut busy, Some(path.clone())));

        // A corrupt project fails without touching the live document.
        session.add_blank_layer();
        assert!(matches!(
            controller.open(&mut session, &mut busy, &dir.join("Nope.comp")),
            Step::Done(false)
        ));
        assert!(matches!(
            controller.pending(),
            Some(Pending::Error { title, .. }) if title == "Couldn’t open the project"
        ));
        assert_eq!(session.document.as_ref().unwrap().layers.len(), 2);

        // Open the same file while modified: the snapshot loaded first has
        // one layer; Save overwrites the file (generation bumps), so the
        // reload branch installs the two-layer document.
        let previous = controller.save_generation();
        match controller.open(&mut session, &mut busy, &path) {
            Step::Pending(Pending::Confirm { message, .. }) => {
                assert_eq!(message, "Save changes to Baz.comp?");
            }
            other => panic!("expected a confirmation, got {other:?}"),
        }
        match controller.resolve_confirm(&mut session, UnsavedChoice::Save) {
            Step::Done(true) => {}
            other => panic!("expected the open to complete, got {other:?}"),
        }
        assert!(controller.save_generation() > previous);
        assert_eq!(session.document.as_ref().unwrap().layers.len(), 2);
        assert!(!session.is_modified());
        assert_eq!(session.project_url.as_deref(), Some(path.as_path()));

        // Don't Save installs the pre-confirmation on-disk snapshot (two
        // layers as of the Save above).
        session.add_blank_layer();
        assert!(matches!(
            controller.open(&mut session, &mut busy, &path),
            Step::Pending(_)
        ));
        assert!(matches!(
            controller.resolve_confirm(&mut session, UnsavedChoice::DontSave),
            Step::Done(true)
        ));
        assert_eq!(session.document.as_ref().unwrap().layers.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `drainIncoming` (:302-322): images import in order; more than one
    /// `.comp` in a batch is an error; a single `.comp` opens first and the
    /// trailing images import with no drop point.
    #[test]
    fn incoming_queue_drains() {
        let dir = temp_dir("drain");
        let mut session = EditorSession::new();
        let mut busy = ProjectBusy::new();
        let mut controller = ProjectController::new();

        // Write two tiny PNGs.
        let png = |name: &str| {
            let path = dir.join(name);
            let bytes =
                compositor_core::export::png_data(&Bitmap::new_clearing(4, 4, [255, 0, 0, 255]))
                    .unwrap();
            std::fs::write(&path, bytes).unwrap();
            path
        };
        let a = png("a.png");
        let b = png("b.png");

        controller.receive(vec![a.clone(), b.clone()], None);
        assert!(controller.is_processing());
        match controller.drain(&mut session, &busy) {
            Drain::Imported { failures } => assert!(failures.is_empty()),
            other => panic!("expected an import, got {other:?}"),
        }
        assert!(!controller.is_processing());
        assert!(matches!(controller.drain(&mut session, &busy), Drain::Idle));

        // Two projects in one batch: the "one at a time" alert (:308-309).
        let project = dir.join("One.comp");
        controller.receive(vec![project.clone(), dir.join("Two.comp")], None);
        match controller.drain(&mut session, &busy) {
            Drain::Error { title, .. } => assert_eq!(title, "Open one project at a time"),
            other => panic!("expected an error, got {other:?}"),
        }
        // The failed batch is consumed; the queue is otherwise empty.
        assert!(!controller.has_incoming());

        // One project: the caller opens it, then `advance` imports the
        // images without a drop point (:313-315).
        controller.receive(
            vec![a.clone(), project.clone(), b.clone()],
            Some(CGPoint::new(1.0, 2.0)),
        );
        match controller.drain(&mut session, &busy) {
            Drain::OpenProject { url } => assert_eq!(url, project),
            other => panic!("expected an open step, got {other:?}"),
        }
        match controller.advance(&mut session, &busy, true) {
            Drain::Imported { failures } => assert!(failures.is_empty()),
            other => panic!("expected the trailing import, got {other:?}"),
        }
        assert!(matches!(controller.drain(&mut session, &busy), Drain::Idle));

        // A declined open skips the trailing images.
        controller.receive(vec![project.clone(), a.clone()], None);
        assert!(matches!(
            controller.drain(&mut session, &busy),
            Drain::OpenProject { .. }
        ));
        assert!(matches!(
            controller.advance(&mut session, &busy, false),
            Drain::Idle
        ));

        // A busy session parks the queue (:305).
        busy.is_importing = true;
        controller.receive(vec![a.clone()], None);
        assert!(matches!(controller.drain(&mut session, &busy), Drain::Wait));
        busy.is_importing = false;
        assert!(matches!(
            controller.drain(&mut session, &busy),
            Drain::Imported { .. }
        ));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `exportPNG` (:31-49) writes the captured snapshot to the chosen URL.
    #[test]
    fn export_png_writes_the_snapshot() {
        let dir = temp_dir("png");
        let mut session = EditorSession::new();
        make_document(&mut session);
        let mut busy = ProjectBusy::new();
        let mut controller = ProjectController::new();

        match controller.export_png(&mut session, &mut busy) {
            Step::Pending(Pending::ExportPng { default_name, .. }) => {
                assert_eq!(default_name, "Untitled.png");
            }
            other => panic!("expected a png panel, got {other:?}"),
        }
        let url = dir.join("Untitled.png");
        assert!(controller.finish_export_png(&mut busy, Some(&url)));
        assert!(std::fs::metadata(&url).unwrap().len() > 0);
        // Cancel leaves nothing written.
        assert!(matches!(
            controller.export_png(&mut session, &mut busy),
            Step::Pending(_)
        ));
        assert!(!controller.finish_export_png(&mut busy, None));

        std::fs::remove_dir_all(&dir).ok();
    }
}
