//! The workspace: window layout, the action dispatch for every menu item and
//! keyboard shortcut, and the status bar.

use gpui_kit::{div, px, rgb, Context, IntoElement, InteractiveElement, ParentElement, Render, Styled, Window};

use compositor_core::adjustment::AdjustmentKind;
use compositor_core::document::NavigationTool;
use compositor_core::filters::FilterKind;
use compositor_core::session::SelectionAmountOperation;

use crate::keys;
use crate::state::{AppState, SheetKind};
use crate::ui::{text_label, FG_DIM};

impl Render for AppState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("workspace")
            .key_context("Compositor")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(crate::ui::BG_APP))
            .text_color(rgb(crate::ui::FG_TEXT))
            .text_size(px(12.0))
            // Tools
            .on_action(cx.listener(Self::on_tool_move))
            .on_action(cx.listener(Self::on_tool_marquee))
            .on_action(cx.listener(Self::on_tool_lasso))
            .on_action(cx.listener(Self::on_tool_wand))
            .on_action(cx.listener(Self::on_tool_crop))
            .on_action(cx.listener(Self::on_tool_brush))
            .on_action(cx.listener(Self::on_tool_eraser))
            .on_action(cx.listener(Self::on_tool_healing))
            .on_action(cx.listener(Self::on_tool_clone))
            .on_action(cx.listener(Self::on_tool_smear))
            .on_action(cx.listener(Self::on_tool_gradient))
            .on_action(cx.listener(Self::on_tool_shape))
            .on_action(cx.listener(Self::on_tool_type))
            .on_action(cx.listener(Self::on_tool_eyedropper))
            .on_action(cx.listener(Self::on_tool_hand))
            .on_action(cx.listener(Self::on_tool_zoom))
            // Palette and modes
            .on_action(cx.listener(Self::on_swap_colors))
            .on_action(cx.listener(Self::on_default_colors))
            .on_action(cx.listener(Self::on_cycle_tool_mode))
            // File
            .on_action(cx.listener(Self::on_new_canvas))
            .on_action(cx.listener(Self::on_open))
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_save_as))
            .on_action(cx.listener(Self::on_export))
            .on_action(cx.listener(Self::on_export_jpeg))
            .on_action(cx.listener(Self::on_import_images))
            .on_action(cx.listener(Self::on_close_document))
            // Edit
            .on_action(cx.listener(Self::on_undo))
            .on_action(cx.listener(Self::on_redo))
            .on_action(cx.listener(Self::on_fill_foreground))
            .on_action(cx.listener(Self::on_fill_background))
            .on_action(cx.listener(Self::on_free_transform))
            .on_action(cx.listener(Self::on_shortcuts))
            .on_action(cx.listener(Self::on_clear_selection_pixels))
            .on_action(cx.listener(Self::on_load_layer_selection))
            .on_action(cx.listener(Self::on_load_mask_selection))
            // Image
            .on_action(cx.listener(Self::on_image_size))
            .on_action(cx.listener(Self::on_canvas_size))
            .on_action(cx.listener(Self::on_trim))
            .on_action(cx.listener(Self::on_crop_to_selection))
            .on_action(cx.listener(Self::on_flip_canvas_h))
            .on_action(cx.listener(Self::on_flip_canvas_v))
            .on_action(cx.listener(Self::on_levels))
            .on_action(cx.listener(Self::on_hue_saturation))
            .on_action(cx.listener(Self::on_curves))
            .on_action(cx.listener(Self::on_filter_black_white))
            .on_action(cx.listener(Self::on_filter_color_balance))
            .on_action(cx.listener(Self::on_filter_exposure))
            .on_action(cx.listener(Self::on_filter_gradient_map))
            .on_action(cx.listener(Self::on_filter_grain))
            // Layer
            .on_action(cx.listener(Self::on_new_layer))
            .on_action(cx.listener(Self::on_duplicate_layer))
            .on_action(cx.listener(Self::on_delete_layer))
            .on_action(cx.listener(Self::on_merge_down))
            .on_action(cx.listener(Self::on_new_group))
            .on_action(cx.listener(Self::on_add_mask))
            .on_action(cx.listener(Self::on_flip_layer_h))
            .on_action(cx.listener(Self::on_flip_layer_v))
            .on_action(cx.listener(Self::on_group_selected_layers))
            .on_action(cx.listener(Self::on_move_out_of_folder))
            .on_action(cx.listener(Self::on_rename_layer))
            .on_action(cx.listener(Self::on_toggle_layer_visibility))
            .on_action(cx.listener(Self::on_add_adjustment_hsv))
            .on_action(cx.listener(Self::on_add_adjustment_levels))
            .on_action(cx.listener(Self::on_add_adjustment_curves))
            .on_action(cx.listener(Self::on_add_adjustment_exposure))
            .on_action(cx.listener(Self::on_add_adjustment_gradient_map))
            .on_action(cx.listener(Self::on_add_adjustment_grain))
            .on_action(cx.listener(Self::on_add_adjustment_add_noise))
            .on_action(cx.listener(Self::on_add_adjustment_gaussian_blur))
            .on_action(cx.listener(Self::on_add_adjustment_motion_blur))
            .on_action(cx.listener(Self::on_add_adjustment_invert))
            .on_action(cx.listener(Self::on_add_adjustment_black_white))
            .on_action(cx.listener(Self::on_add_adjustment_color_balance))
            .on_action(cx.listener(Self::on_edit_adjustment))
            // Select
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_deselect))
            .on_action(cx.listener(Self::on_invert_selection))
            .on_action(cx.listener(Self::on_feather))
            .on_action(cx.listener(Self::on_expand))
            .on_action(cx.listener(Self::on_contract))
            // Filter
            .on_action(cx.listener(Self::on_filter_blur))
            .on_action(cx.listener(Self::on_filter_motion))
            .on_action(cx.listener(Self::on_filter_noise))
            .on_action(cx.listener(Self::on_filter_lens))
            .on_action(cx.listener(Self::on_filter_camera_raw))
            .on_action(cx.listener(Self::on_remove_background))
            // View
            .on_action(cx.listener(Self::on_toggle_rulers))
            .on_action(cx.listener(Self::on_toggle_grid))
            .on_action(cx.listener(Self::on_toggle_guides))
            .on_action(cx.listener(Self::on_toggle_pixel_grid))
            .on_action(cx.listener(Self::on_snap_guides))
            .on_action(cx.listener(Self::on_snap_toggle))
            .on_action(cx.listener(Self::on_lock_guides))
            .on_action(cx.listener(Self::on_snap_grid))
            .on_action(cx.listener(Self::on_snap_layers))
            .on_action(cx.listener(Self::on_snap_bounds))
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .on_action(cx.listener(Self::on_zoom_fit))
            .on_action(cx.listener(Self::on_zoom_100))
            .on_action(cx.listener(Self::on_clear_guides))
            // Window / Help
            .on_action(cx.listener(Self::on_reset_panels))
            .on_action(cx.listener(Self::on_about))
            // Canvas keys
            .on_action(cx.listener(Self::on_nudge_left))
            .on_action(cx.listener(Self::on_nudge_right))
            .on_action(cx.listener(Self::on_nudge_up))
            .on_action(cx.listener(Self::on_nudge_down))
            .on_action(cx.listener(Self::on_nudge_left_more))
            .on_action(cx.listener(Self::on_nudge_right_more))
            .on_action(cx.listener(Self::on_nudge_up_more))
            .on_action(cx.listener(Self::on_nudge_down_more))
            .on_action(cx.listener(Self::on_brush_smaller))
            .on_action(cx.listener(Self::on_brush_larger))
            .on_action(cx.listener(Self::on_brush_softer))
            .on_action(cx.listener(Self::on_brush_harder))
            .on_action(cx.listener(Self::on_blend_mode_previous))
            .on_action(cx.listener(Self::on_blend_mode_next))
            .on_action(cx.listener(Self::on_cycle_shape_kind))
            .on_action(cx.listener(Self::on_show_transform_controls))
            .on_action(cx.listener(Self::on_invert_pixels))
            .on_action(cx.listener(Self::on_toggle_clipping_mask))
            .on_action(cx.listener(Self::on_move_layer_down))
            .on_action(cx.listener(Self::on_move_layer_up))
            .on_action(cx.listener(Self::on_cut))
            .on_action(cx.listener(Self::on_copy_pixels))
            .on_action(cx.listener(Self::on_copy_merged))
            .on_action(cx.listener(Self::on_paste_pixels))
            .on_action(cx.listener(Self::on_content_aware_fill))
            .on_action(cx.listener(Self::on_toggle_levels_preview))
            .on_action(cx.listener(Self::on_finish_lasso))
            .on_action(cx.listener(Self::on_cancel))
            // Layout
            .child(crate::menu::render_menu_bar(self))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_tool_rail(cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(self.render_options_header(cx))
                            .child(self.render_canvas(cx)),
                    )
                    .child(self.render_layers_panel(cx)),
            )
            .child(self.render_status_bar(cx))
            .children(self.render_sheet_overlay(cx))
            .children(crate::color_picker_ui::render_color_picker(self, cx))
    }
}

/// The layer's adjustment settings as the filter-sheet editor reads them:
/// every slot present on the adjustment overrides the default.
fn filter_settings_from_adjustment(
    adjustment: &compositor_core::adjustment::LayerAdjustment,
) -> compositor_core::filters::FilterSettings {
    let mut settings = compositor_core::filters::FilterSettings::default();
    if let Some(value) = &adjustment.exposure_settings {
        settings.exposure = value.clone();
    }
    if let Some(value) = &adjustment.gradient_map_settings {
        settings.gradient_map = value.clone();
    }
    if let Some(value) = &adjustment.grain_settings {
        settings.grain = value.clone();
    }
    if let Some(value) = &adjustment.black_white_settings {
        settings.black_white = value.clone();
    }
    if let Some(value) = &adjustment.color_balance_settings {
        settings.color_balance = value.clone();
    }
    if let Some(value) = adjustment.blur_radius {
        settings.radius = value;
    }
    if let Some(value) = adjustment.motion_angle {
        settings.angle = value;
    }
    if let Some(value) = adjustment.motion_distance {
        settings.distance = value;
    }
    if let Some(value) = adjustment.noise_amount {
        settings.amount = value;
    }
    if let Some(value) = adjustment.noise_gaussian {
        settings.gaussian = value;
    }
    if let Some(value) = adjustment.noise_monochromatic {
        settings.monochromatic = value;
    }
    settings.curves = adjustment.curves.clone();
    settings
}

fn color_bytes(color: compositor_core::document::PaletteColor) -> [u8; 4] {
    [
        (color.red * 255.0).round() as u8,
        (color.green * 255.0).round() as u8,
        (color.blue * 255.0).round() as u8,
        255,
    ]
}

impl AppState {
    // MARK: Shortcut registry

    /// The live shortcut registry (KeyboardShortcuts.swift:129-213). It is
    /// loaded from `%APPDATA%/compositor/shortcuts.json` — or the platform
    /// equivalent — when the app starts (`keys::bindings()` reads it before
    /// the window opens, so custom chords and menu hints are in effect from
    /// the first frame). The shortcut settings sheet reads this for its
    /// draft, assigns overrides with `set_override`, and persists with
    /// `save`; a new launch (or a rebind of `keys::bindings_with`) applies
    /// the change.
    pub fn shortcuts(&self) -> std::sync::MutexGuard<'static, crate::shortcuts::ShortcutRegistry> {
        crate::shortcuts::lock_global()
    }

    // MARK: Tools

    fn on_tool_move(&mut self, _: &keys::ToolMove, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::Move, cx));
    }
    fn on_tool_marquee(&mut self, _: &keys::ToolMarquee, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::Marquee, cx));
    }
    fn on_tool_lasso(&mut self, _: &keys::ToolLasso, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::Lasso, cx));
    }
    fn on_tool_wand(&mut self, _: &keys::ToolWand, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::Wand, cx));
    }
    fn on_tool_crop(&mut self, _: &keys::ToolCrop, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::Crop, cx));
    }
    fn on_tool_brush(&mut self, _: &keys::ToolBrush, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::Brush, cx));
    }
    fn on_tool_eraser(&mut self, _: &keys::ToolEraser, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool_eraser(cx));
    }
    fn on_tool_healing(&mut self, _: &keys::ToolHealing, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::SpotHealing, cx));
    }
    fn on_tool_clone(&mut self, _: &keys::ToolClone, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::CloneStamp, cx));
    }
    fn on_tool_smear(&mut self, _: &keys::ToolSmear, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::Blur, cx));
    }
    fn on_tool_gradient(&mut self, _: &keys::ToolGradient, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::Gradient, cx));
    }
    fn on_tool_shape(&mut self, _: &keys::ToolShape, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::Shape, cx));
    }
    fn on_tool_type(&mut self, _: &keys::ToolType, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::Type, cx));
    }
    fn on_tool_eyedropper(&mut self, _: &keys::ToolEyedropper, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::Eyedropper, cx));
    }
    fn on_tool_hand(&mut self, _: &keys::ToolHand, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::Hand, cx));
    }
    fn on_tool_zoom(&mut self, _: &keys::ToolZoom, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::Zoom, cx));
    }

    fn on_swap_colors(&mut self, _: &keys::SwapColors, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.session.swap_palette_colors();
            t.touch(cx);
        });
    }
    fn on_default_colors(&mut self, _: &keys::DefaultColors, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.session.reset_palette_colors();
            t.touch(cx);
        });
    }
    fn on_cycle_tool_mode(&mut self, _: &keys::CycleToolMode, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.cycle_tool_mode(cx));
    }

    // MARK: File

    fn on_new_canvas(&mut self, _: &keys::NewCanvas, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::NewCanvas, _w, cx);
    }
    fn on_open(&mut self, _: &keys::OpenDocument, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Open, _w, cx);
    }
    fn on_save(&mut self, _: &keys::SaveDocument, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::SaveComp, _w, cx);
    }
    fn on_save_as(&mut self, _: &keys::SaveAs, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::SaveComp, _w, cx);
    }
    fn on_export(&mut self, _: &keys::ExportPng, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::ExportPng, _w, cx);
    }
    fn on_export_jpeg(&mut self, _: &keys::ExportJpeg, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::ExportJpeg, _w, cx);
    }
    fn on_import_images(&mut self, _: &keys::ImportImages, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Import, _w, cx);
    }
    fn on_close_document(&mut self, _: &keys::CloseDocument, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.cancel_crop();
        self.session.cancel_transform();
        self.session.document = None;
        self.session.active_layer_id = None;
        self.session.selected_layer_ids.clear();
        self.fit_for = (0, 0);
        self.touch(cx);
    }

    // MARK: Edit

    fn on_undo(&mut self, _: &keys::Undo, _w: &mut Window, cx: &mut Context<Self>) {
        // A restored document of a different size refits the viewport
        // (EditorSession.undo → viewport.fit).
        let before = self.session.document.as_ref().map(|d| (d.id, d.width, d.height));
        self.session.undo();
        self.refit_if_document_changed(before);
        self.touch(cx);
    }
    fn on_redo(&mut self, _: &keys::Redo, _w: &mut Window, cx: &mut Context<Self>) {
        let before = self.session.document.as_ref().map(|d| (d.id, d.width, d.height));
        self.session.redo();
        self.refit_if_document_changed(before);
        self.touch(cx);
    }
    /// Refits the viewport when undo/redo swapped in a different document.
    fn refit_if_document_changed(&mut self, before: Option<(uuid::Uuid, usize, usize)>) {
        let after = self.session.document.as_ref().map(|d| (d.id, d.width, d.height));
        if before.is_some() && before != after {
            let vw = f32::from(self.viewport.size.width);
            let vh = f32::from(self.viewport.size.height);
            if vw > 10.0 && vh > 10.0 {
                self.zoom_fit((vw, vh));
            }
        }
    }
    fn on_fill_foreground(&mut self, _: &keys::FillForeground, _w: &mut Window, cx: &mut Context<Self>) {
        let color = color_bytes(self.session.palette_color(false));
        self.fill_active(color, "Fill Foreground", cx);
    }
    fn on_fill_background(&mut self, _: &keys::FillBackground, _w: &mut Window, cx: &mut Context<Self>) {
        let color = color_bytes(self.session.palette_color(true));
        self.fill_active(color, "Fill Background", cx);
    }
    fn on_free_transform(&mut self, _: &keys::FreeTransform, _w: &mut Window, cx: &mut Context<Self>) {
        // transformCommand: the selection floats when there is one (FloatingSelection.swift:22).
        if self.session.can_transform_selection() {
            self.session.begin_selection_transform();
        } else {
            self.session.begin_transform();
        }
        self.touch(cx);
    }
    fn on_shortcuts(&mut self, _: &keys::ShowShortcuts, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Shortcuts, _w, cx);
    }

    // MARK: Image

    fn on_image_size(&mut self, _: &keys::ImageSize, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::ImageSize, _w, cx);
    }
    fn on_canvas_size(&mut self, _: &keys::CanvasSize, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::CanvasSize, _w, cx);
    }
    fn on_trim(&mut self, _: &keys::TrimSheet, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Trim, _w, cx);
    }
    fn on_crop_to_selection(&mut self, _: &keys::CropToSelection, _w: &mut Window, cx: &mut Context<Self>) {
        if let Some(selection) = self.session.selection() {
            if let Some(bounds) = selection.path.bounds() {
                self.session.crop_rect = Some(bounds);
                self.set_status("Crop to selection: press Apply in the Crop header", cx);
            }
        }
    }
    fn on_flip_canvas_h(&mut self, _: &keys::FlipCanvasHorizontal, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.flip_canvas(true);
        self.touch(cx);
    }
    fn on_flip_canvas_v(&mut self, _: &keys::FlipCanvasVertical, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.flip_canvas(false);
        self.touch(cx);
    }

    fn on_levels(&mut self, _: &keys::LevelsSheet, w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Levels, w, cx);
    }
    fn on_hue_saturation(&mut self, _: &keys::HueSaturationSheet, w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::HueSaturation, w, cx);
    }
    fn on_curves(&mut self, _: &keys::CurvesSheet, w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Filter(FilterKind::Curves), w, cx);
    }
    fn on_filter_black_white(&mut self, _: &keys::FilterBlackWhite, w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Filter(FilterKind::BlackWhite), w, cx);
    }
    fn on_filter_color_balance(&mut self, _: &keys::FilterColorBalance, w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Filter(FilterKind::ColorBalance), w, cx);
    }
    fn on_filter_exposure(&mut self, _: &keys::FilterExposure, w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Filter(FilterKind::Exposure), w, cx);
    }
    fn on_filter_gradient_map(&mut self, _: &keys::FilterGradientMap, w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Filter(FilterKind::GradientMap), w, cx);
    }
    fn on_filter_grain(&mut self, _: &keys::FilterGrain, w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Filter(FilterKind::Grain), w, cx);
    }

    // MARK: Layer

    fn on_new_layer(&mut self, _: &keys::NewLayer, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.add_blank_layer();
        self.touch(cx);
    }
    fn on_duplicate_layer(&mut self, _: &keys::DuplicateLayer, _w: &mut Window, cx: &mut Context<Self>) {
        // layerViaCopy duplicates when nothing is selected (SelectionClipboard.swift:138).
        self.session.layer_via_copy();
        self.touch(cx);
    }
    fn on_delete_layer(&mut self, _: &keys::DeleteLayer, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            // deleteLayerOrMask: the mask target deletes the mask; otherwise
            // the whole selection goes through one undo (SelectionEdits.swift:71).
            let mask_delete = t.session.is_mask_selected
                && t.session
                    .active_layer()
                    .map(|layer| layer.mask.is_some())
                    .unwrap_or(false)
                && t.session.selected_layer_ids.len() <= 1;
            if mask_delete {
                t.session.delete_layer_mask();
            } else {
                t.session.delete_selected_layers();
            }
            t.touch(cx);
        });
    }
    fn on_merge_down(&mut self, _: &keys::MergeDown, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.merge_layers();
        self.touch(cx);
    }
    fn on_new_group(&mut self, _: &keys::NewGroup, _w: &mut Window, cx: &mut Context<Self>) {
        self.add_group(cx);
    }
    fn on_add_mask(&mut self, _: &keys::AddMask, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.add_layer_mask(true);
        self.touch(cx);
    }
    fn on_flip_layer_h(&mut self, _: &keys::FlipLayerHorizontal, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.flip_layers(true);
        self.touch(cx);
    }
    fn on_flip_layer_v(&mut self, _: &keys::FlipLayerVertical, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.flip_layers(false);
        self.touch(cx);
    }
    fn on_group_selected_layers(&mut self, _: &keys::GroupSelectedLayers, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.group_selected_layers();
        self.touch(cx);
    }
    fn on_move_out_of_folder(&mut self, _: &keys::MoveOutOfFolder, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.move_active_layer_out_of_group();
        self.touch(cx);
    }
    fn on_rename_layer(&mut self, _: &keys::RenameLayer, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.session.active_layer_id {
            self.begin_rename(id, w, cx);
        }
    }
    fn on_toggle_layer_visibility(&mut self, _: &keys::ToggleLayerVisibility, _w: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.session.active_layer_id {
            self.session.toggle_layer_visibility(id);
            self.touch(cx);
        }
    }
    fn add_adjustment(&mut self, kind: AdjustmentKind, cx: &mut Context<Self>) {
        self.session.add_adjustment(kind);
        self.touch(cx);
    }
    fn on_add_adjustment_hsv(&mut self, _: &keys::AddAdjustmentHsv, _w: &mut Window, cx: &mut Context<Self>) {
        self.add_adjustment(AdjustmentKind::Hsv, cx);
    }
    fn on_add_adjustment_levels(&mut self, _: &keys::AddAdjustmentLevels, _w: &mut Window, cx: &mut Context<Self>) {
        self.add_adjustment(AdjustmentKind::Levels, cx);
    }
    fn on_add_adjustment_curves(&mut self, _: &keys::AddAdjustmentCurves, _w: &mut Window, cx: &mut Context<Self>) {
        self.add_adjustment(AdjustmentKind::Curves, cx);
    }
    fn on_add_adjustment_exposure(&mut self, _: &keys::AddAdjustmentExposure, _w: &mut Window, cx: &mut Context<Self>) {
        self.add_adjustment(AdjustmentKind::Exposure, cx);
    }
    fn on_add_adjustment_gradient_map(&mut self, _: &keys::AddAdjustmentGradientMap, _w: &mut Window, cx: &mut Context<Self>) {
        self.add_adjustment(AdjustmentKind::GradientMap, cx);
    }
    fn on_add_adjustment_grain(&mut self, _: &keys::AddAdjustmentGrain, _w: &mut Window, cx: &mut Context<Self>) {
        self.add_adjustment(AdjustmentKind::Grain, cx);
    }
    fn on_add_adjustment_add_noise(&mut self, _: &keys::AddAdjustmentAddNoise, _w: &mut Window, cx: &mut Context<Self>) {
        self.add_adjustment(AdjustmentKind::AddNoise, cx);
    }
    fn on_add_adjustment_gaussian_blur(&mut self, _: &keys::AddAdjustmentGaussianBlur, _w: &mut Window, cx: &mut Context<Self>) {
        self.add_adjustment(AdjustmentKind::GaussianBlur, cx);
    }
    fn on_add_adjustment_motion_blur(&mut self, _: &keys::AddAdjustmentMotionBlur, _w: &mut Window, cx: &mut Context<Self>) {
        self.add_adjustment(AdjustmentKind::MotionBlur, cx);
    }
    fn on_add_adjustment_invert(&mut self, _: &keys::AddAdjustmentInvert, _w: &mut Window, cx: &mut Context<Self>) {
        self.add_adjustment(AdjustmentKind::Invert, cx);
    }
    fn on_add_adjustment_black_white(&mut self, _: &keys::AddAdjustmentBlackWhite, _w: &mut Window, cx: &mut Context<Self>) {
        self.add_adjustment(AdjustmentKind::BlackWhite, cx);
    }
    fn on_add_adjustment_color_balance(&mut self, _: &keys::AddAdjustmentColorBalance, _w: &mut Window, cx: &mut Context<Self>) {
        self.add_adjustment(AdjustmentKind::ColorBalance, cx);
    }
    fn on_edit_adjustment(&mut self, _: &keys::EditAdjustment, w: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.session.active_layer_id else {
            return;
        };
        let Some(adjustment) = self
            .session
            .active_layer()
            .and_then(|layer| layer.adjustment.clone())
        else {
            return;
        };
        // Invert has no settings to edit.
        if adjustment.kind == AdjustmentKind::Invert
            || !self.session.begin_adjustment_editing(id)
        {
            return;
        }
        // Prefill the editor from the layer's current settings.
        match adjustment.kind {
            AdjustmentKind::Levels => {
                self.session.levels = Some(adjustment.levels.clone());
                self.open_sheet(SheetKind::Levels, w, cx);
            }
            AdjustmentKind::Hsv => {
                self.session.hue_saturation =
                    Some(adjustment.hsv_settings.clone().unwrap_or_else(|| {
                        compositor_core::adjust::HueSaturationSettings::new(
                            adjustment.hue,
                            adjustment.saturation,
                            adjustment.lightness,
                            adjustment.colorize,
                        )
                    }));
                self.open_sheet(SheetKind::HueSaturation, w, cx);
            }
            other => {
                if let Some(filter) = other.filter_kind() {
                    self.session.filter_edit = Some(filter_settings_from_adjustment(&adjustment));
                    self.open_sheet(SheetKind::Filter(filter), w, cx);
                }
            }
        }
    }

    // MARK: Select

    fn on_select_all(&mut self, _: &keys::SelectAllAction, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.select_all();
        self.touch(cx);
    }
    fn on_deselect(&mut self, _: &keys::DeselectAction, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.deselect();
        self.touch(cx);
    }
    fn on_invert_selection(&mut self, _: &keys::InvertSelection, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.invert_selection();
        self.touch(cx);
    }
    fn on_feather(&mut self, _: &keys::FeatherSheet, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Amount(SelectionAmountOperation::Feather), _w, cx);
    }
    fn on_expand(&mut self, _: &keys::ExpandSheet, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Amount(SelectionAmountOperation::Expand), _w, cx);
    }
    fn on_contract(&mut self, _: &keys::ContractSheet, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Amount(SelectionAmountOperation::Contract), _w, cx);
    }

    // MARK: Filter

    fn on_filter_blur(&mut self, _: &keys::FilterGaussianBlur, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Filter(FilterKind::GaussianBlur), _w, cx);
    }
    fn on_filter_motion(&mut self, _: &keys::FilterMotionBlur, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Filter(FilterKind::MotionBlur), _w, cx);
    }
    fn on_filter_noise(&mut self, _: &keys::FilterAddNoise, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Filter(FilterKind::AddNoise), _w, cx);
    }
    fn on_filter_lens(&mut self, _: &keys::FilterLensCorrection, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Filter(FilterKind::LensCorrection), _w, cx);
    }
    fn on_filter_camera_raw(&mut self, _: &keys::FilterCameraRaw, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Filter(FilterKind::CameraRaw), _w, cx);
    }

    // MARK: View

    fn on_toggle_rulers(&mut self, _: &keys::ToggleRulers, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.shows_rulers = !self.session.shows_rulers;
        self.touch(cx);
    }
    fn on_toggle_grid(&mut self, _: &keys::ToggleGrid, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.shows_grid = !self.session.shows_grid;
        self.touch(cx);
    }
    fn on_toggle_guides(&mut self, _: &keys::ToggleGuides, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.shows_guides = !self.session.shows_guides;
        self.touch(cx);
    }
    fn on_toggle_pixel_grid(&mut self, _: &keys::TogglePixelGrid, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.shows_pixel_grid = !self.session.shows_pixel_grid;
        self.touch(cx);
    }
    fn on_snap_guides(&mut self, _: &keys::SnapGuides, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.snap_to_guides = !self.session.snap_to_guides;
        self.touch(cx);
    }
    fn on_snap_toggle(&mut self, _: &keys::SnapToggle, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.snapping_enabled = !self.session.snapping_enabled;
        self.touch(cx);
    }
    fn on_lock_guides(&mut self, _: &keys::LockGuides, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.locks_guides = !self.session.locks_guides;
        self.touch(cx);
    }
    fn on_snap_grid(&mut self, _: &keys::SnapGrid, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.snap_to_grid = !self.session.snap_to_grid;
        self.touch(cx);
    }
    fn on_snap_layers(&mut self, _: &keys::SnapLayers, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.snap_to_layers = !self.session.snap_to_layers;
        self.touch(cx);
    }
    fn on_snap_bounds(&mut self, _: &keys::SnapBounds, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.snap_to_document_bounds = !self.session.snap_to_document_bounds;
        self.touch(cx);
    }
    fn on_zoom_in(&mut self, _: &keys::ZoomIn, _w: &mut Window, cx: &mut Context<Self>) {
        self.zoom_at_viewport(2.0);
        self.touch(cx);
    }
    fn on_zoom_out(&mut self, _: &keys::ZoomOut, _w: &mut Window, cx: &mut Context<Self>) {
        self.zoom_at_viewport(0.5);
        self.touch(cx);
    }
    fn on_zoom_fit(&mut self, _: &keys::ZoomFit, _w: &mut Window, cx: &mut Context<Self>) {
        self.zoom_fit(self.viewport_size());
        self.touch(cx);
    }
    fn on_zoom_100(&mut self, _: &keys::Zoom100, _w: &mut Window, cx: &mut Context<Self>) {
        self.zoom = 1.0;
        self.revision += 1;
        self.touch(cx);
    }

    // MARK: Window / Help

    fn on_reset_panels(&mut self, _: &keys::ResetPanels, _w: &mut Window, cx: &mut Context<Self>) {
        self.set_status("Panel layout is fixed in this build", cx);
    }
    fn on_about(&mut self, _: &keys::About, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::About, _w, cx);
    }

    // MARK: Canvas keys

    fn on_nudge_left(&mut self, _: &keys::NudgeLeft, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge(-1.0, 0.0, false, w, cx);
    }
    fn on_nudge_right(&mut self, _: &keys::NudgeRight, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge(1.0, 0.0, false, w, cx);
    }
    fn on_nudge_up(&mut self, _: &keys::NudgeUp, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge(0.0, -1.0, false, w, cx);
    }
    fn on_nudge_down(&mut self, _: &keys::NudgeDown, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge(0.0, 1.0, false, w, cx);
    }
    fn on_nudge_left_more(&mut self, _: &keys::NudgeLeftMore, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge(-10.0, 0.0, true, w, cx);
    }
    fn on_nudge_right_more(&mut self, _: &keys::NudgeRightMore, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge(10.0, 0.0, true, w, cx);
    }
    fn on_nudge_up_more(&mut self, _: &keys::NudgeUpMore, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge(0.0, -10.0, true, w, cx);
    }
    fn on_nudge_down_more(&mut self, _: &keys::NudgeDownMore, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge(0.0, 10.0, true, w, cx);
    }
    fn on_brush_smaller(&mut self, _: &keys::BrushSmaller, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, w, cx| t.resize_brush(1.0 / 1.25, w, cx));
    }
    fn on_brush_larger(&mut self, _: &keys::BrushLarger, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, w, cx| t.resize_brush(1.25, w, cx));
    }
    fn on_brush_softer(&mut self, _: &keys::BrushSofter, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, w, cx| t.change_brush_hardness(false, w, cx));
    }
    fn on_brush_harder(&mut self, _: &keys::BrushHarder, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, w, cx| t.change_brush_hardness(true, w, cx));
    }
    fn on_blend_mode_previous(&mut self, _: &keys::BlendModePrevious, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.cycle_blend_mode(false);
        cx.notify();
    }
    fn on_blend_mode_next(&mut self, _: &keys::BlendModeNext, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.cycle_blend_mode(true);
        cx.notify();
    }
    fn on_cycle_shape_kind(&mut self, _: &keys::CycleShapeKind, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.toggle_shape_kind();
        cx.notify();
    }
    fn on_show_transform_controls(&mut self, _: &keys::ShowTransformControls, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.shows_transform_controls = !self.session.shows_transform_controls;
        self.touch(cx);
    }
    fn on_invert_pixels(&mut self, _: &keys::InvertPixels, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.invert_pixels();
        self.touch(cx);
    }
    fn on_toggle_clipping_mask(&mut self, _: &keys::ToggleClippingMask, _w: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.session.active_layer_id {
            self.session.toggle_clipping_mask(id);
        }
        self.touch(cx);
    }
    fn on_move_layer_down(&mut self, _: &keys::MoveLayerDown, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.move_active_layer(-1);
        self.touch(cx);
    }
    fn on_move_layer_up(&mut self, _: &keys::MoveLayerUp, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.move_active_layer(1);
        self.touch(cx);
    }
    fn on_cut(&mut self, _: &keys::Cut, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.session.cut_selection();
            t.touch(cx);
        });
    }
    fn on_copy_pixels(&mut self, _: &keys::CopyPixels, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.session.copy_selection();
            t.touch(cx);
        });
    }
    fn on_copy_merged(&mut self, _: &keys::CopyMerged, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.session.copy_merged_selection();
            t.touch(cx);
        });
    }
    fn on_paste_pixels(&mut self, _: &keys::PastePixels, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.session.paste_pixels();
            t.touch(cx);
        });
    }
    fn on_content_aware_fill(&mut self, _: &keys::ContentAwareFill, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.run_content_aware_fill(cx));
    }
    fn on_clear_selection_pixels(&mut self, _: &keys::ClearSelectionPixels, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.session.clear_selected_pixels();
            t.touch(cx);
        });
    }
    fn on_load_layer_selection(&mut self, _: &keys::LoadLayerSelection, _w: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.session.active_layer_id {
            self.session
                .load_layer_selection(id, compositor_core::selection::SelectionMode::Replace);
            self.touch(cx);
        }
    }
    fn on_load_mask_selection(&mut self, _: &keys::LoadMaskSelection, _w: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.session.active_layer_id {
            self.session
                .load_mask_selection(id, compositor_core::selection::SelectionMode::Replace);
            self.touch(cx);
        }
    }
    fn on_remove_background(&mut self, _: &keys::RemoveBackground, _w: &mut Window, cx: &mut Context<Self>) {
        // Runs on the layer directly: no dialog, one undo (FilterKind menu).
        let Some(base) = self
            .session
            .active_layer()
            .and_then(|layer| layer.asset.as_ref().map(|asset| asset.image.clone()))
        else {
            return;
        };
        let job = compositor_core::filters::FilterJob {
            kind: FilterKind::RemoveBackground,
            image: &base,
            settings: compositor_core::filters::FilterSettings::default(),
            scale: 1.0,
            selection: self.selection_clip(),
            mapping: compositor_core::geometry::CGAffineTransform::IDENTITY,
            seed: 7,
            noise_origin: compositor_core::geometry::CGPoint::ZERO,
            camera_raw_clipping: None,
            visualize_point_color: -1,
            sharpen_mask: false,
        };
        let result = compositor_core::filters::run_filter(&job);
        if let Some(id) = self.session.active_layer_id {
            self.session.history.begin(
                "Remove Background",
                &self.session.document,
                self.session.active_layer_id,
            );
            if let Some(doc) = self.session.document.as_mut() {
                if let Some(layer) = doc.layer_mut(id) {
                    layer.asset = Some(compositor_core::raster::ImportedImage::new(result, layer.name.clone()));
                }
            }
            self.session
                .history
                .end(&self.session.document, self.session.active_layer_id);
        }
        self.touch(cx);
    }
    fn on_clear_guides(&mut self, _: &keys::ClearGuides, _w: &mut Window, cx: &mut Context<Self>) {
        self.session.clear_guides();
        self.touch(cx);
    }
    fn on_toggle_levels_preview(&mut self, _: &keys::ToggleLevelsPreview, _w: &mut Window, cx: &mut Context<Self>) {
        self.toggle_levels_preview(cx);
    }
    fn on_finish_lasso(&mut self, _: &keys::FinishLasso, _w: &mut Window, cx: &mut Context<Self>) {
        if self.session.lasso_draft.is_some() {
            self.session.finish_lasso();
            self.touch(cx);
        }
    }
    fn on_cancel(&mut self, _: &keys::CancelAction, _w: &mut Window, cx: &mut Context<Self>) {
        self.cancel_active(cx);
    }

    // MARK: Shared handler helpers

    /// Runs a hotkey body only when no text input or sheet owns the keyboard.
    fn hotkey(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        body: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>),
    ) {
        if self.canvas_keys_active(window, cx) {
            body(self, window, cx);
        }
    }

    pub(crate) fn zoom_at_viewport(&mut self, factor: f64) {
        let size = self.viewport_size();
        self.zoom_at((size.0 / 2.0, size.1 / 2.0), factor);
    }

    pub(crate) fn viewport_size(&self) -> (f32, f32) {
        (
            f32::from(self.viewport.size.width),
            f32::from(self.viewport.size.height),
        )
    }

    fn nudge(&mut self, dx: f64, dy: f64, large: bool, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            let scale = if large { 10.0 } else { 1.0 };
            if t.session.transform_edit.is_some()
                || (t.session.tool == NavigationTool::Move && t.session.can_transform())
            {
                t.session.nudge_layer(dx * scale, dy * scale);
            } else {
                t.session.nudge_selection(dx * scale, dy * scale);
            }
            t.touch(cx);
        });
    }

    fn render_status_bar(&mut self, _cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let (width, height) = self.doc_size();
        let tool = self.session.tool.label().to_string();
        let zoom = format!("{:.0}%", self.zoom * 100.0);
        let undo = self.session.history.undo_name().to_string();
        let redo = self.session.history.redo_name().to_string();
        let undo_text = if self.session.can_undo() {
            format!("Undo: {}", undo)
        } else {
            "Undo: \u{2014}".to_string()
        };
        let redo_text = if self.session.can_redo() {
            format!("Redo: {}", redo)
        } else {
            "Redo: \u{2014}".to_string()
        };
        let message = self
            .status
            .clone()
            .or_else(|| self.session.brush_error.clone())
            .unwrap_or_default();

        div()
            .id("status-bar")
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(24.0))
            .px_2()
            .flex_shrink_0()
            .bg(rgb(crate::ui::BG_PANEL))
            .border_t_1()
            .border_color(gpui_kit::black().opacity(0.4))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_3()
                    .child(text_label(
                        format!(
                            "{} \u{d7} {} px",
                            width as usize, height as usize
                        ),
                        false,
                    ))
                    .child(text_label(zoom, false))
                    .child(text_label(tool, true)),
            )
            .child(
                div()
                    .flex_1()
                    .px_3()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(text_label(message, true).text_color(rgb(FG_DIM))),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_3()
                    .child(text_label(undo_text, true))
                    .child(text_label(redo_text, true)),
            )
            .into_any_element()
    }
}
