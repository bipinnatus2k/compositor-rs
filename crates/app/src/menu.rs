//! The in-window menu bar. Each top-level menu is a button whose dropdown
//! dispatches the app's actions (keyboard hints come from the key bindings).
//! Labels that depend on session state follow the Swift menu's dynamic
//! titles (CompositorApp.swift:47-318).

use gpui_kit::{px, Context, IntoElement, ParentElement, Styled, Window};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenu};
use gpui_kit::component::Sizable;

use crate::keys;
use crate::state::AppState;


pub fn render_menu_bar(state: &AppState) -> gpui_kit::AnyElement {
    let session = &state.current_tab().session;
    let rulers = session.shows_rulers;
    let grid = session.shows_grid;
    let guides = session.shows_guides;
    let pixel_grid = session.shows_pixel_grid;
    let snap_guides = session.snap_to_guides;
    let snap_grid = session.snap_to_grid;
    let snap_layers = session.snap_to_layers;
    let snap_bounds = session.snap_to_document_bounds;
    let snapping = session.snapping_enabled;
    let lock_guides = session.locks_guides;
    let transform_controls = session.shows_transform_controls;
    // Dynamic titles (CompositorApp.swift).
    let invert_label = if session.is_mask_selected { "Invert Mask" } else { "Invert" };
    let transform_label = if session.can_transform_selection() {
        "Transform Selection"
    } else {
        "Transform Layer"
    };
    let duplicate_label = if session.selection().is_some() {
        "Layer via Copy"
    } else {
        "Duplicate Layer"
    };
    let clipping_label = match session.active_layer() {
        Some(layer) if layer.mask_source_id.is_some() => "Release Clipping Mask",
        _ => "Create Clipping Mask",
    };
    let visibility_label = match session.active_layer() {
        Some(layer) if !layer.is_visible => "Show Layer",
        _ => "Hide Layer",
    };
    let delete_label = if session.is_mask_selected
        && session
            .active_layer()
            .map(|layer| layer.mask.is_some())
            .unwrap_or(false)
        && session.selected_layer_ids.len() <= 1
    {
        "Delete Layer Mask"
    } else if session.selected_layer_ids.len() > 1 {
        "Delete Layers"
    } else {
        "Delete Layer"
    };
    let merge_label = session.merge_title();
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
                .separator()
                .menu("Quit", Box::new(keys::Quit))
        }))
        .child(menu_button("menu-edit", "Edit", |menu, _, _| {
            menu.menu("Undo", Box::new(keys::Undo))
                .menu("Redo", Box::new(keys::Redo))
                .separator()
                .menu("Cut", Box::new(keys::Cut))
                .menu("Copy", Box::new(keys::CopyPixels))
                .menu("Copy Merged", Box::new(keys::CopyMerged))
                .menu("Paste", Box::new(keys::PastePixels))
                .separator()
                .menu("Fill Foreground", Box::new(keys::FillForeground))
                .menu("Fill Background", Box::new(keys::FillBackground))
                .menu("Clear Selection Pixels", Box::new(keys::ClearSelectionPixels))
                .menu("Content-Aware Fill", Box::new(keys::ContentAwareFill))
                .separator()
                .menu("Free Transform", Box::new(keys::FreeTransform))
                .separator()
                .menu("Keyboard Shortcuts…", Box::new(keys::ShowShortcuts))
        }))
        .child(menu_button("menu-image", "Image", move |menu, _, _| {
            menu.menu("Curves…", Box::new(keys::CurvesSheet))
                .menu("Levels…", Box::new(keys::LevelsSheet))
                .menu("Hue/Saturation…", Box::new(keys::HueSaturationSheet))
                .menu("Black & White…", Box::new(keys::FilterBlackWhite))
                .menu("Color Balance…", Box::new(keys::FilterColorBalance))
                .menu("Exposure…", Box::new(keys::FilterExposure))
                .menu("Gradient Map…", Box::new(keys::FilterGradientMap))
                .menu("Grain…", Box::new(keys::FilterGrain))
                .menu(invert_label, Box::new(keys::InvertPixels))
                .separator()
                .menu("Image Size…", Box::new(keys::ImageSize))
                .menu("Canvas Size…", Box::new(keys::CanvasSize))
                .separator()
                .menu("Trim…", Box::new(keys::TrimSheet))
                .menu("Crop to Selection", Box::new(keys::CropToSelection))
                .separator()
                .menu("Flip Canvas Horizontal", Box::new(keys::FlipCanvasHorizontal))
                .menu("Flip Canvas Vertical", Box::new(keys::FlipCanvasVertical))
        }))
        .child(menu_button("menu-layer", "Layer", move |menu, window, cx| {
            menu.submenu("New Adjustment Layer", window, cx, |menu, _, _| {
                menu.menu("Hue/Saturation", Box::new(keys::AddAdjustmentHsv))
                    .menu("Levels", Box::new(keys::AddAdjustmentLevels))
                    .menu("Curves", Box::new(keys::AddAdjustmentCurves))
                    .menu("Exposure", Box::new(keys::AddAdjustmentExposure))
                    .menu("Gradient Map", Box::new(keys::AddAdjustmentGradientMap))
                    .menu("Grain", Box::new(keys::AddAdjustmentGrain))
                    .menu("Add Noise", Box::new(keys::AddAdjustmentAddNoise))
                    .menu("Gaussian Blur", Box::new(keys::AddAdjustmentGaussianBlur))
                    .menu("Motion Blur", Box::new(keys::AddAdjustmentMotionBlur))
                    .menu("Invert", Box::new(keys::AddAdjustmentInvert))
                    .menu("Black & White", Box::new(keys::AddAdjustmentBlackWhite))
                    .menu("Color Balance", Box::new(keys::AddAdjustmentColorBalance))
            })
            .menu("Edit Adjustment…", Box::new(keys::EditAdjustment))
            .separator()
            .menu(transform_label, Box::new(keys::FreeTransform))
            .menu(duplicate_label, Box::new(keys::DuplicateLayer))
            .separator()
            .menu(clipping_label, Box::new(keys::ToggleClippingMask))
            .separator()
            .menu("Group Selected Layers", Box::new(keys::GroupSelectedLayers))
            .menu("Move Out of Folder", Box::new(keys::MoveOutOfFolder))
            .menu("New Blank Layer", Box::new(keys::NewLayer))
            .menu("Rename Layer…", Box::new(keys::RenameLayer))
            .menu(visibility_label, Box::new(keys::ToggleLayerVisibility))
            .separator()
            .menu("Move Layer Up", Box::new(keys::MoveLayerUp))
            .menu("Move Layer Down", Box::new(keys::MoveLayerDown))
            .separator()
            .menu(merge_label, Box::new(keys::MergeDown))
            .menu("Add Mask", Box::new(keys::AddMask))
            .separator()
            .menu("Flip Horizontal", Box::new(keys::FlipLayerHorizontal))
            .menu("Flip Vertical", Box::new(keys::FlipLayerVertical))
            .separator()
            .menu(delete_label, Box::new(keys::DeleteLayer))
        }))
        .child(menu_button("menu-select", "Select", |menu, _, _| {
            menu.menu("All", Box::new(keys::SelectAllAction))
                .menu("Deselect", Box::new(keys::DeselectAction))
                .menu("Inverse", Box::new(keys::InvertSelection))
                .separator()
                .menu("Layer's Pixels", Box::new(keys::LoadLayerSelection))
                .menu("Mask's Black Areas", Box::new(keys::LoadMaskSelection))
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
                .separator()
                .menu("Remove Background…", Box::new(keys::RemoveBackground))
        }))
        .child(menu_button("menu-view", "View", move |menu, _, _| {
            menu.menu("Fit Canvas", Box::new(keys::ZoomFit))
                .menu("Actual Pixels", Box::new(keys::Zoom100))
                .menu("Zoom In", Box::new(keys::ZoomIn))
                .menu("Zoom Out", Box::new(keys::ZoomOut))
                .separator()
                .menu_with_check("Show Transform Controls", transform_controls, Box::new(keys::ShowTransformControls))
                .menu_with_check("Rulers", rulers, Box::new(keys::ToggleRulers))
                .menu_with_check("Grid", grid, Box::new(keys::ToggleGrid))
                .menu_with_check("Guides", guides, Box::new(keys::ToggleGuides))
                .menu_with_check("Pixel Grid", pixel_grid, Box::new(keys::TogglePixelGrid))
                .separator()
                .menu_with_check("Snap", snapping, Box::new(keys::SnapToggle))
                .menu_with_check("Snap to Guides", snap_guides, Box::new(keys::SnapGuides))
                .menu_with_check("Snap to Grid", snap_grid, Box::new(keys::SnapGrid))
                .menu_with_check("Snap to Layers", snap_layers, Box::new(keys::SnapLayers))
                .menu_with_check(
                    "Snap to Document Bounds",
                    snap_bounds,
                    Box::new(keys::SnapBounds),
                )
                .separator()
                .menu_with_check("Lock Guides", lock_guides, Box::new(keys::LockGuides))
                .menu("Clear Guides", Box::new(keys::ClearGuides))
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
