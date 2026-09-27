//! The app-side half of the two-phase dialog state machine (design doc §8):
//! every [`Pending`] the model raises is presented as a sheet here, and the
//! sheet's OK/Cancel (or the unsaved-changes Confirm's three buttons)
//! resolves it through the owning tab's `ProjectController` or the
//! workspace. Errors the model raises surface as the Alert sheet; failed
//! opens surface from `workspace.errors`.

use std::path::PathBuf;

use gpui_kit::{px, rgb, Context, IntoElement, Styled, Window};

use crate::project::{ConfirmStep, Pending, Step, TabId, UnsavedChoice};
use crate::state::{AppState, SaveAfter, SheetKind};
use compositor_core::transform::LayerSampling;
use crate::tabs::ProjectTab;

impl AppState {
    /// Runs a closure with the current tab's (controller, session, busy)
    /// triple — the shape every controller entry point takes.
    pub(crate) fn with_current_tab<R>(
        &mut self,
        f: impl FnOnce(&mut crate::project::ProjectController, &mut compositor_core::session::EditorSession, &mut crate::tabs::ProjectBusy) -> R,
    ) -> R {
        let ProjectTab {
            session,
            busy,
            controller,
            ..
        } = self.workspace.current_mut();
        f(controller, session, busy)
    }

    /// Opens a path-seeded sheet (the save/export/open panels the model
    /// asked for, with the panel's `nameFieldStringValue` in the field).
    fn open_path_sheet(
        &mut self,
        kind: SheetKind,
        default_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_sheet(kind, window, cx);
        let value = default_name.to_string();
        if let Some(sheet) = self.sheet.as_mut() {
            let path = sheet.path.clone();
            path.update(cx, |input, cx| input.set_value(value, window, cx));
        }
    }

    /// Opens the Alert sheet (a `Pending::Error`: title + message + OK).
    pub(crate) fn open_alert(
        &mut self,
        title: &str,
        message: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_sheet(SheetKind::Alert, window, cx);
        if let Some(sheet) = self.sheet.as_mut() {
            sheet.message = format!("{title}\n{message}");
        }
    }

    /// Presents a dialog the model asked for. Save panels record the
    /// resumption context first (callers set `pending_save` before calling
    /// for `SaveAs`).
    pub fn present_pending(
        &mut self,
        pending: Pending,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match pending {
            Pending::SaveAs { default_name, .. } => {
                self.open_path_sheet(SheetKind::SaveComp, &default_name, window, cx);
            }
            Pending::ExportPng { default_name, .. } => {
                self.open_path_sheet(SheetKind::ExportPng, &default_name, window, cx);
            }
            Pending::ExportJpeg { default_name } => {
                self.open_path_sheet(SheetKind::ExportJpeg, &default_name, window, cx);
            }
            Pending::Options { kind } => {
                let sheet_kind = match kind {
                    crate::project::OptionsKind::CanvasSize => SheetKind::CanvasSize,
                    crate::project::OptionsKind::ImageSize => SheetKind::ImageSize,
                    crate::project::OptionsKind::Trim => SheetKind::Trim,
                };
                self.open_sheet(sheet_kind, window, cx);
            }
            Pending::Open => {
                self.open_sheet(SheetKind::Open, window, cx);
            }
            Pending::Confirm { step, message, .. } => {
                self.open_sheet(SheetKind::Confirm, window, cx);
                if let Some(sheet) = self.sheet.as_mut() {
                    sheet.message = message;
                    sheet.confirm_step = Some(step);
                }
            }
            Pending::Error { title, message } => {
                self.open_alert(&title, &message, window, cx);
            }
        }
    }

    /// Surfaces a controller's error instruction, if one was just raised.
    pub(crate) fn drain_controller_error(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self.with_current_tab(|controller, _, _| controller.take_pending());
        if let Some(Pending::Error { title, message }) = pending {
            self.open_alert(&title, &message, window, cx);
        }
    }

    // MARK: Save panels

    /// OK/Cancel on a save panel: `finish_save` on the tab that owns the
    /// pending `SaveAs`, then whatever the flow scheduled next.
    pub fn finish_pending_save(
        &mut self,
        url: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (tab_id, after) = self
            .pending_save
            .take()
            .unwrap_or((self.workspace.selected_id(), SaveAfter::Nothing));
        let saved = match self.workspace.tab_mut(tab_id) {
            Some(ProjectTab {
                session,
                busy,
                controller,
                ..
            }) => controller.finish_save(session, busy, url),
            None => false,
        };
        self.drain_controller_error(window, cx);
        self.follow_save(after, tab_id, saved, window, cx);
    }

    /// Continues after a mid-flow save panel: a plain Save just reports,
    /// a confirmation's Save carries the result back into the close/quit
    /// flow.
    fn follow_save(
        &mut self,
        after: SaveAfter,
        tab_id: TabId,
        saved: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match after {
            SaveAfter::Nothing => {
                if saved {
                    self.set_status("Saved", cx);
                }
            }
            SaveAfter::Close => {
                let step = self.workspace.close_after_save(saved);
                self.follow_close_step(step, window, cx);
            }
            SaveAfter::Quit => {
                let _ = tab_id;
                let step = self.workspace.quit_after_save(saved);
                self.follow_quit_step(step, window, cx);
            }
        }
    }

    fn follow_close_step(
        &mut self,
        step: Step<()>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match step {
            Step::Done(()) => {
                self.request_refit();
                self.touch(cx);
            }
            Step::Pending(pending) => self.present_pending(pending, _window, cx),
        }
    }

    fn follow_quit_step(
        &mut self,
        step: Step<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match step {
            Step::Done(true) => {
                // The quit (or window-close) flow finished: the window goes
                // away (`closeWindow` already reset the tabs).
                window.remove_window();
            }
            Step::Done(false) => {
                self.touch(cx);
            }
            Step::Pending(pending) => self.present_pending(pending, window, cx),
        }
    }

    // MARK: Export panels

    /// OK/Cancel on the PNG export panel.
    pub fn finish_pending_export_png(
        &mut self,
        url: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ok = {
            let ProjectTab {
                busy, controller, ..
            } = self.workspace.current_mut();
            controller.finish_export_png(busy, url.as_deref())
        };
        self.drain_controller_error(window, cx);
        if ok {
            if let Some(url) = url {
                self.set_status(format!("Exported {}", url.display()), cx);
            }
        }
    }

    /// OK/Cancel on the JPEG export panel (`finish(result?.data)`).
    pub fn finish_pending_export_jpeg(
        &mut self,
        url: Option<PathBuf>,
        data: &[u8],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ok = {
            let ProjectTab {
                busy, controller, ..
            } = self.workspace.current_mut();
            controller.finish_export_jpeg(busy, url.as_deref(), data)
        };
        self.drain_controller_error(window, cx);
        if ok {
            if let Some(url) = url {
                self.set_status(format!("Exported {}", url.display()), cx);
            }
        }
    }

    // MARK: The unsaved-changes confirm

    /// One of the confirm's three buttons. The answer routes back into the
    /// flow the question belonged to (`Confirm.step`).
    pub fn resolve_confirm_choice(
        &mut self,
        choice: UnsavedChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let step = self
            .sheet
            .as_ref()
            .and_then(|sheet| sheet.confirm_step)
            .unwrap_or(ConfirmStep::Quit);
        self.close_sheet(cx);
        match step {
            ConfirmStep::Close => {
                match self.workspace.resolve_close(choice) {
                    Step::Done(()) => {
                        self.request_refit();
                        self.touch(cx);
                    }
                    Step::Pending(Pending::SaveAs { .. }) => {
                        // The Save button raised a save panel on the asked
                        // tab; resume the close flow after it completes.
                        self.pending_save = Some((
                            self.workspace.close_pending_tab().unwrap_or_else(|| self.workspace.selected_id()),
                            SaveAfter::Close,
                        ));
                        self.present_pending_save_flow(window, cx);
                    }
                    Step::Pending(pending) => self.present_pending(pending, window, cx),
                };
            }
            ConfirmStep::Quit => {
                match self.workspace.resolve_quit(choice) {
                    Step::Done(true) => {
                        // The flow finished — a window close (the tabs were
                        // reset) or an explicit quit either way ends here.
                        window.remove_window();
                    }
                    Step::Done(false) => self.touch(cx),
                    Step::Pending(Pending::SaveAs { .. }) => {
                        // The asked tab is the one on screen.
                        self.pending_save = Some((self.workspace.selected_id(), SaveAfter::Quit));
                        self.present_pending_save_flow(window, cx);
                    }
                    Step::Pending(pending) => self.present_pending(pending, window, cx),
                }
            }
            ConfirmStep::Open => {
                // Only reachable from a controller-driven `open` on a live
                // session; the workspace path opens into detached, unmodified
                // tabs, so this never asks.
                let pending = self.with_current_tab(|controller, session, _| {
                    match controller.resolve_confirm(session, choice) {
                        Step::Pending(pending) => Some(pending),
                        _ => None,
                    }
                });
                if let Some(pending) = pending {
                    self.present_pending(pending, window, cx);
                }
            }
        }
    }

    /// Presents the SaveAs pending stored on the flow's tab (the confirm's
    /// Save button path).
    fn present_pending_save_flow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self.with_current_tab(|controller, _, _| controller.take_pending());
        match pending {
            Some(pending @ Pending::SaveAs { .. }) => {
                self.present_pending(pending, window, cx);
            }
            Some(pending) => self.present_pending(pending, window, cx),
            None => {}
        }
    }

    /// The confirm's Cancel: aborts the flow, closes nothing.
    pub fn resolve_confirm_cancel(&mut self, cx: &mut Context<Self>) {
        let step = self
            .sheet
            .as_ref()
            .and_then(|sheet| sheet.confirm_step)
            .unwrap_or(ConfirmStep::Quit);
        self.close_sheet(cx);
        match step {
            ConfirmStep::Close => {
                self.workspace.resolve_close(UnsavedChoice::Cancel);
                self.touch(cx);
            }
            ConfirmStep::Quit => {
                self.workspace.resolve_quit(UnsavedChoice::Cancel);
                self.touch(cx);
            }
            ConfirmStep::Open => {
                self.with_current_tab(|controller, session, _| {
                    controller.resolve_confirm(session, UnsavedChoice::Cancel);
                });
                self.touch(cx);
            }
        }
    }

    /// Cancel on one of the controller-driven sheets: the second half runs
    /// with no answer, releasing the flow's `is_project_busy` / `is_managing`
    /// guard (Swift's panel Cancel runs the same completion with nil).
    pub(crate) fn cancel_pending_sheet(&mut self, cx: &mut Context<Self>) {
        match self.sheet.as_ref().map(|sheet| sheet.kind) {
            Some(SheetKind::Open) => {
                self.workspace.finish_open(Vec::new());
            }
            Some(SheetKind::SaveComp) => {
                self.pending_save = None;
                self.with_current_tab(|controller, session, busy| {
                    controller.finish_save(session, busy, None);
                });
            }
            Some(SheetKind::ExportPng) => {
                self.with_current_tab(|controller, _, busy| {
                    controller.finish_export_png(busy, None);
                });
            }
            Some(SheetKind::ExportJpeg) => {
                self.with_current_tab(|controller, _, busy| {
                    controller.finish_export_jpeg(busy, None, &[]);
                });
            }
            Some(SheetKind::ImageSize) => {
                self.with_current_tab(|controller, session, busy| {
                    controller.finish_image_size(session, busy, None, None, None, LayerSampling::High);
                });
            }
            Some(SheetKind::CanvasSize) => {
                self.with_current_tab(|controller, session, busy| {
                    controller.finish_canvas_size(session, busy, None);
                });
            }
            Some(SheetKind::Trim) => {
                self.with_current_tab(|controller, session, busy| {
                    controller.finish_trim(session, busy, None);
                });
            }
            Some(SheetKind::Confirm) | Some(SheetKind::Alert) | _ => {}
        }
        self.close_sheet(cx);
    }

    /// The Alert sheet's OK (a `Pending::Error` was dismissed).
    pub fn dismiss_error_alert(&mut self, cx: &mut Context<Self>) {
        self.with_current_tab(|controller, _, _| controller.take_pending());
        self.close_sheet(cx);
    }

    // MARK: Workspace error / receive drains (render-time)

    /// Drains queued workspace errors into the Alert sheet (failed project
    /// opens and other workspace-level `showError`s). Call when no sheet is
    /// open.
    pub fn drain_workspace_errors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sheet.is_some() {
            return;
        }
        if let Some((title, message)) = if self.workspace.errors.is_empty() {
            None
        } else {
            Some(self.workspace.errors.remove(0))
        } {
            self.open_alert(&title, &message, window, cx);
        }
    }

    /// Drains queued file receives once the guards cleared; failures land in
    /// the status bar.
    pub fn drain_queued_receives(&mut self) {
        if self.workspace.has_queued_receives() {
            let failures = self.workspace.drain_incoming();
            if !failures.is_empty() {
                self.status = Some(failures.join("; "));
            }
        }
    }
}

/// The unsaved dot drawn in a tab's prefix (`ProjectTabButton`'s 5px circle).
pub(crate) fn unsaved_dot() -> gpui_kit::AnyElement {
    gpui_kit::div()
        .size(px(5.0))
        .rounded_full()
        .bg(rgb(crate::ui::FG_TEXT))
        .into_any_element()
}
