//! Port of the shape tool's option bar (`ShapeControls.swift`): the shape
//! kind picker and the drag hint. The bar is a view entity (ADR 0003 step
//! E, ticket 42); the bar shell itself stays in
//! ui/navigation_tool_header.rs.

use gpui_kit::{Context, Entity, IntoElement, WeakEntity, px};

use compositor_core::document::ShapeKind;
use compositor_core::session::EditorSession;

use crate::state::AppState;
use crate::ui::navigation_tool_header::menu_entries;
use crate::ui::{Dropdown, app_dispatch, text_label};

/// The shape tool's option bar (`ShapeControls.swift`): a view entity since
/// ticket 42. It carries no widget state of its own (Swift's bar is
/// `@Bindable session` only); only in the render tree while the shape tool
/// is active, so it keeps no session observer, and its session field is
/// re-pointed on tab changes by [`AppState::sync_session_observer`].
pub struct ShapeControlsView {
    /// The active tab's session — the kind readout speaks to it directly.
    session: Entity<EditorSession>,
    /// The window shell (the transitional root): the shared header row
    /// stays there. No AppState dependencies beyond what the moved handlers
    /// already used.
    app: WeakEntity<AppState>,
}

impl ShapeControlsView {
    pub(crate) fn new(session: Entity<EditorSession>, app: WeakEntity<AppState>) -> Self {
        ShapeControlsView { session, app }
    }

    /// Re-points the session on tab changes (`AppState::sync_session_observer`,
    /// same as the canvas view and the layers panel).
    pub(crate) fn set_session(&mut self, session: Entity<EditorSession>) {
        self.session = session;
    }

    /// `ShapeControls.swift`'s bar controls: the Rectangle/Rounded/Ellipse/
    /// Line picker and the drag hint.
    pub(crate) fn shape_controls(&mut self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let app = self.app.clone();
        let kind = self.session.read(cx).shape_kind;
        let label = |kind: ShapeKind| match kind {
            ShapeKind::Rectangle => "Rectangle",
            ShapeKind::RoundedRectangle => "Rounded Rectangle",
            ShapeKind::Ellipse => "Ellipse",
            ShapeKind::Line => "Line",
        };
        vec![
            Dropdown::new("shape-kind", label(kind))
                .with_width(px(150.0))
                .with_entries(menu_entries(
                    [
                        (ShapeKind::Rectangle, "Rectangle"),
                        (ShapeKind::RoundedRectangle, "Rounded Rectangle"),
                        (ShapeKind::Ellipse, "Ellipse"),
                        (ShapeKind::Line, "Line"),
                    ],
                    kind,
                    |session, value| session.shape_kind = value,
                ))
                .with_dispatch(app_dispatch(&app))
                .into_any_element(),
            text_label("Drag on the canvas · Shift-U switches shape", true).into_any_element(),
        ]
    }
}
