//! The workspace: window layout, the action dispatch for every menu item and
//! keyboard shortcut, and the status bar.

use gpui_kit::{div, px, rgb, Context, IntoElement, InteractiveElement, ParentElement, Render, Styled, Window};

use compositor_core::adjustment::AdjustmentKind;
use compositor_core::document::NavigationTool;
use compositor_core::filters::FilterKind;
use compositor_core::session::SelectionAmountOperation;

use crate::keys;
use crate::project::{Pending, Step};
use crate::state::{AppState, SaveAfter, SheetKind};
use crate::ui::{text_label, FG_DIM};

impl Render for AppState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Startup load of the persisted tool toggles (ToolDefaults.swift):
        // the first render of the freshly opened window is the earliest point
        // this file owns, and it runs before any input can reach the app.
        // Once per process; a no-op afterwards.
        crate::settings::apply_startup_defaults(
            &mut self.workspace.current_mut().session,
            &mut self.transform_auto_select,
        );
        // Drains: workspace-level errors (failed opens) surface as the Alert
        // sheet; queued file receives run once the guards cleared.
        self.drain_workspace_errors(window, cx);
        self.drain_queued_receives();
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
            .on_action(cx.listener(Self::on_quit))
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
            .on_action(cx.listener(Self::on_nudge_pixels_left))
            .on_action(cx.listener(Self::on_nudge_pixels_right))
            .on_action(cx.listener(Self::on_nudge_pixels_up))
            .on_action(cx.listener(Self::on_nudge_pixels_down))
            .on_action(cx.listener(Self::on_nudge_pixels_left_more))
            .on_action(cx.listener(Self::on_nudge_pixels_right_more))
            .on_action(cx.listener(Self::on_nudge_pixels_up_more))
            .on_action(cx.listener(Self::on_nudge_pixels_down_more))
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
            .child(self.render_tab_bar(cx))
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

/// `projectTabLabelWidth` (ProjectTabs.swift:59-64): the title's rendered
/// width plus the 10px unsaved dot when it shows, clamped to 35...155. The
/// port estimates the title width at 7px per character (the window's 12px
/// UI face) — gpui lays the label out itself, so the number drives the
/// pill's width cap rather than an exact frame.
pub fn project_tab_label_width(title: &str, modified: bool) -> f32 {
    let title_width = title.chars().count() as f32 * 7.0;
    let dot_width = if modified { 10.0 } else { 0.0 };
    155.0f32.min(35.0f32.max((title_width + dot_width).ceil()))
}

/// `projectTabPillWidth` (:66-69): the label plus 11px leading, 8px
/// trailing, the 16px close button and the 5px gap after it.
pub fn project_tab_pill_width(title: &str, modified: bool) -> f32 {
    project_tab_label_width(title, modified) + 40.0
}

#[cfg(test)]
mod tab_width_tests {
    use super::{project_tab_label_width, project_tab_pill_width};

    /// The clamp from ProjectTabs.swift:59-64.
    #[test]
    fn tab_label_width_clamps_like_swift() {
        // Short titles bottom out at 35.
        assert_eq!(project_tab_label_width("", false), 35.0);
        assert_eq!(project_tab_label_width("Untitled", false), 56.0);
        // The unsaved dot adds 10 (5px dot and 5px gap).
        assert_eq!(project_tab_label_width("Untitled", true), 66.0);
        // Long titles cap at 155.
        let long = "a".repeat(40);
        assert_eq!(project_tab_label_width(&long, true), 155.0);
    }

    /// The pill adds 40 for padding + the close button (:66-69).
    #[test]
    fn tab_pill_width_adds_forty() {
        assert_eq!(project_tab_pill_width("Untitled", false), 96.0);
        assert_eq!(project_tab_pill_width("Untitled", true), 106.0);
        assert_eq!(project_tab_pill_width(&"x".repeat(60), false), 195.0);
    }
}

impl AppState {
    /// The 34px project tab strip (`ProjectTabs.swift`'s `ProjectTabStrip`):
    /// one pill per tab — title, unsaved dot, close button — plus a "New"
    /// slot. Clicking selects (`workspace.select`, refitting the viewport);
    /// tabs and close buttons disable while `canSwitch` is false.
    pub fn render_tab_bar(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        use gpui_kit::SharedString;
        use gpui_kit::component::button::{Button, ButtonVariants};
        use gpui_kit::component::tab::{Tab, TabBar};
        use gpui_kit::component::{Disableable, Selectable};

        let can_switch = self.workspace.can_switch();
        let selected = self.workspace.selected_id();
        let tabs: Vec<(crate::project::TabId, String, bool)> = self
            .workspace
            .tabs()
            .iter()
            .map(|tab| (tab.id, tab.title(), tab.session.is_modified()))
            .collect();

        let mut bar = TabBar::new("project-tabs")
            .pill()
            .max_width(px(project_tab_pill_width("x".repeat(60).as_str(), true)));
        for (id, title, modified) in tabs {
            let active = id == selected;
            let mut tab = Tab::new()
                .label(title.clone())
                .selected(active)
                .disabled(!can_switch && !active)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.workspace.select(id) {
                        this.request_refit();
                        this.touch(cx);
                    }
                }));
            if modified {
                tab = tab.prefix(crate::flows::unsaved_dot());
            }
            tab = tab.suffix(
                Button::new(SharedString::from(format!("tab-close-{}", id)))
                    .label("\u{d7}")
                    .ghost()
                    .compact()
                    .disabled(!can_switch)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        match this.workspace.close(id) {
                            Step::Pending(pending) => this.present_pending(pending, window, cx),
                            Step::Done(()) => {
                                this.request_refit();
                                this.touch(cx);
                            }
                        }
                    })),
            );
            bar = bar.child(tab);
        }
        // The "New" slot (`NewTabDropSlot`, ProjectTabs.swift:200-216) is a
        // drop target in Swift; v1 makes it a plain New-tab button (the
        // workspace design's deviation #3).
        bar = bar.suffix(
            Button::new("tab-new")
                .label("+ New")
                .ghost()
                .compact()
                .disabled(!can_switch)
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.workspace.can_switch() {
                        this.workspace.add_tab(true);
                        this.request_refit();
                        this.touch(cx);
                    }
                })),
        );

        gpui_kit::div()
            .flex()
            .items_center()
            .h(px(34.0))
            .px_1()
            .flex_shrink_0()
            .bg(crate::ui::panel_bg())
            .border_b_1()
            .border_color(gpui_kit::black().opacity(0.4))
            .child(bar)
            .into_any_element()
    }
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

    /// Saves the persisted tool toggles (`settings.rs`): Swift's
    /// `EditorSession` properties write each change straight through
    /// `didSet { ToolDefaults.set(...) }`; the port snapshots the whole table
    /// at the change point and stores it atomically. Every handler below that
    /// flips one of the persisted switches calls this right after mutating
    /// `self.tab().session` / `self.transform_auto_select`, which is also why no
    /// explicit exit save is wired: the table on disk is always current.
    pub fn persist_tool_defaults(&mut self) {
        let session = self.current_tab();
        crate::settings::persist_session(&session.session, self.transform_auto_select);
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
            t.tab().session.swap_palette_colors();
            t.touch(cx);
        });
    }
    fn on_default_colors(&mut self, _: &keys::DefaultColors, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.tab().session.reset_palette_colors();
            t.touch(cx);
        });
    }
    fn on_cycle_tool_mode(&mut self, _: &keys::CycleToolMode, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.cycle_tool_mode(cx));
    }

    // MARK: File

    fn on_new_canvas(&mut self, _: &keys::NewCanvas, _w: &mut Window, cx: &mut Context<Self>) {
        // `newCanvas` (:53-57): a fresh tab, no modal.
        if self.workspace.new_canvas() {
            self.request_refit();
            self.touch(cx);
        }
    }
    fn on_open(&mut self, _: &keys::OpenDocument, w: &mut Window, cx: &mut Context<Self>) {
        // `open(_:)` (:58-76), two-phase: the dialog ask comes back as
        // `Pending::Open` and the Open sheet plays the file picker's part
        // (semicolon-separated paths stand in for the multi-select panel).
        match self.workspace.open(None) {
            Step::Pending(pending) => self.present_pending(pending, w, cx),
            Step::Done(_) => {}
        }
    }
    fn on_save(&mut self, _: &keys::SaveDocument, w: &mut Window, cx: &mut Context<Self>) {
        self.start_save(false, w, cx);
    }
    fn on_save_as(&mut self, _: &keys::SaveAs, w: &mut Window, cx: &mut Context<Self>) {
        self.start_save(true, w, cx);
    }
    /// `save(asNew:)` (:25-29): a known destination writes straight to disk;
    /// otherwise the save panel's pending surfaces and the sheet finishes it.
    fn start_save(&mut self, as_new: bool, w: &mut Window, cx: &mut Context<Self>) {
        let result = self.with_current_tab(|controller, session, busy| {
            controller.save(session, busy, as_new)
        });
        match result {
            Step::Pending(Pending::SaveAs { default_name, .. }) => {
                self.pending_save =
                    Some((self.workspace.selected_id(), SaveAfter::Nothing));
                self.present_pending(
                    Pending::SaveAs {
                        as_new,
                        title: String::new(),
                        default_name,
                    },
                    w,
                    cx,
                );
            }
            Step::Pending(pending) => self.present_pending(pending, w, cx),
            Step::Done(ok) => {
                self.drain_controller_error(w, cx);
                if ok {
                    self.set_status("Saved", cx);
                }
            }
        }
    }
    fn on_export(&mut self, _: &keys::ExportPng, w: &mut Window, cx: &mut Context<Self>) {
        match self.with_current_tab(|controller, session, busy| {
            controller.export_png(session, busy)
        }) {
            Step::Pending(pending) => self.present_pending(pending, w, cx),
            Step::Done(()) => {}
        }
    }
    fn on_export_jpeg(&mut self, _: &keys::ExportJpeg, w: &mut Window, cx: &mut Context<Self>) {
        match self.with_current_tab(|controller, session, busy| {
            controller.export_jpeg(session, busy)
        }) {
            Step::Pending(pending) => self.present_pending(pending, w, cx),
            Step::Done(()) => {}
        }
    }
    fn on_import_images(&mut self, _: &keys::ImportImages, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Import, _w, cx);
    }
    fn on_close_document(&mut self, _: &keys::CloseDocument, w: &mut Window, cx: &mut Context<Self>) {
        // `close(_:)` (:89-96): confirm unsaved changes, then remove the tab.
        match self.workspace.close(self.workspace.selected_id()) {
            Step::Pending(pending) => self.present_pending(pending, w, cx),
            Step::Done(()) => {
                self.request_refit();
                self.touch(cx);
            }
        }
    }
    fn on_quit(&mut self, _: &keys::Quit, w: &mut Window, cx: &mut Context<Self>) {
        // `confirmQuit` (:106-114): ask each tab in order; on success the
        // window closes.
        match self.workspace.confirm_quit() {
            Step::Pending(pending) => self.present_pending(pending, w, cx),
            Step::Done(true) => w.remove_window(),
            Step::Done(false) => {}
        }
    }

    // MARK: Edit

    fn on_undo(&mut self, _: &keys::Undo, _w: &mut Window, cx: &mut Context<Self>) {
        // A restored document of a different size refits the viewport
        // (EditorSession.undo → viewport.fit).
        let before = self.tab().session.document.as_ref().map(|d| (d.id, d.width, d.height));
        self.tab().session.undo();
        self.refit_if_document_changed(before);
        self.touch(cx);
    }
    fn on_redo(&mut self, _: &keys::Redo, _w: &mut Window, cx: &mut Context<Self>) {
        let before = self.tab().session.document.as_ref().map(|d| (d.id, d.width, d.height));
        self.tab().session.redo();
        self.refit_if_document_changed(before);
        self.touch(cx);
    }
    /// Refits the viewport when undo/redo swapped in a different document.
    fn refit_if_document_changed(&mut self, before: Option<(uuid::Uuid, usize, usize)>) {
        let after = self.tab().session.document.as_ref().map(|d| (d.id, d.width, d.height));
        if before.is_some() && before != after {
            let vw = f32::from(self.viewport.size.width);
            let vh = f32::from(self.viewport.size.height);
            if vw > 10.0 && vh > 10.0 {
                self.zoom_fit((vw, vh));
            }
        }
    }
    fn on_fill_foreground(&mut self, _: &keys::FillForeground, _w: &mut Window, cx: &mut Context<Self>) {
        let color = color_bytes(self.tab().session.palette_color(false));
        self.fill_active(color, "Fill Foreground", cx);
    }
    fn on_fill_background(&mut self, _: &keys::FillBackground, _w: &mut Window, cx: &mut Context<Self>) {
        let color = color_bytes(self.tab().session.palette_color(true));
        self.fill_active(color, "Fill Background", cx);
    }
    fn on_free_transform(&mut self, _: &keys::FreeTransform, _w: &mut Window, cx: &mut Context<Self>) {
        // transformCommand: the selection floats when there is one (FloatingSelection.swift:22).
        if self.tab().session.can_transform_selection() {
            self.tab().session.begin_selection_transform();
        } else {
            self.tab().session.begin_transform();
        }
        self.touch(cx);
    }
    fn on_shortcuts(&mut self, _: &keys::ShowShortcuts, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(SheetKind::Shortcuts, _w, cx);
    }

    // MARK: Image

    fn on_image_size(&mut self, _: &keys::ImageSize, w: &mut Window, cx: &mut Context<Self>) {
        // `imageSize()` (:76-87): guard + begin, then the Image Size sheet.
        match self.with_current_tab(|controller, session, busy| {
            controller.image_size(session, busy)
        }) {
            Step::Pending(pending) => self.present_pending(pending, w, cx),
            Step::Done(()) => {}
        }
    }
    fn on_canvas_size(&mut self, _: &keys::CanvasSize, w: &mut Window, cx: &mut Context<Self>) {
        match self.with_current_tab(|controller, session, busy| {
            controller.canvas_size(session, busy)
        }) {
            Step::Pending(pending) => self.present_pending(pending, w, cx),
            Step::Done(()) => {}
        }
    }
    fn on_trim(&mut self, _: &keys::TrimSheet, w: &mut Window, cx: &mut Context<Self>) {
        match self.with_current_tab(|controller, session, busy| controller.trim(session, busy)) {
            Step::Pending(pending) => self.present_pending(pending, w, cx),
            Step::Done(()) => {}
        }
    }
    fn on_crop_to_selection(&mut self, _: &keys::CropToSelection, _w: &mut Window, cx: &mut Context<Self>) {
        if let Some(selection) = self.tab().session.selection() {
            if let Some(bounds) = selection.path.bounds() {
                self.tab().session.crop_rect = Some(bounds);
                self.set_status("Crop to selection: press Apply in the Crop header", cx);
            }
        }
    }
    fn on_flip_canvas_h(&mut self, _: &keys::FlipCanvasHorizontal, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.flip_canvas(true);
        self.touch(cx);
    }
    fn on_flip_canvas_v(&mut self, _: &keys::FlipCanvasVertical, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.flip_canvas(false);
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
        self.tab().session.add_blank_layer();
        self.touch(cx);
    }
    fn on_duplicate_layer(&mut self, _: &keys::DuplicateLayer, _w: &mut Window, cx: &mut Context<Self>) {
        // layerViaCopy duplicates when nothing is selected (SelectionClipboard.swift:138).
        self.tab().session.layer_via_copy();
        self.touch(cx);
    }
    fn on_delete_layer(&mut self, _: &keys::DeleteLayer, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            // deleteLayerOrMask: the mask target deletes the mask; otherwise
            // the whole selection goes through one undo (SelectionEdits.swift:71).
            let mask_delete = t.tab().session.is_mask_selected
                && t.tab().session
                    .active_layer()
                    .map(|layer| layer.mask.is_some())
                    .unwrap_or(false)
                && t.tab().session.selected_layer_ids.len() <= 1;
            if mask_delete {
                t.tab().session.delete_layer_mask();
            } else {
                t.tab().session.delete_selected_layers();
            }
            t.touch(cx);
        });
    }
    fn on_merge_down(&mut self, _: &keys::MergeDown, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.merge_layers();
        self.touch(cx);
    }
    fn on_new_group(&mut self, _: &keys::NewGroup, _w: &mut Window, cx: &mut Context<Self>) {
        self.add_group(cx);
    }
    fn on_add_mask(&mut self, _: &keys::AddMask, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.add_layer_mask(true);
        self.touch(cx);
    }
    fn on_flip_layer_h(&mut self, _: &keys::FlipLayerHorizontal, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.flip_layers(true);
        self.touch(cx);
    }
    fn on_flip_layer_v(&mut self, _: &keys::FlipLayerVertical, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.flip_layers(false);
        self.touch(cx);
    }
    fn on_group_selected_layers(&mut self, _: &keys::GroupSelectedLayers, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.group_selected_layers();
        self.touch(cx);
    }
    fn on_move_out_of_folder(&mut self, _: &keys::MoveOutOfFolder, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.move_active_layer_out_of_group();
        self.touch(cx);
    }
    fn on_rename_layer(&mut self, _: &keys::RenameLayer, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.tab().session.active_layer_id {
            self.begin_rename(id, w, cx);
        }
    }
    fn on_toggle_layer_visibility(&mut self, _: &keys::ToggleLayerVisibility, _w: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.tab().session.active_layer_id {
            self.tab().session.toggle_layer_visibility(id);
            self.touch(cx);
        }
    }
    fn add_adjustment(&mut self, kind: AdjustmentKind, cx: &mut Context<Self>) {
        self.tab().session.add_adjustment(kind);
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
        let Some(id) = self.tab().session.active_layer_id else {
            return;
        };
        let Some(adjustment) = self
            .tab().session
            .active_layer()
            .and_then(|layer| layer.adjustment.clone())
        else {
            return;
        };
        // Invert has no settings to edit.
        if adjustment.kind == AdjustmentKind::Invert
            || !self.tab().session.begin_adjustment_editing(id)
        {
            return;
        }
        // Prefill the editor from the layer's current settings.
        match adjustment.kind {
            AdjustmentKind::Levels => {
                self.tab().session.levels = Some(adjustment.levels.clone());
                self.open_sheet(SheetKind::Levels, w, cx);
            }
            AdjustmentKind::Hsv => {
                self.tab().session.hue_saturation =
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
                    self.tab().session.filter_edit = Some(filter_settings_from_adjustment(&adjustment));
                    self.open_sheet(SheetKind::Filter(filter), w, cx);
                }
            }
        }
    }

    // MARK: Select

    fn on_select_all(&mut self, _: &keys::SelectAllAction, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.select_all();
        self.touch(cx);
    }
    fn on_deselect(&mut self, _: &keys::DeselectAction, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.deselect();
        self.touch(cx);
    }
    fn on_invert_selection(&mut self, _: &keys::InvertSelection, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.invert_selection();
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
        self.tab().session.shows_rulers = !self.tab().session.shows_rulers;
        self.persist_tool_defaults();
        self.touch(cx);
    }
    fn on_toggle_grid(&mut self, _: &keys::ToggleGrid, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.shows_grid = !self.tab().session.shows_grid;
        self.persist_tool_defaults();
        self.touch(cx);
    }
    fn on_toggle_guides(&mut self, _: &keys::ToggleGuides, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.shows_guides = !self.tab().session.shows_guides;
        self.persist_tool_defaults();
        self.touch(cx);
    }
    fn on_toggle_pixel_grid(&mut self, _: &keys::TogglePixelGrid, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.shows_pixel_grid = !self.tab().session.shows_pixel_grid;
        self.persist_tool_defaults();
        self.touch(cx);
    }
    fn on_snap_guides(&mut self, _: &keys::SnapGuides, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.snap_to_guides = !self.tab().session.snap_to_guides;
        self.persist_tool_defaults();
        self.touch(cx);
    }
    fn on_snap_toggle(&mut self, _: &keys::SnapToggle, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.snapping_enabled = !self.tab().session.snapping_enabled;
        self.persist_tool_defaults();
        self.touch(cx);
    }
    fn on_lock_guides(&mut self, _: &keys::LockGuides, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.locks_guides = !self.tab().session.locks_guides;
        self.persist_tool_defaults();
        self.touch(cx);
    }
    fn on_snap_grid(&mut self, _: &keys::SnapGrid, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.snap_to_grid = !self.tab().session.snap_to_grid;
        self.persist_tool_defaults();
        self.touch(cx);
    }
    fn on_snap_layers(&mut self, _: &keys::SnapLayers, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.snap_to_layers = !self.tab().session.snap_to_layers;
        self.persist_tool_defaults();
        self.touch(cx);
    }
    fn on_snap_bounds(&mut self, _: &keys::SnapBounds, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.snap_to_document_bounds = !self.tab().session.snap_to_document_bounds;
        self.persist_tool_defaults();
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
    // Cmd-arrow: the selected pixels move, not the outline or the layer
    // (SelectionEdits.swift:141).
    fn on_nudge_pixels_left(&mut self, _: &keys::NudgePixelsLeft, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge_pixels(-1.0, 0.0, w, cx);
    }
    fn on_nudge_pixels_right(&mut self, _: &keys::NudgePixelsRight, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge_pixels(1.0, 0.0, w, cx);
    }
    fn on_nudge_pixels_up(&mut self, _: &keys::NudgePixelsUp, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge_pixels(0.0, -1.0, w, cx);
    }
    fn on_nudge_pixels_down(&mut self, _: &keys::NudgePixelsDown, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge_pixels(0.0, 1.0, w, cx);
    }
    fn on_nudge_pixels_left_more(&mut self, _: &keys::NudgePixelsLeftMore, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge_pixels(-10.0, 0.0, w, cx);
    }
    fn on_nudge_pixels_right_more(&mut self, _: &keys::NudgePixelsRightMore, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge_pixels(10.0, 0.0, w, cx);
    }
    fn on_nudge_pixels_up_more(&mut self, _: &keys::NudgePixelsUpMore, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge_pixels(0.0, -10.0, w, cx);
    }
    fn on_nudge_pixels_down_more(&mut self, _: &keys::NudgePixelsDownMore, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge_pixels(0.0, 10.0, w, cx);
    }
    fn nudge_pixels(&mut self, dx: f64, dy: f64, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.tab().session.nudge_pixels(dx, dy);
            t.touch(cx);
        });
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
        self.tab().session.cycle_blend_mode(false);
        cx.notify();
    }
    fn on_blend_mode_next(&mut self, _: &keys::BlendModeNext, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.cycle_blend_mode(true);
        cx.notify();
    }
    fn on_cycle_shape_kind(&mut self, _: &keys::CycleShapeKind, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.toggle_shape_kind();
        cx.notify();
    }
    fn on_show_transform_controls(&mut self, _: &keys::ShowTransformControls, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.shows_transform_controls = !self.tab().session.shows_transform_controls;
        self.persist_tool_defaults();
        self.touch(cx);
    }
    fn on_invert_pixels(&mut self, _: &keys::InvertPixels, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.invert_pixels();
        self.touch(cx);
    }
    fn on_toggle_clipping_mask(&mut self, _: &keys::ToggleClippingMask, _w: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.tab().session.active_layer_id {
            self.tab().session.toggle_clipping_mask(id);
        }
        self.touch(cx);
    }
    fn on_move_layer_down(&mut self, _: &keys::MoveLayerDown, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.move_active_layer(-1);
        self.touch(cx);
    }
    fn on_move_layer_up(&mut self, _: &keys::MoveLayerUp, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.move_active_layer(1);
        self.touch(cx);
    }
    fn on_cut(&mut self, _: &keys::Cut, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.tab().session.cut_selection();
            t.touch(cx);
        });
    }
    fn on_copy_pixels(&mut self, _: &keys::CopyPixels, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.tab().session.copy_selection();
            t.touch(cx);
        });
    }
    fn on_copy_merged(&mut self, _: &keys::CopyMerged, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.tab().session.copy_merged_selection();
            t.touch(cx);
        });
    }
    fn on_paste_pixels(&mut self, _: &keys::PastePixels, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.tab().session.paste_pixels();
            t.touch(cx);
        });
    }
    fn on_content_aware_fill(&mut self, _: &keys::ContentAwareFill, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.run_content_aware_fill(cx));
    }
    fn on_clear_selection_pixels(&mut self, _: &keys::ClearSelectionPixels, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.tab().session.clear_selected_pixels();
            t.touch(cx);
        });
    }
    fn on_load_layer_selection(&mut self, _: &keys::LoadLayerSelection, _w: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.tab().session.active_layer_id {
            self.tab().session
                .load_layer_selection(id, compositor_core::selection::SelectionMode::Replace);
            self.touch(cx);
        }
    }
    fn on_load_mask_selection(&mut self, _: &keys::LoadMaskSelection, _w: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.tab().session.active_layer_id {
            self.tab().session
                .load_mask_selection(id, compositor_core::selection::SelectionMode::Replace);
            self.touch(cx);
        }
    }
    fn on_remove_background(&mut self, _: &keys::RemoveBackground, _w: &mut Window, cx: &mut Context<Self>) {
        // Runs on the layer directly: no dialog, one undo (FilterKind menu).
        let Some(base) = self
            .tab().session
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
        self.history_edit("Remove Background", move |session| {
            if let Some(id) = session.active_layer_id {
                if let Some(doc) = session.document.as_mut() {
                    if let Some(layer) = doc.layer_mut(id) {
                        layer.asset = Some(compositor_core::raster::ImportedImage::new(result, layer.name.clone()));
                    }
                }
            }
        });
        self.touch(cx);
    }
    fn on_clear_guides(&mut self, _: &keys::ClearGuides, _w: &mut Window, cx: &mut Context<Self>) {
        self.tab().session.clear_guides();
        self.touch(cx);
    }
    fn on_toggle_levels_preview(&mut self, _: &keys::ToggleLevelsPreview, _w: &mut Window, cx: &mut Context<Self>) {
        self.toggle_levels_preview(cx);
    }
    fn on_finish_lasso(&mut self, _: &keys::FinishLasso, _w: &mut Window, cx: &mut Context<Self>) {
        if self.tab().session.lasso_draft.is_some() {
            self.tab().session.finish_lasso();
            self.touch(cx);
        } else if self.tab().session.transform_edit.is_some() {
            // Return applies the pending transform (the inspector's Apply).
            self.tab().session.commit_transform();
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
            if t.tab().session.transform_edit.is_some()
                || (t.tab().session.tool == NavigationTool::Move && t.tab().session.can_transform())
            {
                t.tab().session.nudge_layer(dx * scale, dy * scale);
            } else {
                t.tab().session.nudge_selection(dx * scale, dy * scale);
            }
            t.touch(cx);
        });
    }

    fn render_status_bar(&mut self, _cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let (width, height) = self.doc_size();
        let tool = self.tab().session.tool.label().to_string();
        let zoom = format!("{:.0}%", self.zoom * 100.0);
        let undo = self.tab().session.history.undo_name().to_string();
        let redo = self.tab().session.history.redo_name().to_string();
        let undo_text = if self.tab().session.can_undo() {
            format!("Undo: {}", undo)
        } else {
            "Undo: \u{2014}".to_string()
        };
        let redo_text = if self.tab().session.can_redo() {
            format!("Redo: {}", redo)
        } else {
            "Redo: \u{2014}".to_string()
        };
        let import_error = self.workspace.current_mut().import_error.take();
        let message = import_error.or_else(|| self.status.clone()).or_else(|| {
            self.current_tab().session.brush_error.clone()
        }).unwrap_or_default();

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
