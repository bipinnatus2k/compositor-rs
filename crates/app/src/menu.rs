//! The in-window menu bar. Each top-level menu is a button whose dropdown
//! dispatches the app's actions (keyboard hints come from the key bindings).

use gpui_kit::{px, Context, IntoElement, ParentElement, Styled, Window};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenu};
use gpui_kit::component::Sizable;

use crate::keys;
use crate::state::AppState;


pub fn render_menu_bar(state: &AppState) -> gpui_kit::AnyElement {
    let session = &state.session;
    let rulers = session.shows_rulers;
    let grid = session.shows_grid;
    let guides = session.shows_guides;
    let pixel_grid = session.shows_pixel_grid;
    let snap_guides = session.snap_to_guides;
    let snap_grid = session.snap_to_grid;
    let snap_layers = session.snap_to_layers;
    let snap_bounds = session.snap_to_document_bounds;
    let bar = gpui_kit::div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(28.0))
        .px_1()
        .gap_0p5()
        .flex_shrink_0()
        .bg(crate::ui::panel_bg())
        .border_b_1()
        .border_color(gpui_kit::black().opacity(0.4))
        .child(menu_button("menu-file", "File", |menu, _, _| {
            menu.menu("New…", Box::new(keys::NewCanvas))
                .menu("Open…", Box::new(keys::OpenDocument))
                .separator()
                .menu("Save", Box::new(keys::SaveDocument))
                .menu("Export PNG…", Box::new(keys::ExportPng))
                .menu("Export JPEG…", Box::new(keys::ExportJpeg))
                .separator()
                .menu("Import Images…", Box::new(keys::ImportImages))
                .separator()
                .menu("Close", Box::new(keys::CloseDocument))
        }))
        .child(menu_button("menu-edit", "Edit", |menu, _, _| {
            menu.menu("Undo", Box::new(keys::Undo))
                .menu("Redo", Box::new(keys::Redo))
                .separator()
                .menu("Fill Foreground", Box::new(keys::FillForeground))
                .menu("Fill Background", Box::new(keys::FillBackground))
                .separator()
                .menu("Free Transform", Box::new(keys::FreeTransform))
                .separator()
                .menu("Keyboard Shortcuts…", Box::new(keys::ShowShortcuts))
        }))
        .child(menu_button("menu-image", "Image", |menu, _, _| {
            menu.menu("Image Size…", Box::new(keys::ImageSize))
                .menu("Canvas Size…", Box::new(keys::CanvasSize))
                .separator()
                .menu("Trim…", Box::new(keys::TrimSheet))
                .menu("Crop to Selection", Box::new(keys::CropToSelection))
                .separator()
                .menu("Flip Canvas Horizontal", Box::new(keys::FlipCanvasHorizontal))
                .menu("Flip Canvas Vertical", Box::new(keys::FlipCanvasVertical))
        }))
        .child(menu_button("menu-layer", "Layer", |menu, _, _| {
            menu.menu("New Layer", Box::new(keys::NewLayer))
                .menu("Duplicate Layer", Box::new(keys::DuplicateLayer))
                .menu("Delete Layer", Box::new(keys::DeleteLayer))
                .separator()
                .menu("Merge Down", Box::new(keys::MergeDown))
                .menu("New Group", Box::new(keys::NewGroup))
                .separator()
                .menu("Add Mask", Box::new(keys::AddMask))
                .separator()
                .menu("Flip Horizontal", Box::new(keys::FlipLayerHorizontal))
                .menu("Flip Vertical", Box::new(keys::FlipLayerVertical))
        }))
        .child(menu_button("menu-select", "Select", |menu, _, _| {
            menu.menu("All", Box::new(keys::SelectAllAction))
                .menu("Deselect", Box::new(keys::DeselectAction))
                .menu("Inverse", Box::new(keys::InvertSelection))
                .separator()
                .menu("Feather…", Box::new(keys::FeatherSheet))
                .menu("Expand…", Box::new(keys::ExpandSheet))
                .menu("Contract…", Box::new(keys::ContractSheet))
        }))
        .child(menu_button("menu-filter", "Filter", |menu, _, _| {
            menu.menu("Gaussian Blur…", Box::new(keys::FilterGaussianBlur))
                .menu("Motion Blur…", Box::new(keys::FilterMotionBlur))
                .menu("Add Noise…", Box::new(keys::FilterAddNoise))
                .separator()
                .menu("Lens Correction…", Box::new(keys::FilterLensCorrection))
                .menu("Camera Raw Filter…", Box::new(keys::FilterCameraRaw))
        }))
        .child(menu_button("menu-view", "View", move |menu, _, _| {
            menu.menu_with_check("Rulers", rulers, Box::new(keys::ToggleRulers))
                .menu_with_check("Grid", grid, Box::new(keys::ToggleGrid))
                .menu_with_check("Guides", guides, Box::new(keys::ToggleGuides))
                .menu_with_check("Pixel Grid", pixel_grid, Box::new(keys::TogglePixelGrid))
                .separator()
                .menu_with_check("Snap to Guides", snap_guides, Box::new(keys::SnapGuides))
                .menu_with_check("Snap to Grid", snap_grid, Box::new(keys::SnapGrid))
                .menu_with_check("Snap to Layers", snap_layers, Box::new(keys::SnapLayers))
                .menu_with_check(
                    "Snap to Document Bounds",
                    snap_bounds,
                    Box::new(keys::SnapBounds),
                )
        }))
        .child(menu_button("menu-window", "Window", |menu, _, _| {
            menu.menu("Reset Panel Layout", Box::new(keys::ResetPanels))
        }))
        .child(menu_button("menu-help", "Help", |menu, _, _| {
            menu.menu("About Compositor", Box::new(keys::About))
        }));

    bar.into_any_element()
}

fn menu_button(
    id: &'static str,
    label: &'static str,
    build: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> impl IntoElement {
    Button::new(id)
        .label(label)
        .ghost()
        .small()
        .dropdown_menu(build)
}
