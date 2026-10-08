//! Port of the shape tool's option bar (`ShapeControls.swift`): the shape
//! kind picker and the drag hint.

use gpui_kit::{Context, IntoElement, px};

use compositor_core::document::ShapeKind;

use crate::state::AppState;
use crate::ui::navigation_tool_header::menu_entries;
use crate::ui::{Dropdown, app_dispatch, text_label};

impl AppState {
    /// `ShapeControls.swift`'s bar controls: the Rectangle/Rounded/Ellipse/
    /// Line picker and the drag hint.
    pub(crate) fn shape_controls(&mut self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let app = cx.entity().downgrade();
        let kind = self.tab().session.read(cx).shape_kind;
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
