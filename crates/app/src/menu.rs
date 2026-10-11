//! The application menus (`CompositorApp.swift`'s `.commands`, drawn
//! in-window because the port shows no native menu bar): `build_menus`
//! snapshots the active session into gpui `Menu`/`MenuItem`s for the
//! gpui-kit `AppMenuBar` the root hosts inside the title bar. Labels that
//! depend on session state follow the Swift menu's dynamic titles
//! (CompositorApp.swift:47-318); the root re-registers the snapshot when
//! [`MenuSignature`] says it drifted.

use gpui_kit::{Menu, MenuItem, SharedString};

use compositor_core::session::EditorSession;

use crate::keys;

/// The app menu snapshot: File / Edit / Image / Layer / Select / Filter /
/// View / Window / Help, with the session-dependent labels and View
/// checkmarks resolved at build time. Dispatch stays action-based — every
/// item carries a `keys` action, so the popup needs no app handle.
pub(crate) fn build_menus(session: &EditorSession) -> Vec<Menu> {
    // Dynamic titles (CompositorApp.swift).
    let invert_label = if session.is_mask_selected {
        "Invert Mask"
    } else {
        "Invert"
    };
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

    vec![
        Menu::new("File").items([
            item("New…", keys::NewCanvas),
            item("Open…", keys::OpenDocument),
            sep(),
            item("Save", keys::SaveDocument),
            item("Export PNG…", keys::ExportPng),
            item("Export JPEG…", keys::ExportJpeg),
            sep(),
            item("Import Images…", keys::ImportImages),
            sep(),
            item("Close", keys::CloseDocument),
            sep(),
            item("Quit", keys::Quit),
        ]),
        Menu::new("Edit").items([
            item("Undo", keys::Undo),
            item("Redo", keys::Redo),
            sep(),
            item("Cut", keys::Cut),
            item("Copy", keys::CopyPixels),
            item("Copy Merged", keys::CopyMerged),
            item("Paste", keys::PastePixels),
            sep(),
            item("Fill Foreground", keys::FillForeground),
            item("Fill Background", keys::FillBackground),
            item("Clear Selection Pixels", keys::ClearSelectionPixels),
            item("Content-Aware Fill", keys::ContentAwareFill),
            sep(),
            item("Free Transform", keys::FreeTransform),
            sep(),
            item("Keyboard Shortcuts…", keys::ShowShortcuts),
        ]),
        Menu::new("Image").items([
            item("Curves…", keys::CurvesSheet),
            item("Levels…", keys::LevelsSheet),
            item("Hue/Saturation…", keys::HueSaturationSheet),
            item("Black & White…", keys::FilterBlackWhite),
            item("Color Balance…", keys::FilterColorBalance),
            item("Exposure…", keys::FilterExposure),
            item("Gradient Map…", keys::FilterGradientMap),
            item("Grain…", keys::FilterGrain),
            item(invert_label, keys::InvertPixels),
            sep(),
            item("Image Size…", keys::ImageSize),
            item("Canvas Size…", keys::CanvasSize),
            sep(),
            item("Trim…", keys::TrimSheet),
            item("Crop to Selection", keys::CropToSelection),
            sep(),
            item("Flip Canvas Horizontal", keys::FlipCanvasHorizontal),
            item("Flip Canvas Vertical", keys::FlipCanvasVertical),
        ]),
        Menu::new("Layer").items([
            MenuItem::submenu(Menu::new("New Adjustment Layer").items([
                item("Hue/Saturation", keys::AddAdjustmentHsv),
                item("Levels", keys::AddAdjustmentLevels),
                item("Curves", keys::AddAdjustmentCurves),
                item("Exposure", keys::AddAdjustmentExposure),
                item("Gradient Map", keys::AddAdjustmentGradientMap),
                item("Grain", keys::AddAdjustmentGrain),
                item("Add Noise", keys::AddAdjustmentAddNoise),
                item("Gaussian Blur", keys::AddAdjustmentGaussianBlur),
                item("Motion Blur", keys::AddAdjustmentMotionBlur),
                item("Invert", keys::AddAdjustmentInvert),
                item("Black & White", keys::AddAdjustmentBlackWhite),
                item("Color Balance", keys::AddAdjustmentColorBalance),
            ])),
            item("Edit Adjustment…", keys::EditAdjustment),
            sep(),
            item(transform_label, keys::FreeTransform),
            item(duplicate_label, keys::DuplicateLayer),
            sep(),
            item(clipping_label, keys::ToggleClippingMask),
            sep(),
            item("Group Selected Layers", keys::GroupSelectedLayers),
            item("Move Out of Folder", keys::MoveOutOfFolder),
            item("New Blank Layer", keys::NewLayer),
            item("Rename Layer…", keys::RenameLayer),
            item(visibility_label, keys::ToggleLayerVisibility),
            sep(),
            item("Move Layer Up", keys::MoveLayerUp),
            item("Move Layer Down", keys::MoveLayerDown),
            sep(),
            item(merge_label, keys::MergeDown),
            item("Add Mask", keys::AddMask),
            sep(),
            item("Flip Horizontal", keys::FlipLayerHorizontal),
            item("Flip Vertical", keys::FlipLayerVertical),
            sep(),
            item(delete_label, keys::DeleteLayer),
        ]),
        Menu::new("Select").items([
            item("All", keys::SelectAllAction),
            item("Deselect", keys::DeselectAction),
            item("Inverse", keys::InvertSelection),
            sep(),
            item("Layer's Pixels", keys::LoadLayerSelection),
            item("Mask's Black Areas", keys::LoadMaskSelection),
            sep(),
            item("Feather…", keys::FeatherSheet),
            item("Expand…", keys::ExpandSheet),
            item("Contract…", keys::ContractSheet),
        ]),
        Menu::new("Filter").items([
            item("Gaussian Blur…", keys::FilterGaussianBlur),
            item("Motion Blur…", keys::FilterMotionBlur),
            item("Add Noise…", keys::FilterAddNoise),
            sep(),
            item("Lens Correction…", keys::FilterLensCorrection),
            item("Camera Raw Filter…", keys::FilterCameraRaw),
        ]),
        Menu::new("View").items([
            item("Fit Canvas", keys::ZoomFit),
            item("Actual Pixels", keys::Zoom100),
            item("Zoom In", keys::ZoomIn),
            item("Zoom Out", keys::ZoomOut),
            sep(),
            check(
                "Show Transform Controls",
                keys::ShowTransformControls,
                session.shows_transform_controls,
            ),
            check("Rulers", keys::ToggleRulers, session.shows_rulers),
            check("Grid", keys::ToggleGrid, session.shows_grid),
            check("Guides", keys::ToggleGuides, session.shows_guides),
            check(
                "Pixel Grid",
                keys::TogglePixelGrid,
                session.shows_pixel_grid,
            ),
            sep(),
            check("Snap", keys::SnapToggle, session.snapping_enabled),
            check("Snap to Guides", keys::SnapGuides, session.snap_to_guides),
            check("Snap to Grid", keys::SnapGrid, session.snap_to_grid),
            check("Snap to Layers", keys::SnapLayers, session.snap_to_layers),
            check(
                "Snap to Document Bounds",
                keys::SnapBounds,
                session.snap_to_document_bounds,
            ),
            sep(),
            check("Lock Guides", keys::LockGuides, session.locks_guides),
            item("Clear Guides", keys::ClearGuides),
        ]),
        Menu::new("Window").items([item("Reset Panel Layout", keys::ResetPanels)]),
        Menu::new("Help").items([item("About Compositor", keys::About)]),
    ]
}

/// Every menu-affecting bit of the session, in one comparable value: the
/// root rebuilds the app menus only when this drifted since the last
/// registration, so unrelated re-renders don't drop an open popup state.
#[derive(Clone, PartialEq)]
pub(crate) struct MenuSignature {
    rulers: bool,
    grid: bool,
    guides: bool,
    pixel_grid: bool,
    snapping: bool,
    snap_guides: bool,
    snap_grid: bool,
    snap_layers: bool,
    snap_bounds: bool,
    lock_guides: bool,
    transform_controls: bool,
    mask_selected: bool,
    has_selection: bool,
    transformable: bool,
    layer_visible: bool,
    layer_has_mask: bool,
    clipping_mask: bool,
    selected_count: usize,
    merge_title: &'static str,
}

impl MenuSignature {
    pub(crate) fn of(session: &EditorSession) -> Self {
        MenuSignature {
            rulers: session.shows_rulers,
            grid: session.shows_grid,
            guides: session.shows_guides,
            pixel_grid: session.shows_pixel_grid,
            snapping: session.snapping_enabled,
            snap_guides: session.snap_to_guides,
            snap_grid: session.snap_to_grid,
            snap_layers: session.snap_to_layers,
            snap_bounds: session.snap_to_document_bounds,
            lock_guides: session.locks_guides,
            transform_controls: session.shows_transform_controls,
            mask_selected: session.is_mask_selected,
            has_selection: session.selection().is_some(),
            transformable: session.can_transform_selection(),
            layer_visible: session
                .active_layer()
                .map(|layer| layer.is_visible)
                .unwrap_or_default(),
            layer_has_mask: session
                .active_layer()
                .map(|layer| layer.mask.is_some())
                .unwrap_or_default(),
            clipping_mask: session
                .active_layer()
                .map(|layer| layer.mask_source_id.is_some())
                .unwrap_or_default(),
            selected_count: session.selected_layer_ids.len(),
            merge_title: session.merge_title(),
        }
    }
}

fn item(label: impl Into<SharedString>, action: impl gpui_kit::Action) -> MenuItem {
    MenuItem::action(label, action)
}

fn check(label: impl Into<SharedString>, action: impl gpui_kit::Action, checked: bool) -> MenuItem {
    MenuItem::action(label, action).checked(checked)
}

fn sep() -> MenuItem {
    MenuItem::separator()
}
