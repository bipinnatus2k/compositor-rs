//! Port of the crop tool's option bar (`CropControls.swift`): the ratio
//! picker and the Apply/Cancel pair for the in-canvas crop frame. The bar
//! is a view entity (ADR 0003 step E, ticket 42); the bar shell itself
//! stays in ui/navigation_tool_header.rs.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{Context, Entity, IntoElement, WeakEntity, px};

use compositor_core::session::EditorSession;

use crate::state::AppState;
use crate::ui::navigation_tool_header::menu_entries;
use crate::ui::{Dropdown, app_dispatch};

/// The crop tool's option bar (`CropControls.swift`): a view entity since
/// ticket 42. It carries no widget state of its own (Swift's bar is
/// `@Bindable session` only); only in the render tree while the crop tool
/// is active, so it keeps no session observer, and its session field is
/// re-pointed on tab changes by [`AppState::sync_session_observer`].
pub struct CropControlsView {
    /// The active tab's session — the ratio readout and the Apply/Cancel
    /// writes speak to it directly.
    session: Entity<EditorSession>,
    /// The window shell (the transitional root): the repaint epoch and the
    /// shared header row stay there. No AppState dependencies beyond what
    /// the moved handlers already used.
    app: WeakEntity<AppState>,
}

impl CropControlsView {
    pub(crate) fn new(session: Entity<EditorSession>, app: WeakEntity<AppState>) -> Self {
        CropControlsView { session, app }
    }

    /// Re-points the session on tab changes (`AppState::sync_session_observer`,
    /// same as the canvas view and the layers panel).
    pub(crate) fn set_session(&mut self, session: Entity<EditorSession>) {
        self.session = session;
    }

    /// The view's route to the app-side repaint epoch (`touch`): the frame
    /// cache and `revision` stay on AppState (ticket 39's field audit), so
    /// an interaction that changed the session asks the shell through the
    /// seam.
    fn touch(&mut self, cx: &mut Context<Self>) {
        self.app.update(cx, |app, cx| app.touch(cx)).ok();
    }

    /// `CropControls.swift`'s bar controls: the aspect-ratio dropdown and
    /// the Apply/Cancel buttons (both dead while no crop frame exists).
    pub(crate) fn crop_controls(&mut self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let app = self.app.clone();
        let (choice, has_crop) = {
            let session = self.session.read(cx);
            (
                session.crop_ratio_choice.clone(),
                session.crop_rect.is_some(),
            )
        };
        let entries = menu_entries(
            [
                ("Free", "Free"),
                ("Original", "Original"),
                ("1:1", "1:1"),
                ("4:3", "4:3"),
                ("16:9", "16:9"),
            ],
            choice.as_str(),
            |session, value| session.crop_ratio_choice = value.to_string(),
        );
        vec![
            Dropdown::new("crop-ratio", choice.clone())
                .with_width(px(92.0))
                .with_entries(entries)
                .with_dispatch(app_dispatch(&app))
                .into_any_element(),
            Button::new("crop-apply")
                .label("Apply")
                .small()
                .primary()
                .disabled(!has_crop)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.session.update(cx, |session, _| session.commit_crop());
                    this.touch(cx);
                }))
                .into_any_element(),
            Button::new("crop-cancel")
                .label("Cancel")
                .small()
                .disabled(!has_crop)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.session.update(cx, |session, _| session.cancel_crop());
                    this.touch(cx);
                }))
                .into_any_element(),
        ]
    }
}
