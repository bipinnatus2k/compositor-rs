//! Port of `ContentView.swift`: the per-tab content view entity (the tool
//! rail, the options header row, the canvas, the layers panel and the
//! status bar — the middle section of the window), plus the root view's
//! render (`ProjectWorkspaceView`, ProjectTabs.swift:6-19), the action
//! dispatch for every menu item and keyboard shortcut, the drain pump and
//! the status bar entity. The root owns the tab strip and the menu
//! assembly; the per-tab `ContentView` is rebuilt on tab switches by the
//! root's `sync_content` (the `.id(workspace.current.id)` mirror).

use gpui_kit::component::TitleBar;
use gpui_kit::{
    App, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, div, px, rgb,
};

use compositor_core::adjustment::AdjustmentKind;
use compositor_core::document::NavigationTool;
use compositor_core::filters::FilterKind;
use compositor_core::session::SelectionAmountOperation;
use compositor_core::session::{BrushToolMode, EditorSession};

use crate::canvas::editor_canvas::EditorCanvasView;
use crate::keys;
use crate::project::{Pending, Step};
use crate::state::{ProjectWorkspaceView, SaveAfter, Sheet};
use crate::ui::brush_controls::BrushControlsView;
use crate::ui::crop_controls::CropControlsView;
use crate::ui::lasso_controls::LassoControlsView;
use crate::ui::native_layer_list::LayersPanelView;
use crate::ui::navigation_tool_header::{
    GradientControlsView, NavigationToolHeaderView, Swatch, TOOLS, ToolButton, tool_title,
};
use crate::ui::shape_controls::ShapeControlsView;
use crate::ui::transform_inspector::TransformInspectorView;
use crate::ui::type_controls::TypeControlsView;
use crate::ui::{FG_DIM, app_on_click, text_label};
use crate::welcome::WelcomeView;

impl Render for ProjectWorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Workspace errors (failed opens → the Alert sheet) and queued file
        // receives progress on the async drain pump, never during draw;
        // rendering stays a pure function of state and only requests a pump
        // when state says work is pending.
        self.schedule_drain_pump(window, cx);
        // The `.id(workspace.current.id)` mirror: rebuild the per-tab
        // content when the workspace's selected tab changed (runs here,
        // where the window is available for the constructors' inputs).
        self.sync_content(window, cx);
        // The app menus ride the same render head: re-registered only when
        // the active session's menu-affecting state drifted.
        self.refresh_menus(cx);
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
            .on_action(cx.listener(Self::on_delete_key_pressed))
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
            // Layout. The title bar is the gpui-kit `TitleBar`: it owns the
            // window drag region and the Windows min/max/close controls that
            // the transparent system title bar has no surface for (the
            // macOS original uses the native chrome; the in-window bar is
            // the port's recorded deviation). The gpui-kit `AppMenuBar`
            // rides in it — the component's title-bar-with-menu-bar
            // arrangement; the bar's chrome (panel surface + hairline) is
            // the title bar's, and the menus themselves are re-registered
            // from the active session in `refresh_menus`.
            .child(
                TitleBar::new()
                    .bg(crate::ui::panel_bg())
                    .border_color(gpui_kit::black().opacity(0.4))
                    .child(self.app_menu_bar.clone()),
            )
            .child(self.render_tab_bar(cx))
            // The active tab's content (Swift `ContentView`, ticket 43): a
            // fresh per-tab entity hosting the tool rail, the options
            // header row, the canvas (with the welcome branch), the layers
            // panel and the status bar. The tab strip and the menu assembly
            // stay on the root — the root re-renders on workspace
            // notifications anyway, so a separate entity would buy zero
            // invalidation (declared in the ticket's landing record).
            .children(self.content.clone())
            .children(self.render_sheet_overlay(cx))
            // The floating panels render above the workspace; the color
            // picker is one of them now (ColorPickerSheet.swift:167).
            .children(self.render_panels_overlay(window, cx))
    }
}

impl ProjectWorkspaceView {
    /// Spawns the workspace drain pump when state says work is pending. The
    /// pump — not `render` — drains workspace-level errors into the Alert
    /// sheet and runs queued file receives once the guards cleared, the two
    /// side effects the render head used to carry. Spawning is idempotent:
    /// one pump at a time (the [`Self::drain_pump_running`] flag), and a
    /// pump that finds nothing left to do exits without ever holding entity
    /// state across an await.
    fn schedule_drain_pump(&mut self, window: &Window, cx: &Context<Self>) {
        let workspace = self.workspace.read(cx);
        if workspace.errors.is_empty() && !workspace.has_queued_receives() {
            return;
        }
        if self.drain_pump_running {
            return;
        }
        self.drain_pump_running = true;
        cx.spawn_in(window, async move |this, cx| {
            loop {
                // One drain pass, with the same order and guards the render
                // head used: errors first (the Alert sheet), then queued
                // receives. A pass that changed state asks for a frame, the
                // way the old same-frame drains showed their result.
                let mut waiting_on_guards = false;
                let window_open = this.update_in(cx, |this, window, cx| {
                    let sheet_free = this.sheet.is_none();
                    let had_errors = !this.workspace.read(cx).errors.is_empty();
                    let had_receives = this.workspace.read(cx).has_queued_receives();
                    this.drain_workspace_errors(window, cx);
                    this.drain_queued_receives(cx);
                    let alert_opened = sheet_free && had_errors && this.sheet.is_some();
                    let receives_ran =
                        had_receives && !this.workspace.read(cx).has_queued_receives();
                    if alert_opened || receives_ran {
                        cx.notify();
                    }
                    // Receives still queued mean the guards block them (a
                    // floating panel or a mid-flow dialog): keep Swift's
                    // `while !canSwitch` 30 ms poll going
                    // (ProjectWorkspace.swift:126-129) until they clear.
                    // Errors left behind an open Alert wait for its
                    // dismissal — the close notifies, and the next render
                    // re-requests a pump.
                    waiting_on_guards = this.workspace.read(cx).has_queued_receives();
                });
                if window_open.is_err() || !waiting_on_guards {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(30))
                    .await;
            }
            // A window that went away drops the flag with the root entity.
            this.update(cx, |this, _| this.drain_pump_running = false)
                .ok();
        })
        .detach();
    }
}

/// The active tab's content (Swift `ContentView`, ContentView.swift:4-18):
/// a thin per-tab container built fresh by the root's `sync_content`
/// whenever the workspace's selected tab changed — the `.id(workspace
/// .current.id)` mirror. It holds the tab's session and the child view
/// entities (Swift `ContentView`'s members), each constructed with the
/// session at construction time, so every per-tab UI draft (canvas drags,
/// the rename field, the welcome form's fields) dies with the rebuild and
/// the fresh children observe the new tab's session from their
/// constructors. The window-level state — modal sheets, floating panels,
/// frame cache, preview pipelines, flows — stays on the root, reached
/// through the same `WeakEntity<ProjectWorkspaceView>` seam every child has
/// held since ticket 39.
pub struct ContentView {
    /// The tab this content was built for (`workspace.current.id` — the
    /// `.id` mirror's comparison key).
    pub(crate) tab_id: crate::project::TabId,
    /// The active tab's session (`@Bindable var session`).
    pub(crate) session: Entity<EditorSession>,
    /// The root view: the flows, pipelines and frame cache stay there
    /// (window-level); the per-tab views reach them through this seam.
    pub(crate) app: WeakEntity<ProjectWorkspaceView>,
    /// The canvas view (Swift `CanvasView`, ticket 39): pointer/keyboard
    /// gestures, the inline text editor and the drag state.
    pub(crate) canvas_view: Entity<EditorCanvasView>,
    /// The layers panel (Swift `LayersPanel`, ticket 40): rename field,
    /// thumbnail cache, header opacity slider.
    pub(crate) layers_panel: Entity<LayersPanelView>,
    /// The tool option headers (Swift's per-tool header views, ticket 42):
    /// each owns its option-bar widget entities and their write
    /// subscriptions, and its controls enter the shared header row only
    /// while its tool is active (Swift's `if session.tool == …` branches).
    pub(crate) brush_header: Entity<BrushControlsView>,
    pub(crate) lasso_header: Entity<LassoControlsView>,
    pub(crate) gradient_header: Entity<GradientControlsView>,
    pub(crate) crop_header: Entity<CropControlsView>,
    pub(crate) shape_header: Entity<ShapeControlsView>,
    pub(crate) type_header: Entity<TypeControlsView>,
    pub(crate) transform_header: Entity<TransformInspectorView>,
    /// The Zoom/Hand header (Swift `NavigationToolHeader`; the Swift view's
    /// zoom-text drafts have no Rust counterpart — recorded in the ticket).
    pub(crate) navigation_header: Entity<NavigationToolHeaderView>,
    /// The empty tab's welcome form (Swift `ContentView.welcome`, the
    /// tab-embedded `NewCanvasSheet`): owns the Width/Height fields and
    /// renders the whole form; the canvas view's welcome branch clones it.
    pub(crate) welcome: Entity<WelcomeView>,
    /// The status bar (Swift `ContentView.statusBar`): owns the status line
    /// and observes the session for its readouts.
    pub(crate) status_bar: Entity<StatusBarView>,
}

impl ContentView {
    /// Builds the per-tab content: the children are constructed in the same
    /// order the root's former constructor used (ticket 42), each taking
    /// the session and the root seam it needs — an `InputState` needs the
    /// window, so the constructor runs where one is available.
    pub(crate) fn new(
        tab_id: crate::project::TabId,
        session: Entity<EditorSession>,
        app: WeakEntity<ProjectWorkspaceView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The tool headers (Swift's per-tool header views, ticket 42): each
        // builds its own widgets — an `InputState` needs the window.
        let brush_header = cx.new(|cx| BrushControlsView::new(session.clone(), app.clone(), cx));
        let lasso_header = cx.new(|cx| LassoControlsView::new(session.clone(), app.clone(), cx));
        let gradient_header =
            cx.new(|cx| GradientControlsView::new(session.clone(), app.clone(), cx));
        let crop_header = cx.new(|_| CropControlsView::new(session.clone(), app.clone()));
        let shape_header = cx.new(|_| ShapeControlsView::new(session.clone(), app.clone()));
        let type_header =
            cx.new(|cx| TypeControlsView::new(session.clone(), app.clone(), window, cx));
        let transform_header =
            cx.new(|cx| TransformInspectorView::new(session.clone(), app.clone(), window, cx));
        let navigation_header =
            cx.new(|_| NavigationToolHeaderView::new(session.clone(), app.clone()));
        let welcome = cx.new(|cx| WelcomeView::new(session.clone(), app.clone(), window, cx));
        // The status bar observes the session from its constructor — fresh
        // per tab, so no re-pointing.
        let status_bar = cx.new(|cx| StatusBarView::new(session.clone(), app.clone(), cx));

        // The canvas view (Swift `CanvasView`): it builds its own inline
        // text input (an `InputState` needs the window), and it holds the
        // root's handle so its handlers can reach the pipelines that stay
        // there (ticket 39's seam).
        let canvas_view =
            cx.new(|cx| EditorCanvasView::new(session.clone(), app.clone(), window, cx));
        // The layers panel (Swift `LayersPanel`): it builds its own rename
        // input and header opacity slider, and reaches the root's flows
        // through the same seam (ticket 40).
        let layers_panel = cx.new(|cx| LayersPanelView::new(session.clone(), app.clone(), cx));

        ContentView {
            tab_id,
            session: session.clone(),
            app,
            canvas_view,
            layers_panel,
            brush_header,
            lasso_header,
            gradient_header,
            crop_header,
            shape_header,
            type_header,
            transform_header,
            navigation_header,
            welcome,
            status_bar,
        }
    }

    // MARK: Tool rail

    /// The tool rail (Swift `ContentView`'s `toolRail` computed property):
    /// one entry per tool plus the foreground/background swatches. The
    /// buttons dispatch the root's tool flows through the seam.
    pub(crate) fn render_tool_rail(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let app = self.app.clone();
        let (active, eraser_active) = {
            let session = self.session.read(cx);
            (
                session.tool,
                session.tool == NavigationTool::Brush && session.brush_mode == BrushToolMode::Erase,
            )
        };

        let mut rail = gpui_kit::div()
            .flex()
            .flex_col()
            .items_center()
            .w(px(44.0))
            .flex_shrink_0()
            .py_1()
            .bg(crate::ui::panel_bg());

        for (tool, letter, icon, name, eraser_entry) in TOOLS {
            let selected = if eraser_entry {
                eraser_active
            } else if tool == NavigationTool::Brush {
                active == NavigationTool::Brush && !eraser_active
            } else {
                active == tool
            };
            rail = rail.child(
                ToolButton::new(tool, letter, icon, name, eraser_entry, selected).with_on_click(
                    app_on_click(&app, move |state, _, _, cx| {
                        if eraser_entry {
                            state.set_tool_eraser(cx);
                        } else {
                            state.set_tool(tool, cx);
                        }
                    }),
                ),
            );
        }

        // Foreground / background swatches: clicking either opens the color
        // picker on it (ColorPalette.swift:60; the X key still swaps).
        let foreground = self.session.read(cx).palette_color(false);
        let background = self.session.read(cx).palette_color(true);
        rail = rail.child(
            gpui_kit::div()
                .mt_2()
                .id("palette")
                .w(px(28.0))
                .h(px(28.0))
                .relative()
                .child(
                    Swatch::new("swatch-background", 6.0, 6.0, background).with_on_click(
                        app_on_click(&app, |state, _, window, cx| {
                            state.open_color_picker_ui(true, window, cx);
                        }),
                    ),
                )
                .child(
                    Swatch::new("swatch-foreground", 0.0, 0.0, foreground).with_on_click(
                        app_on_click(&app, |state, _, window, cx| {
                            state.open_color_picker_ui(false, window, cx);
                        }),
                    ),
                ),
        );

        rail.into_any_element()
    }

    // MARK: Options header

    /// The options header row (Swift `toolHeaders`' shared bar): the title
    /// plus the active tool's controls. Each per-tool control set is a view
    /// entity (ticket 42); only the active tool's entity is consulted per
    /// pass, mirroring Swift's `if session.tool == ...` branches — the
    /// entities are this content's fields, built fresh per tab.
    pub(crate) fn render_options_header(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let (tool, erasing) = {
            let session = self.session.read(cx);
            (session.tool, session.brush_mode == BrushToolMode::Erase)
        };

        let mut header = gpui_kit::div()
            // Stateful so the bar can scroll sideways (overflow_x_scroll is a
            // StatefulInteractiveElement method, gpui div.rs).
            .id("options-header")
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            // ToolHeaderStyle.swift:5 (height 42) and BrushControls.swift:111
            // (horizontal padding 18).
            .h(px(42.0))
            .px(px(18.0))
            .gap_3()
            .flex_shrink_0()
            // Dense bars (Type, Move) run out of width; scroll sideways
            // instead of clipping the tail controls.
            .overflow_x_scroll()
            .bg(crate::ui::panel_bg());

        header = header.child(
            text_label(tool_title(tool, erasing), false)
                .w(px(82.0))
                // ToolHeaderStyle.swift:6: the title is 13pt semibold.
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .flex_shrink_0(),
        );

        match tool {
            NavigationTool::Brush
            | NavigationTool::SpotHealing
            | NavigationTool::Blur
            | NavigationTool::CloneStamp => {
                header = header.children(
                    self.brush_header
                        .update(cx, BrushControlsView::brush_controls),
                );
            }
            NavigationTool::Marquee => {
                header = header.children(
                    self.lasso_header
                        .update(cx, LassoControlsView::marquee_controls),
                );
            }
            NavigationTool::Lasso => {
                header = header.children(
                    self.lasso_header
                        .update(cx, LassoControlsView::lasso_controls),
                );
            }
            NavigationTool::Wand => {
                header = header.children(
                    self.lasso_header
                        .update(cx, LassoControlsView::wand_controls),
                );
            }
            NavigationTool::Gradient => {
                // Swift home: GradientControls.swift; the entity lives in
                // `ui::navigation_tool_header`, where the arm was carved (no
                // file of its own in the port).
                header = header.children(
                    self.gradient_header
                        .update(cx, GradientControlsView::gradient_controls),
                );
            }
            NavigationTool::Zoom | NavigationTool::Hand => {
                header = header.children(
                    self.navigation_header
                        .update(cx, NavigationToolHeaderView::controls),
                );
            }
            NavigationTool::Crop => {
                header =
                    header.children(self.crop_header.update(cx, CropControlsView::crop_controls));
            }
            _ => {}
        }

        if tool == NavigationTool::Move {
            header = header.children(
                self.transform_header
                    .update(cx, TransformInspectorView::transform_inspector),
            );
        }
        if tool == NavigationTool::Type {
            header = header.children(self.type_header.update(cx, TypeControlsView::type_controls));
        }
        if tool == NavigationTool::Shape {
            header = header.children(
                self.shape_header
                    .update(cx, ShapeControlsView::shape_controls),
            );
        }

        header.into_any_element()
    }
}

impl Render for ContentView {
    /// The per-tab content (Swift `ContentView`'s `editorStack`): the tool
    /// rail and canvas column row plus the status bar, filling the window
    /// below the tab strip. The geometry is the former root layout verbatim
    /// (the root hosts this entity where the middle row used to sit).
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
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
                            // The canvas is its own view entity (Swift
                            // `CanvasView`, ticket 39): it renders itself,
                            // including the welcome form on an empty tab.
                            .child(self.canvas_view.clone()),
                    )
                    // The layers panel is its own view entity too (Swift
                    // `LayersPanel`, ticket 40): header, layer list and
                    // footer render there.
                    .child(self.layers_panel.clone()),
            )
            // The status bar is its own view entity too (ticket 42): it owns
            // the status line and observes the session for its readouts.
            .child(self.status_bar.clone())
    }
}

/// The layer's adjustment settings as the filter-sheet editor reads them:
/// every slot present on the adjustment overrides the default.
fn filter_settings_from_adjustment(
    adjustment: &compositor_core::adjustment::LayerAdjustment,
) -> compositor_core::filters::FilterSettings {
    let mut settings = compositor_core::filters::FilterSettings::default();
    if let Some(value) = &adjustment.exposure_settings {
        settings.exposure = *value;
    }
    if let Some(value) = &adjustment.gradient_map_settings {
        settings.gradient_map = *value;
    }
    if let Some(value) = &adjustment.grain_settings {
        settings.grain = *value;
    }
    if let Some(value) = &adjustment.black_white_settings {
        settings.black_white = *value;
    }
    if let Some(value) = &adjustment.color_balance_settings {
        settings.color_balance = *value;
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

impl ProjectWorkspaceView {
    // MARK: Shortcut registry

    /// The live shortcut registry (KeyboardShortcuts.swift:129-213). It is
    /// loaded from `%APPDATA%/compositor/shortcuts.json` — or the platform
    /// equivalent — when the app starts (`keys::bindings()` reads it before
    /// the window opens, so custom chords and menu hints are in effect from
    /// the first frame). The shortcut settings sheet reads this for its
    /// draft, assigns overrides with `set_override`, and persists with
    /// `save`; a new launch (or a rebind of `keys::bindings_with`) applies
    /// the change.
    pub fn shortcuts(
        &self,
    ) -> std::sync::MutexGuard<'static, crate::ui::keyboard_shortcuts::ShortcutRegistry> {
        crate::ui::keyboard_shortcuts::lock_global()
    }

    /// Saves the persisted tool toggles (`settings.rs`): Swift's
    /// `EditorSession` properties write each change straight through
    /// `didSet { ToolDefaults.set(...) }`; the port snapshots the whole table
    /// at the change point and stores it atomically. Every handler below that
    /// flips one of the persisted switches calls this right after mutating
    /// the active tab's session, which is also why no
    /// explicit exit save is wired: the table on disk is always current.
    pub fn persist_tool_defaults(&mut self, cx: &App) {
        let session = self.current_tab(cx).session.read(cx);
        crate::settings::persist_session(session);
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
        self.hotkey(w, cx, |t, _w, cx| {
            t.set_tool(NavigationTool::SpotHealing, cx)
        });
    }
    fn on_tool_clone(&mut self, _: &keys::ToolClone, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.set_tool(NavigationTool::CloneStamp, cx)
        });
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
    fn on_tool_eyedropper(
        &mut self,
        _: &keys::ToolEyedropper,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.set_tool(NavigationTool::Eyedropper, cx)
        });
    }
    fn on_tool_hand(&mut self, _: &keys::ToolHand, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::Hand, cx));
    }
    fn on_tool_zoom(&mut self, _: &keys::ToolZoom, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| t.set_tool(NavigationTool::Zoom, cx));
    }

    fn on_swap_colors(&mut self, _: &keys::SwapColors, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.swap_palette_colors())
            });
            t.touch(cx);
        });
    }
    fn on_default_colors(
        &mut self,
        _: &keys::DefaultColors,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.reset_palette_colors())
            });
            t.touch(cx);
        });
    }
    fn on_cycle_tool_mode(
        &mut self,
        _: &keys::CycleToolMode,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hotkey(w, cx, |t, _w, cx| t.cycle_tool_mode(cx));
    }

    // MARK: File

    fn on_new_canvas(&mut self, _: &keys::NewCanvas, _w: &mut Window, cx: &mut Context<Self>) {
        // `newCanvas` (:53-57): a fresh tab, no modal.
        if self.workspace.update(cx, |ws, cx| ws.new_canvas(cx)) {
            self.touch(cx);
        }
    }
    fn on_open(&mut self, _: &keys::OpenDocument, w: &mut Window, cx: &mut Context<Self>) {
        // `open(_:)` (:58-76), two-phase: the dialog ask comes back as
        // `Pending::Open` and the Open sheet plays the file picker's part
        // (semicolon-separated paths stand in for the multi-select panel).
        match self.workspace.update(cx, |ws, cx| ws.open(None, cx)) {
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
        let result =
            self.with_current_tab(cx, |controller, session| controller.save(session, as_new));
        match result {
            Step::Pending(Pending::SaveAs { default_name, .. }) => {
                self.pending_save =
                    Some((self.workspace.read(cx).selected_id(), SaveAfter::Nothing));
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
        match self.with_current_tab(cx, |controller, session| controller.export_png(session)) {
            Step::Pending(pending) => self.present_pending(pending, w, cx),
            Step::Done(()) => {}
        }
    }
    fn on_export_jpeg(&mut self, _: &keys::ExportJpeg, w: &mut Window, cx: &mut Context<Self>) {
        match self.with_current_tab(cx, |controller, session| controller.export_jpeg(session)) {
            Step::Pending(pending) => self.present_pending(pending, w, cx),
            Step::Done(()) => {}
        }
    }
    fn on_import_images(
        &mut self,
        _: &keys::ImportImages,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_import_images(cx);
    }
    fn on_close_document(
        &mut self,
        _: &keys::CloseDocument,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // `close(_:)` (:89-96): confirm unsaved changes, then remove the tab.
        match self
            .workspace
            .update(cx, |ws, cx| ws.close(ws.selected_id(), cx))
        {
            Step::Pending(pending) => self.present_pending(pending, w, cx),
            Step::Done(()) => {
                self.touch(cx);
            }
        }
    }
    fn on_quit(&mut self, _: &keys::Quit, w: &mut Window, cx: &mut Context<Self>) {
        // `confirmQuit` (:106-114): ask each tab in order; on success the
        // window closes.
        match self.workspace.update(cx, |ws, cx| ws.confirm_quit(cx)) {
            Step::Pending(pending) => self.present_pending(pending, w, cx),
            Step::Done(true) => w.remove_window(),
            // A refusal changes nothing on screen (the guard never latched),
            // and the active tab's content is rebuilt from the workspace's
            // selection on the next render anyway.
            Step::Done(false) => {}
        }
    }

    // MARK: Edit

    fn on_undo(&mut self, _: &keys::Undo, _w: &mut Window, cx: &mut Context<Self>) {
        // A restored document of a different canvas refits the viewport
        // inside the session (`restore` → `changedCanvas`, ticket 38).
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.undo())
        });
        self.touch(cx);
    }
    fn on_redo(&mut self, _: &keys::Redo, _w: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.redo())
        });
        self.touch(cx);
    }
    fn on_fill_foreground(
        &mut self,
        _: &keys::FillForeground,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let color = color_bytes(
            self.workspace
                .read(cx)
                .current()
                .session
                .read(cx)
                .palette_color(false),
        );
        self.fill_active(color, "Fill Foreground", cx);
    }
    fn on_fill_background(
        &mut self,
        _: &keys::FillBackground,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let color = color_bytes(
            self.workspace
                .read(cx)
                .current()
                .session
                .read(cx)
                .palette_color(true),
        );
        self.fill_active(color, "Fill Background", cx);
    }
    fn on_free_transform(
        &mut self,
        _: &keys::FreeTransform,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // transformCommand: the selection floats when there is one (FloatingSelection.swift:22).
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .can_transform_selection()
        {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.begin_selection_transform())
            });
        } else {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.begin_transform())
            });
        }
        self.touch(cx);
    }
    fn on_shortcuts(&mut self, _: &keys::ShowShortcuts, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_shortcuts_panel(cx);
    }

    // MARK: Image

    fn on_image_size(&mut self, _: &keys::ImageSize, w: &mut Window, cx: &mut Context<Self>) {
        // `imageSize()` (:76-87): guard + begin, then the Image Size sheet.
        match self.with_current_tab(cx, |controller, session| controller.image_size(session)) {
            Step::Pending(pending) => self.present_pending(pending, w, cx),
            Step::Done(()) => {}
        }
    }
    fn on_canvas_size(&mut self, _: &keys::CanvasSize, w: &mut Window, cx: &mut Context<Self>) {
        match self.with_current_tab(cx, |controller, session| controller.canvas_size(session)) {
            Step::Pending(pending) => self.present_pending(pending, w, cx),
            Step::Done(()) => {}
        }
    }
    fn on_trim(&mut self, _: &keys::TrimSheet, w: &mut Window, cx: &mut Context<Self>) {
        match self.with_current_tab(cx, |controller, session| controller.trim(session)) {
            Step::Pending(pending) => self.present_pending(pending, w, cx),
            Step::Done(()) => {}
        }
    }
    fn on_crop_to_selection(
        &mut self,
        _: &keys::CropToSelection,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(selection) = self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .selection()
            && let Some(bounds) = selection.path.bounds()
        {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.crop_rect = Some(bounds))
            });
            self.set_status("Crop to selection: press Apply in the Crop header", cx);
        }
    }
    fn on_flip_canvas_h(
        &mut self,
        _: &keys::FlipCanvasHorizontal,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.flip_canvas(true))
        });
        self.touch(cx);
    }
    fn on_flip_canvas_v(
        &mut self,
        _: &keys::FlipCanvasVertical,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.flip_canvas(false))
        });
        self.touch(cx);
    }

    fn on_levels(&mut self, _: &keys::LevelsSheet, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_levels_panel(cx);
    }
    fn on_hue_saturation(
        &mut self,
        _: &keys::HueSaturationSheet,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_hue_saturation_panel(cx);
    }
    fn on_curves(&mut self, _: &keys::CurvesSheet, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_filter_panel(FilterKind::Curves, cx);
    }
    fn on_filter_black_white(
        &mut self,
        _: &keys::FilterBlackWhite,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_filter_panel(FilterKind::BlackWhite, cx);
    }
    fn on_filter_color_balance(
        &mut self,
        _: &keys::FilterColorBalance,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_filter_panel(FilterKind::ColorBalance, cx);
    }
    fn on_filter_exposure(
        &mut self,
        _: &keys::FilterExposure,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_filter_panel(FilterKind::Exposure, cx);
    }
    fn on_filter_gradient_map(
        &mut self,
        _: &keys::FilterGradientMap,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_filter_panel(FilterKind::GradientMap, cx);
    }
    fn on_filter_grain(&mut self, _: &keys::FilterGrain, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_filter_panel(FilterKind::Grain, cx);
    }

    // MARK: Layer

    fn on_new_layer(&mut self, _: &keys::NewLayer, _w: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.add_blank_layer())
        });
        self.touch(cx);
    }
    fn on_duplicate_layer(
        &mut self,
        _: &keys::DuplicateLayer,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // layerViaCopy duplicates when nothing is selected (SelectionClipboard.swift:138).
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.layer_via_copy())
        });
        self.touch(cx);
    }
    fn on_delete_layer(&mut self, _: &keys::DeleteLayer, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, w, cx| {
            // deleteLayerOrMask: the mask target deletes the mask; a live
            // mask dependency raises the bake/remove-links ask (Swift's
            // finishDeletingLayers dialog).
            t.request_delete_layer(w, cx);
        });
    }
    fn on_delete_key_pressed(
        &mut self,
        _: &keys::DeleteKeyPressed,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hotkey(w, cx, |t, w, cx| {
            // The Delete key (SelectionEdits.swift:63-67): the selected
            // effect goes first, then a selection clears its pixels; the
            // fallthrough is deleteLayerOrMask — the live-mask ask entry.
            let consumed = t.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.delete_key_pressed())
            });
            if !consumed {
                t.request_delete_layer(w, cx);
            }
        });
    }
    fn on_merge_down(&mut self, _: &keys::MergeDown, _w: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.merge_layers())
        });
        self.touch(cx);
    }
    fn on_new_group(&mut self, _: &keys::NewGroup, _w: &mut Window, cx: &mut Context<Self>) {
        self.add_group(cx);
    }

    /// New Group: a folder inserted above the active layer, which becomes
    /// its first child.
    pub(crate) fn add_group(&mut self, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.add_group())
        });
        self.touch(cx);
    }
    fn on_add_mask(&mut self, _: &keys::AddMask, _w: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.add_layer_mask(true))
        });
        self.touch(cx);
    }
    fn on_flip_layer_h(
        &mut self,
        _: &keys::FlipLayerHorizontal,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.flip_layers(true))
        });
        self.touch(cx);
    }
    fn on_flip_layer_v(
        &mut self,
        _: &keys::FlipLayerVertical,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.flip_layers(false))
        });
        self.touch(cx);
    }
    fn on_group_selected_layers(
        &mut self,
        _: &keys::GroupSelectedLayers,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.group_selected_layers())
        });
        self.touch(cx);
    }
    fn on_move_out_of_folder(
        &mut self,
        _: &keys::MoveOutOfFolder,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.move_active_layer_out_of_group())
        });
        self.touch(cx);
    }
    fn on_rename_layer(&mut self, _: &keys::RenameLayer, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .active_layer_id
        {
            // The rename field is the layers panel's (ticket 40; the panel
            // is the active tab content's child, ticket 43).
            if let Some(content) = self.content.as_ref() {
                let panel = content.read(cx).layers_panel.clone();
                panel.update(cx, |panel, cx| panel.begin_rename(id, w, cx));
            }
        }
    }
    fn on_toggle_layer_visibility(
        &mut self,
        _: &keys::ToggleLayerVisibility,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .active_layer_id
        {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.toggle_layer_visibility(id))
            });
            self.touch(cx);
        }
    }
    pub(crate) fn add_adjustment(&mut self, kind: AdjustmentKind, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.add_adjustment(kind))
        });
        self.touch(cx);
    }
    fn on_add_adjustment_hsv(
        &mut self,
        _: &keys::AddAdjustmentHsv,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_adjustment(AdjustmentKind::Hsv, cx);
    }
    fn on_add_adjustment_levels(
        &mut self,
        _: &keys::AddAdjustmentLevels,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_adjustment(AdjustmentKind::Levels, cx);
    }
    fn on_add_adjustment_curves(
        &mut self,
        _: &keys::AddAdjustmentCurves,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_adjustment(AdjustmentKind::Curves, cx);
    }
    fn on_add_adjustment_exposure(
        &mut self,
        _: &keys::AddAdjustmentExposure,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_adjustment(AdjustmentKind::Exposure, cx);
    }
    fn on_add_adjustment_gradient_map(
        &mut self,
        _: &keys::AddAdjustmentGradientMap,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_adjustment(AdjustmentKind::GradientMap, cx);
    }
    fn on_add_adjustment_grain(
        &mut self,
        _: &keys::AddAdjustmentGrain,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_adjustment(AdjustmentKind::Grain, cx);
    }
    fn on_add_adjustment_add_noise(
        &mut self,
        _: &keys::AddAdjustmentAddNoise,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_adjustment(AdjustmentKind::AddNoise, cx);
    }
    fn on_add_adjustment_gaussian_blur(
        &mut self,
        _: &keys::AddAdjustmentGaussianBlur,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_adjustment(AdjustmentKind::GaussianBlur, cx);
    }
    fn on_add_adjustment_motion_blur(
        &mut self,
        _: &keys::AddAdjustmentMotionBlur,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_adjustment(AdjustmentKind::MotionBlur, cx);
    }
    fn on_add_adjustment_invert(
        &mut self,
        _: &keys::AddAdjustmentInvert,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_adjustment(AdjustmentKind::Invert, cx);
    }
    fn on_add_adjustment_black_white(
        &mut self,
        _: &keys::AddAdjustmentBlackWhite,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_adjustment(AdjustmentKind::BlackWhite, cx);
    }
    fn on_add_adjustment_color_balance(
        &mut self,
        _: &keys::AddAdjustmentColorBalance,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_adjustment(AdjustmentKind::ColorBalance, cx);
    }
    fn on_edit_adjustment(
        &mut self,
        _: &keys::EditAdjustment,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .active_layer_id
        else {
            return;
        };
        self.edit_layer_adjustment(id, cx);
    }

    /// Edit Adjustment… for the layer `id`: refuses Invert (no settings to
    /// edit) and busy states, prefills the editor from the layer's current
    /// settings and opens the matching panel. False when nothing opened.
    pub(crate) fn edit_layer_adjustment(&mut self, id: uuid::Uuid, cx: &mut Context<Self>) -> bool {
        let Some(adjustment) = self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .document
            .as_ref()
            .and_then(|document| document.layer(id))
            .and_then(|layer| layer.adjustment.clone())
        else {
            return false;
        };
        if adjustment.kind == AdjustmentKind::Invert
            || !self.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.begin_adjustment_editing(id))
            })
        {
            return false;
        }
        // Prefill the editor from the layer's current settings.
        match adjustment.kind {
            AdjustmentKind::Levels => {
                self.workspace.update(cx, |ws, cx| {
                    ws.current_mut()
                        .session
                        .update(cx, |session, _| session.levels = Some(adjustment.levels))
                });
                self.open_levels_panel(cx);
            }
            AdjustmentKind::Hsv => {
                self.workspace.update(cx, |ws, cx| {
                    ws.current_mut().session.update(cx, |session, _| {
                        session.hue_saturation =
                            Some(adjustment.hsv_settings.clone().unwrap_or_else(|| {
                                compositor_core::adjust::HueSaturationSettings::new(
                                    adjustment.hue,
                                    adjustment.saturation,
                                    adjustment.lightness,
                                    adjustment.colorize,
                                )
                            }))
                    })
                });
                self.open_hue_saturation_panel(cx);
            }
            other => {
                if let Some(filter) = other.filter_kind() {
                    self.workspace.update(cx, |ws, cx| {
                        ws.current_mut().session.update(cx, |session, _| {
                            session.filter_edit = Some(filter_settings_from_adjustment(&adjustment))
                        })
                    });
                    self.open_filter_panel(filter, cx);
                }
            }
        }
        true
    }

    // MARK: Select

    fn on_select_all(
        &mut self,
        _: &keys::SelectAllAction,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.select_all())
        });
        self.touch(cx);
    }
    fn on_deselect(&mut self, _: &keys::DeselectAction, _w: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.deselect())
        });
        self.touch(cx);
    }
    fn on_invert_selection(
        &mut self,
        _: &keys::InvertSelection,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.invert_selection())
        });
        self.touch(cx);
    }
    fn on_feather(&mut self, _: &keys::FeatherSheet, w: &mut Window, cx: &mut Context<Self>) {
        self.open_amount_panel(SelectionAmountOperation::Feather, w, cx);
    }
    fn on_expand(&mut self, _: &keys::ExpandSheet, w: &mut Window, cx: &mut Context<Self>) {
        self.open_amount_panel(SelectionAmountOperation::Expand, w, cx);
    }
    fn on_contract(&mut self, _: &keys::ContractSheet, w: &mut Window, cx: &mut Context<Self>) {
        self.open_amount_panel(SelectionAmountOperation::Contract, w, cx);
    }

    // MARK: Filter

    fn on_filter_blur(
        &mut self,
        _: &keys::FilterGaussianBlur,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_filter_panel(FilterKind::GaussianBlur, cx);
    }
    fn on_filter_motion(
        &mut self,
        _: &keys::FilterMotionBlur,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_filter_panel(FilterKind::MotionBlur, cx);
    }
    fn on_filter_noise(
        &mut self,
        _: &keys::FilterAddNoise,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_filter_panel(FilterKind::AddNoise, cx);
    }
    fn on_filter_lens(
        &mut self,
        _: &keys::FilterLensCorrection,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_filter_panel(FilterKind::LensCorrection, cx);
    }
    fn on_filter_camera_raw(
        &mut self,
        _: &keys::FilterCameraRaw,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_filter_panel(FilterKind::CameraRaw, cx);
    }

    // MARK: View

    fn on_toggle_rulers(
        &mut self,
        _: &keys::ToggleRulers,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut().session.update(cx, |session, _| {
                session.shows_rulers = !session.shows_rulers
            })
        });
        self.persist_tool_defaults(cx);
        self.touch(cx);
    }
    fn on_toggle_grid(&mut self, _: &keys::ToggleGrid, _w: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.shows_grid = !session.shows_grid)
        });
        self.persist_tool_defaults(cx);
        self.touch(cx);
    }
    fn on_toggle_guides(
        &mut self,
        _: &keys::ToggleGuides,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut().session.update(cx, |session, _| {
                session.shows_guides = !session.shows_guides
            })
        });
        self.persist_tool_defaults(cx);
        self.touch(cx);
    }
    fn on_toggle_pixel_grid(
        &mut self,
        _: &keys::TogglePixelGrid,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut().session.update(cx, |session, _| {
                session.shows_pixel_grid = !session.shows_pixel_grid
            })
        });
        self.persist_tool_defaults(cx);
        self.touch(cx);
    }
    fn on_snap_guides(&mut self, _: &keys::SnapGuides, _w: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut().session.update(cx, |session, _| {
                session.snap_to_guides = !session.snap_to_guides
            })
        });
        self.persist_tool_defaults(cx);
        self.touch(cx);
    }
    fn on_snap_toggle(&mut self, _: &keys::SnapToggle, _w: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut().session.update(cx, |session, _| {
                session.snapping_enabled = !session.snapping_enabled
            })
        });
        self.persist_tool_defaults(cx);
        self.touch(cx);
    }
    fn on_lock_guides(&mut self, _: &keys::LockGuides, _w: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut().session.update(cx, |session, _| {
                session.locks_guides = !session.locks_guides
            })
        });
        self.persist_tool_defaults(cx);
        self.touch(cx);
    }
    fn on_snap_grid(&mut self, _: &keys::SnapGrid, _w: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut().session.update(cx, |session, _| {
                session.snap_to_grid = !session.snap_to_grid
            })
        });
        self.persist_tool_defaults(cx);
        self.touch(cx);
    }
    fn on_snap_layers(&mut self, _: &keys::SnapLayers, _w: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut().session.update(cx, |session, _| {
                session.snap_to_layers = !session.snap_to_layers
            })
        });
        self.persist_tool_defaults(cx);
        self.touch(cx);
    }
    fn on_snap_bounds(&mut self, _: &keys::SnapBounds, _w: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut().session.update(cx, |session, _| {
                session.snap_to_document_bounds = !session.snap_to_document_bounds
            })
        });
        self.persist_tool_defaults(cx);
        self.touch(cx);
    }
    fn on_zoom_in(&mut self, _: &keys::ZoomIn, _w: &mut Window, cx: &mut Context<Self>) {
        // Zoom In/Out walk the stable keyboard levels (CompositorApp.swift:91-96).
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.zoom_keyboard(1))
        });
        self.touch(cx);
    }
    fn on_zoom_out(&mut self, _: &keys::ZoomOut, _w: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.zoom_keyboard(-1))
        });
        self.touch(cx);
    }
    fn on_zoom_fit(&mut self, _: &keys::ZoomFit, _w: &mut Window, cx: &mut Context<Self>) {
        self.zoom_fit(cx);
        self.touch(cx);
    }
    fn on_zoom_100(&mut self, _: &keys::Zoom100, _w: &mut Window, cx: &mut Context<Self>) {
        // Actual Pixels: `session.zoom(to: 1)` (CompositorApp.swift:88).
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.zoom(1.0, None))
        });
        self.touch(cx);
    }

    // MARK: Window / Help

    fn on_reset_panels(&mut self, _: &keys::ResetPanels, _w: &mut Window, cx: &mut Context<Self>) {
        self.set_status("Panel layout is fixed in this build", cx);
    }
    fn on_about(&mut self, _: &keys::About, _w: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(Sheet::About, _w, cx);
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
    fn on_nudge_left_more(
        &mut self,
        _: &keys::NudgeLeftMore,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge(-10.0, 0.0, true, w, cx);
    }
    fn on_nudge_right_more(
        &mut self,
        _: &keys::NudgeRightMore,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge(10.0, 0.0, true, w, cx);
    }
    fn on_nudge_up_more(&mut self, _: &keys::NudgeUpMore, w: &mut Window, cx: &mut Context<Self>) {
        self.nudge(0.0, -10.0, true, w, cx);
    }
    fn on_nudge_down_more(
        &mut self,
        _: &keys::NudgeDownMore,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge(0.0, 10.0, true, w, cx);
    }
    // Cmd-arrow: the selected pixels move, not the outline or the layer
    // (SelectionEdits.swift:141).
    fn on_nudge_pixels_left(
        &mut self,
        _: &keys::NudgePixelsLeft,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge_pixels(-1.0, 0.0, w, cx);
    }
    fn on_nudge_pixels_right(
        &mut self,
        _: &keys::NudgePixelsRight,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge_pixels(1.0, 0.0, w, cx);
    }
    fn on_nudge_pixels_up(
        &mut self,
        _: &keys::NudgePixelsUp,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge_pixels(0.0, -1.0, w, cx);
    }
    fn on_nudge_pixels_down(
        &mut self,
        _: &keys::NudgePixelsDown,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge_pixels(0.0, 1.0, w, cx);
    }
    fn on_nudge_pixels_left_more(
        &mut self,
        _: &keys::NudgePixelsLeftMore,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge_pixels(-10.0, 0.0, w, cx);
    }
    fn on_nudge_pixels_right_more(
        &mut self,
        _: &keys::NudgePixelsRightMore,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge_pixels(10.0, 0.0, w, cx);
    }
    fn on_nudge_pixels_up_more(
        &mut self,
        _: &keys::NudgePixelsUpMore,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge_pixels(0.0, -10.0, w, cx);
    }
    fn on_nudge_pixels_down_more(
        &mut self,
        _: &keys::NudgePixelsDownMore,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge_pixels(0.0, 10.0, w, cx);
    }
    fn nudge_pixels(&mut self, dx: f64, dy: f64, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.nudge_pixels(dx, dy))
            });
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
    fn on_blend_mode_previous(
        &mut self,
        _: &keys::BlendModePrevious,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.cycle_blend_mode(false))
        });
        cx.notify();
    }
    fn on_blend_mode_next(
        &mut self,
        _: &keys::BlendModeNext,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.cycle_blend_mode(true))
        });
        cx.notify();
    }
    fn on_cycle_shape_kind(
        &mut self,
        _: &keys::CycleShape,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.toggle_shape_kind())
        });
        cx.notify();
    }
    fn on_show_transform_controls(
        &mut self,
        _: &keys::ShowTransformControls,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut().session.update(cx, |session, _| {
                session.shows_transform_controls = !session.shows_transform_controls
            })
        });
        self.persist_tool_defaults(cx);
        self.touch(cx);
    }
    fn on_invert_pixels(
        &mut self,
        _: &keys::InvertPixels,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.invert_pixels())
        });
        self.touch(cx);
    }
    fn on_toggle_clipping_mask(
        &mut self,
        _: &keys::ToggleClippingMask,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .active_layer_id
        {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.toggle_clipping_mask(id))
            });
        }
        self.touch(cx);
    }
    fn on_move_layer_down(
        &mut self,
        _: &keys::MoveLayerDown,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.move_active_layer(-1))
        });
        self.touch(cx);
    }
    fn on_move_layer_up(&mut self, _: &keys::MoveLayerUp, _w: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.move_active_layer(1))
        });
        self.touch(cx);
    }
    fn on_cut(&mut self, _: &keys::Cut, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.cut_selection())
            });
            t.touch(cx);
        });
    }
    fn on_copy_pixels(&mut self, _: &keys::CopyPixels, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.copy_selection())
            });
            t.touch(cx);
        });
    }
    fn on_copy_merged(&mut self, _: &keys::CopyMerged, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.copy_merged_selection())
            });
            t.touch(cx);
        });
    }
    fn on_paste_pixels(&mut self, _: &keys::PastePixels, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.paste_pixels())
            });
            t.touch(cx);
        });
    }
    fn on_content_aware_fill(
        &mut self,
        _: &keys::ContentAwareFill,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hotkey(w, cx, |t, _w, cx| t.run_content_aware_fill(cx));
    }
    fn on_clear_selection_pixels(
        &mut self,
        _: &keys::ClearSelectionPixels,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hotkey(w, cx, |t, _w, cx| {
            t.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.clear_selected_pixels())
            });
            t.touch(cx);
        });
    }
    fn on_load_layer_selection(
        &mut self,
        _: &keys::LoadLayerSelection,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .active_layer_id
        {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut().session.update(cx, |session, _| {
                    session.load_layer_selection(
                        id,
                        compositor_core::selection::SelectionMode::Replace,
                    )
                })
            });
            self.touch(cx);
        }
    }
    fn on_load_mask_selection(
        &mut self,
        _: &keys::LoadMaskSelection,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .active_layer_id
        {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut().session.update(cx, |session, _| {
                    session
                        .load_mask_selection(id, compositor_core::selection::SelectionMode::Replace)
                })
            });
            self.touch(cx);
        }
    }
    fn on_clear_guides(&mut self, _: &keys::ClearGuides, _w: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.clear_guides())
        });
        self.touch(cx);
    }
    fn on_toggle_levels_preview(
        &mut self,
        _: &keys::ToggleLevelsPreview,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_levels_preview(cx);
    }
    fn on_finish_lasso(&mut self, _: &keys::FinishLasso, _w: &mut Window, cx: &mut Context<Self>) {
        let (has_lasso, has_transform) = {
            let session = self.workspace.read(cx).current().session.read(cx);
            (
                session.lasso_draft.is_some(),
                session.transform_edit.is_some(),
            )
        };
        if has_lasso {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.finish_lasso())
            });
            self.touch(cx);
        } else if has_transform {
            // Return applies the pending transform (the inspector's Apply).
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.commit_transform())
            });
            self.touch(cx);
        }
    }
    fn on_cancel(&mut self, _: &keys::CancelAction, w: &mut Window, cx: &mut Context<Self>) {
        self.cancel_active(w, cx);
    }

    // MARK: Shared flows

    /// The one "import images" entry (ticket 02): File > Import Images and
    /// the welcome form's Import image button both land here, so the two
    /// stay behaviorally identical — the native multi-select panel, the
    /// import into the current tab, the status line and the viewport refit
    /// (Swift's `showsImporter` flow, NewCanvasSheet.swift:32).
    pub(crate) fn prompt_import_images(&mut self, cx: &mut Context<Self>) {
        // While an import is in flight the welcome form disables; a second
        // trigger must not stack a second picker (`isImporting`, now a
        // session field — stage-D ticket 34).
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .is_importing
        {
            return;
        }
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.is_importing = true)
        });
        // The native multi-select panel (Swift's showsImporter flow).
        let receiver = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let picked = receiver
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or_default()
                .unwrap_or_default();
            this.update(cx, |this, cx| {
                // The flag clears on every path — cancel included — so the
                // form re-enables as soon as the panel closes.
                this.workspace.update(cx, |ws, cx| {
                    ws.current_mut()
                        .session
                        .update(cx, |session, _| session.is_importing = false)
                });
                if picked.is_empty() {
                    this.touch(cx);
                    return;
                }
                let failures = this.workspace.update(cx, |ws, cx| {
                    ws.current_mut()
                        .session
                        .update(cx, |session, _| session.import_images(&picked, None))
                });
                if failures.is_empty() {
                    this.set_status(format!("Imported {} images", picked.len()), cx);
                } else {
                    this.set_status(failures.join("; "), cx);
                }
                this.touch(cx);
            })
            .ok();
        })
        .detach();
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

    fn nudge(&mut self, dx: f64, dy: f64, large: bool, w: &mut Window, cx: &mut Context<Self>) {
        self.hotkey(w, cx, |t, _w, cx| {
            let scale = if large { 10.0 } else { 1.0 };
            let (transforming, move_tool) = {
                let session = t.workspace.read(cx).current().session.read(cx);
                (
                    session.transform_edit.is_some(),
                    session.tool == NavigationTool::Move && session.can_transform(),
                )
            };
            if transforming || move_tool {
                t.workspace.update(cx, |ws, cx| {
                    ws.current_mut()
                        .session
                        .update(cx, |session, _| session.nudge_layer(dx * scale, dy * scale))
                });
            } else {
                t.workspace.update(cx, |ws, cx| {
                    ws.current_mut().session.update(cx, |session, _| {
                        session.nudge_selection(dx * scale, dy * scale)
                    })
                });
            }
            t.touch(cx);
        });
    }
}

/// The status bar (Swift `ContentView.statusBar`, ContentView.swift:311): a
/// view entity since ticket 42 — it owns the status line (app-local state;
/// the shell's `set_status` forwards into it) and observes the session for
/// its readouts (tool name, undo/redo names, dimensions, zoom), so a
/// session notification re-renders the bar alone. It is built fresh with
/// the per-tab content (ticket 43) and observes the session from its
/// constructor.
pub struct StatusBarView {
    /// The active tab's session — the bar's readouts read it directly.
    session: Entity<EditorSession>,
    /// The root view (the window shell). The bar currently has no
    /// outbound flow (its one write, `set_status`, is forwarded from the
    /// root), so the seam has no reader yet — kept for the pattern, like
    /// every child entity's seam since ticket 39.
    #[allow(dead_code)]
    app: WeakEntity<ProjectWorkspaceView>,
    /// The status line's text (`self.status` when it lived on the shell):
    /// the last message any flow set, shown until the next one.
    status: Option<String>,
    /// The observe subscription on the active tab's session entity: a
    /// session-entity notify re-renders this bar alone. Attached once in
    /// the constructor (a rebuild brings a fresh bar); kept in a dedicated
    /// field so the subscription outlives the constructor borrow.
    session_observer: Option<(Entity<EditorSession>, Subscription)>,
}

impl StatusBarView {
    pub(crate) fn new(
        session: Entity<EditorSession>,
        app: WeakEntity<ProjectWorkspaceView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut bar = StatusBarView {
            session,
            app,
            status: None,
            session_observer: None,
        };
        bar.attach_session_observer(cx);
        bar
    }

    /// Observes the session entity so its notifications re-render this bar
    /// alone (the readouts are all session reads). Drops the outgoing
    /// observation.
    fn attach_session_observer(&mut self, cx: &mut Context<Self>) {
        let session = self.session.clone();
        let subscription = cx.observe(&session, |_, _, cx| cx.notify());
        self.session_observer = Some((session, subscription));
    }

    /// Stores the status line's text. The shell's `set_status` forwards here
    /// and keeps the repaint epoch, exactly as when the field lived there.
    pub(crate) fn set_status(&mut self, message: impl Into<String>) {
        self.status = Some(message.into());
    }
}

impl Render for StatusBarView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (width, height) = self
            .session
            .read(cx)
            .document
            .as_ref()
            .map(|d| (d.width as f64, d.height as f64))
            .unwrap_or((0.0, 0.0));
        let (tool, undo, redo, can_undo, can_redo, brush_error) = {
            let session = self.session.read(cx);
            (
                session.tool.label().to_string(),
                session.history.undo_name().to_string(),
                session.history.redo_name().to_string(),
                session.can_undo(),
                session.can_redo(),
                session.brush_error.clone(),
            )
        };
        let zoom = format!("{:.0}%", self.session.read(cx).viewport.zoom() * 100.0);
        let undo_text = if can_undo {
            format!("Undo: {}", undo)
        } else {
            "Undo: \u{2014}".to_string()
        };
        let redo_text = if can_redo {
            format!("Redo: {}", redo)
        } else {
            "Redo: \u{2014}".to_string()
        };
        // The status line reads state; it never consumes it. Import
        // failures surface at the receiving event (`receive_dropped_files`,
        // `prompt_import_images` both write the status themselves), not by
        // a render-time drain.
        let message = self.status.clone().or(brush_error).unwrap_or_default();

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
                        format!("{} \u{d7} {} px", width as usize, height as usize),
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
