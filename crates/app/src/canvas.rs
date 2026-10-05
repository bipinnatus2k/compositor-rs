//! The canvas viewport: paints the composite, checkerboard, rulers, guides,
//! grid, selection outline, lasso draft, crop box and transform box; owns all
//! pointer interaction with the document.

use std::sync::Arc;

use gpui_kit::Styled;
use gpui_kit::{
    Bounds, Context, Corners, DispatchPhase, InteractiveElement, IntoElement, KeyDownEvent,
    KeyUpEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    PathBuilder, Pixels, Point, RenderImage, ScrollDelta, ScrollWheelEvent, Window, canvas, div,
    point, px, rgb, rgba, size,
};
use gpui_kit::{DragMoveEvent, ExternalPaths};

use compositor_core::canvas::CropDragMode;
use compositor_core::document::LayoutGrid;
use compositor_core::document::{CanvasDocument, GuideAxis, ImageLayer, NavigationTool};
use compositor_core::geometry::{CGPoint, CGRect, CGSize};
use compositor_core::raster::Bitmap;
use compositor_core::selection::LassoKind;
use compositor_core::session::{BlurToolMode, BrushToolMode, EditorSession, SpotHealingMode};
use compositor_core::tools::brush::{GradientShape, RasterEdit};
use compositor_core::tools::warp::WarpStroke;

use crate::inline_text::{
    inline_text_box, inline_text_box_corners, inline_text_measured_size, inline_text_overflows,
    inline_text_resize,
};
use crate::state::{AppState, CanvasDrag, FitDims, RULER_SIZE};

/// `guideHitDistance` (Guides.swift): how close a Move press must land to a
/// guide to take it, in screen points.
const GUIDE_HIT_DISTANCE: f64 = 5.0;

/// Everything the paint pass needs, computed once per frame in screen space.
/// Coordinates are viewport-local; `paint_scene` translates them into window
/// coordinates before painting (gpui paints in window space).
#[derive(Clone)]
pub struct CanvasScene {
    pub image: Option<Arc<RenderImage>>,
    pub image_rect: Bounds<Pixels>,
    pub rulers: bool,
    pub zoom: f64,
    pub pan: (f64, f64),
    pub doc: (f64, f64),
    pub guides: Vec<(bool, f32)>,
    pub grid: Vec<(f32, bool, bool)>,
    pub selection: Vec<Vec<(f32, f32)>>,
    pub lasso: Option<Vec<Vec<(f32, f32)>>>,
    pub crop: Option<((f32, f32), (f32, f32))>,
    pub transform: Option<Vec<(f32, f32)>>,
    pub text_box: Option<((f32, f32), (f32, f32))>,
    /// The inline text editor's frame, rotated with the draft
    /// (`InlineTextEditor.draw`): four screen corners plus the edge
    /// midpoints painted as handles.
    pub inline_text_frame: Option<[(f32, f32); 4]>,
    /// Whether the draft's text does not fit its fixed box — a black plus on
    /// the bottom-right handle (`draw`'s overflow marker,
    /// InlineTextEditor.swift:311-326).
    pub inline_text_overflow: bool,
}

impl AppState {
    pub fn render_canvas(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        // An empty tab shows the welcome form in place of a canvas (Swift's
        // `if session.document == nil { welcome }`, ContentView.swift:99-110 —
        // the tab-embedded NewCanvasSheet).
        if self.current_tab().session.document.is_none() {
            return self.render_welcome(cx);
        }
        let composite = self.composite();
        let image = composite
            .as_ref()
            .map(|composite| self.frame_image(composite));
        let scene = self.build_scene(image);
        // The inline text editor rides on the canvas while a draft is open
        // (InlineTextEditor.synchronize runs on every display sync in Swift;
        // here the overlay re-renders with the scene, so zoom and pan keep
        // it anchored).
        let inline_overlay = self.render_inline_text_overlay(cx);
        let app_move = cx.entity().downgrade();
        let app_up = cx.entity().downgrade();
        let app_prepaint = cx.entity().downgrade();

        div()
            .id("canvas-area")
            .relative()
            .flex_1()
            .overflow_hidden()
            .bg(rgb(0x232326))
            .drag_over::<ExternalPaths>(|style, _, _, _| {
                // File drags tint the canvas (Swift highlights the drop area).
                style.bg(gpui_kit::rgb(0x2a2f3a))
            })
            .on_drag_move::<ExternalPaths>(cx.listener(
                |this, event: &DragMoveEvent<ExternalPaths>, _window, _cx| {
                    // Remember the hover point in document coordinates: the
                    // drop lands its imports there (ImageFileDrop's point).
                    let local = this.local_position(event.event.position);
                    this.drop_point_doc = Some(this.screen_to_doc(local.0, local.1));
                },
            ))
            .on_drop::<ExternalPaths>(cx.listener(|this, dropped: &ExternalPaths, _window, cx| {
                let paths = dropped.paths().to_vec();
                this.receive_dropped_files(paths, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.canvas_mouse_down(event, window, cx);
                }),
            )
            .on_scroll_wheel(
                cx.listener(move |this, event: &ScrollWheelEvent, window, cx| {
                    this.canvas_scroll(event, window, cx);
                }),
            )
            .child(
                canvas(
                    move |bounds, _window, cx| {
                        app_prepaint
                            .update(cx, |state, cx| {
                                state.viewport = bounds;
                                let vw = f32::from(bounds.size.width);
                                let vh = f32::from(bounds.size.height);
                                let dims = {
                                    let (w, h) = state.doc_size();
                                    FitDims {
                                        width: w as usize,
                                        height: h as usize,
                                    }
                                };
                                // Swift rebuilds the view per tab
                                // (`.id(tab.id)`), so a tab switch refits
                                // just like a document size change.
                                let tab_changed = state.fit_tab != state.workspace.selected_id();
                                if (dims != state.fit_for || tab_changed)
                                    && dims.width > 0
                                    && vw > 10.0
                                    && vh > 10.0
                                {
                                    state.zoom_fit((vw, vh));
                                    cx.notify();
                                }
                            })
                            .ok();
                    },
                    move |bounds, _prepaint, window, _cx| {
                        paint_scene(&scene, bounds, window);
                        install_drag_events(&app_move, &app_up, window);
                        install_key_events(&app_move, window);
                    },
                )
                .absolute()
                .inset_0(),
            )
            .children(inline_overlay)
            .into_any_element()
    }

    fn build_scene(&self, image: Option<Arc<RenderImage>>) -> CanvasScene {
        let image_rect = self.image_rect();
        let mut guides = Vec::new();
        let mut grid = Vec::new();
        let mut selection = Vec::new();
        let mut lasso = None;
        let mut crop = None;
        let mut transform = None;
        let mut text_box = None;
        let mut inline_text_frame = None;
        let mut inline_text_overflow = false;

        if let Some(document) = self.current_tab().session.document.as_ref() {
            // `displayedGuides`: a drag in progress replaces its own guide's
            // position live; a new guide joins the list while it is pulled
            // from the ruler.
            let mut drag_drawn = false;
            for guide in &document.guides {
                let (axis, position) = match &self.current_tab().session.guide_drag {
                    Some(drag) if drag.id == guide.id => (drag.axis, drag.position),
                    _ => (guide.axis, guide.position),
                };
                drag_drawn |=
                    self.current_tab().session.guide_drag.as_ref().map(|d| d.id) == Some(guide.id);
                let vertical = axis == GuideAxis::Vertical;
                let position = if vertical {
                    self.doc_to_screen(position, 0.0).0
                } else {
                    self.doc_to_screen(0.0, position).1
                };
                guides.push((vertical, position));
            }
            if let Some(drag) = &self.current_tab().session.guide_drag
                && (drag.is_new || !drag_drawn)
            {
                let vertical = drag.axis == GuideAxis::Vertical;
                let position = if vertical {
                    self.doc_to_screen(drag.position, 0.0).0
                } else {
                    self.doc_to_screen(0.0, drag.position).1
                };
                guides.push((vertical, position));
            }
            if self.current_tab().session.shows_grid {
                for x in LayoutGrid::lines(document.width as f64) {
                    grid.push((self.doc_to_screen(x, 0.0).0, LayoutGrid::is_major(x), true));
                }
                for y in LayoutGrid::lines(document.height as f64) {
                    grid.push((self.doc_to_screen(0.0, y).1, LayoutGrid::is_major(y), false));
                }
            }
            // The ants follow a pixel move or outline drag (`displayedSelection`).
            if let Some(selection_state) = self.current_tab().session.displayed_selection() {
                for ring in &selection_state.path.rings {
                    selection.push(ring.iter().map(|p| self.doc_to_screen(p.x, p.y)).collect());
                }
            } else if let Some(selection_state) = &document.selection {
                for ring in &selection_state.path.rings {
                    selection.push(ring.iter().map(|p| self.doc_to_screen(p.x, p.y)).collect());
                }
            }
            if let Some(draft) = &self.current_tab().session.lasso_draft {
                let mut rings = vec![
                    draft
                        .points
                        .iter()
                        .map(|p| self.doc_to_screen(p.x, p.y))
                        .collect::<Vec<(f32, f32)>>(),
                ];
                if let Some(cursor) = draft.cursor
                    && !rings[0].is_empty()
                {
                    let (x, y) = self.doc_to_screen(cursor.x, cursor.y);
                    rings[0].push((x, y));
                }
                lasso = Some(rings);
            }
            if let Some(rect) = self.current_tab().session.crop_rect {
                let min = self.doc_to_screen(rect.min_x(), rect.min_y());
                let max = self.doc_to_screen(rect.max_x(), rect.max_y());
                crop = Some((min, max));
            }
            if let Some(edit) = &self.current_tab().session.transform_edit {
                // TransformOverlay.swift: handles hide with ⌘H unless an edit must stay visible.
                if edit.draft.is_valid()
                    && (self.current_tab().session.shows_transform_controls || edit.persistent)
                {
                    transform = Some(
                        edit.draft
                            .corners()
                            .iter()
                            .map(|p| self.doc_to_screen(p.x, p.y))
                            .collect(),
                    );
                }
            }
            // The Type tool's box drag preview (`drawTextBoxDraft`).
            if let Some(rect) = self.text_box_rect {
                text_box = Some((
                    self.doc_to_screen(rect.min_x(), rect.min_y()),
                    self.doc_to_screen(rect.max_x(), rect.max_y()),
                ));
            }
            // The inline text editor's frame and handles: the draft's shown
            // frame (`synchronize`'s shownTransform — for point text on a
            // layer, grown with the fresh measurement and anchored back at
            // its top-left corner) rotated about its center.
            if let Some(draft) = &self.current_tab().session.text_draft {
                let measured = inline_text_measured_size(&draft.style);
                let asset_width = self.inline_text_asset_width(draft);
                let ((ox, oy), (w, h)) = inline_text_box(
                    draft.origin,
                    draft.transform,
                    draft.style.box_size,
                    measured,
                    asset_width,
                    self.zoom,
                    self.pan,
                );
                let rotation = draft.transform.map(|t| t.rotation).unwrap_or(0.0);
                inline_text_frame = Some(inline_text_box_corners((ox, oy), (w, h), rotation));
                inline_text_overflow = inline_text_overflows(&draft.style);
            }
        }

        CanvasScene {
            image,
            image_rect,
            rulers: self.current_tab().session.shows_rulers,
            zoom: self.zoom,
            pan: self.pan,
            doc: self.doc_size(),
            guides,
            grid,
            selection,
            lasso,
            crop,
            transform,
            text_box,
            inline_text_frame,
            inline_text_overflow,
        }
    }

    // MARK: Pointer handling

    pub fn canvas_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sheet.is_some() {
            // The Camera Raw defringe eyedropper samples through the open
            // sheet instead of touching the tools.
            if self.camera_raw_samples_defringe {
                let local = self.local_position(event.position);
                let doc = self.screen_to_doc(local.0, local.1);
                self.sample_camera_raw_defringe(doc, cx);
            }
            return;
        }
        if event.click_count > 1 {
            // Double click closes a polygonal lasso instead of starting a drag.
            if self.tab().session.tool == NavigationTool::Lasso
                && self.tab().session.lasso_kind == LassoKind::Polygonal
                && self.tab().session.lasso_draft.is_some()
            {
                self.tab().session.finish_lasso();
                self.touch(cx);
            }
            return;
        }
        let local = self.local_position(event.position);
        let doc = self.screen_to_doc(local.0, local.1);
        let alt = event.modifiers.alt;
        let shift = event.modifiers.shift;

        // The inline text editor's resize handles take a press before any
        // tool (`InlineTextEditor.hitTest`/`mouseDown`); the editor itself
        // swallows presses inside the overlay, so only these reach us.
        if self.tab().session.text_draft.is_some()
            && let Some(handle) = self.inline_text_handle_hit(doc)
        {
            self.begin_inline_text_resize(doc, handle, cx);
            return;
        }

        // A press on a ruler strip starts a guide at the pointer's document
        // coordinate, whatever the tool (`CanvasRulerNSView.mouseDown`); the
        // corner square is inert. On the canvas itself, only a Move press that
        // lands on a guide takes it — before any layer drag
        // (EditorCanvas.mouseDown's `beginGuideDrag`).
        if self.tab().session.shows_rulers
            && let Some(axis) = ruler_axis_at(local)
        {
            self.tab()
                .session
                .begin_guide_creation(axis, guide_axis_position(axis, doc));
            self.set_drag(Some(CanvasDrag::Guide));
            self.touch(cx);
            return;
        }
        if self.tab().session.tool == NavigationTool::Move {
            // `guideHitDistance` is 5 screen points; the core measures in
            // document pixels.
            let tolerance = GUIDE_HIT_DISTANCE / self.zoom.max(0.0001);
            if let Some(guide_id) = self.tab().session.hit_guide(doc, tolerance) {
                self.tab().session.begin_guide_move(guide_id);
                self.set_drag(Some(CanvasDrag::Guide));
                self.touch(cx);
                return;
            }
        }

        match self.tab().session.tool {
            NavigationTool::Hand => {
                self.set_drag(Some(CanvasDrag::Pan {
                    start: event.position,
                    start_pan: self.pan,
                }));
            }
            NavigationTool::Zoom => {
                if alt {
                    self.zoom_at(local, 0.5);
                } else {
                    self.zoom_at(local, 2.0);
                }
            }
            NavigationTool::Marquee | NavigationTool::Lasso => {
                // The polygonal lasso accumulates one point per click; Enter
                // (or double click) closes the outline.
                let polygonal = self.tab().session.tool == NavigationTool::Lasso
                    && self.tab().session.lasso_kind == LassoKind::Polygonal;
                if polygonal && self.tab().session.lasso_draft.is_some() {
                    self.tab().session.extend_lasso(doc);
                    self.touch(cx);
                    return;
                }
                // Cmd-drag inside the selection cuts and moves its pixels
                // (Photoshop's temporary Move tool, Option duplicates); a
                // plain Replace-mode drag inside it moves the outline
                // (EditorCanvas.lassoMouseDown).
                let ctrl = event.modifiers.control;
                let inside = self
                    .tab()
                    .session
                    .selection()
                    .map(|s| s.path.contains(doc))
                    .unwrap_or(false);
                if ctrl && inside {
                    if self.tab().session.begin_pixel_move(alt) {
                        self.set_drag(Some(CanvasDrag::PixelMove { start: doc }));
                        self.touch(cx);
                    }
                    return;
                }
                if !ctrl && !shift && !alt && inside && self.tab().session.begin_selection_move() {
                    self.set_drag(Some(CanvasDrag::SelectionMove { start: doc }));
                    self.touch(cx);
                    return;
                }
                let mode = self.tab().session.selection_mode(shift, alt);
                self.tab().session.begin_lasso(doc, mode);
                self.set_drag(Some(CanvasDrag::MarqueeLasso));
            }
            NavigationTool::Wand => {
                let mode = self.tab().session.selection_mode(shift, alt);
                self.tab().session.magic_wand(doc, mode);
            }
            NavigationTool::Brush
            | NavigationTool::SpotHealing
            | NavigationTool::CloneStamp
            | NavigationTool::Blur => {
                if self.tab().session.tool == NavigationTool::Blur {
                    match self.tab().session.blur_mode {
                        BlurToolMode::Smudge | BlurToolMode::Liquify => {
                            self.begin_warp(doc);
                        }
                        BlurToolMode::Blur => {
                            self.begin_stroke(doc, alt, cx);
                        }
                    }
                } else {
                    self.begin_stroke(doc, alt, cx);
                }
            }
            NavigationTool::Gradient => {
                self.begin_gradient(doc);
            }
            NavigationTool::Move => {
                self.begin_move(doc, event.modifiers.control, cx);
            }
            NavigationTool::Crop => {
                self.begin_crop(doc, cx);
            }
            NavigationTool::Eyedropper => {
                if let Some(document) = self.tab().session.document.as_ref()
                    && let Some(color) =
                        compositor_core::render::sample_composite_color(document, doc)
                {
                    self.tab().session.brush_settings.red = color.red;
                    self.tab().session.brush_settings.green = color.green;
                    self.tab().session.brush_settings.blue = color.blue;
                    self.set_status(format!("Foreground #{}", color.hex()), cx);
                }
            }
            NavigationTool::Type => {
                self.type_mouse_down(doc, window, cx);
            }
            NavigationTool::Shape => {
                self.begin_shape(doc);
            }
            _ => {}
        }
        self.touch(cx);
    }

    pub fn canvas_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sheet.is_some() {
            return;
        }
        let local = self.local_position(event.position);
        let doc = self.screen_to_doc(local.0, local.1);
        let shift = event.modifiers.shift;
        let alt = event.modifiers.alt;

        match self.drag.as_mut() {
            Some(CanvasDrag::Pan { start, start_pan }) => {
                self.pan = (
                    start_pan.0 + (f32::from(event.position.x) - f32::from(start.x)) as f64,
                    start_pan.1 + (f32::from(event.position.y) - f32::from(start.y)) as f64,
                );
            }
            Some(CanvasDrag::Stroke(edit, _)) => {
                edit.append_smoothed(doc);
            }
            Some(CanvasDrag::Warp(stroke)) => {
                stroke.append(doc);
            }
            Some(CanvasDrag::Shape { start }) => {
                let rect = shape_drag_rect(
                    self.workspace.current_mut().session.shape_kind,
                    *start,
                    doc,
                    shift,
                    alt,
                );
                let shape = self.build_shape_for_drag(rect);
                self.shape_rect = Some(rect);
                self.shape_shape = Some(shape);
            }
            Some(CanvasDrag::Gradient(..)) => {
                self.gradient_cursor = Some(doc);
            }
            Some(CanvasDrag::MarqueeLasso) => {
                let kind = self
                    .workspace
                    .current_mut()
                    .session
                    .lasso_draft
                    .as_ref()
                    .map(|d| d.kind);
                if matches!(kind, Some(LassoKind::Rectangle) | Some(LassoKind::Ellipse)) {
                    self.workspace
                        .current_mut()
                        .session
                        .drag_marquee(doc, shift, false);
                } else {
                    self.workspace.current_mut().session.extend_lasso(doc);
                }
            }
            Some(CanvasDrag::TextBox { start }) => {
                // The box draft follows the pointer (`dragTextGesture`).
                let rect = compositor_core::selection::drag_box_rect(*start, doc, false, false);
                self.text_box_rect = Some(rect);
            }
            Some(CanvasDrag::TextResize {
                handle,
                start,
                old,
                source,
                draft,
            }) => {
                // The handle drag resizes the draft's box live
                // (`InlineTextEditor.mouseDragged`).
                let (handle, start) = (*handle, *start);
                let (old, source, draft) = (*old, *source, draft.clone());
                if let Some(resized) = inline_text_resize(&old, source, handle, start, doc) {
                    let mut draft = draft;
                    draft.style.box_size = Some(resized.box_size);
                    draft.origin = resized.transform.origin;
                    draft.transform = Some(resized.transform);
                    self.workspace.current_mut().session.text_draft = Some(draft);
                }
            }
            Some(CanvasDrag::MoveLayer {
                start_origin,
                start,
            }) => {
                let delta = CGPoint::new(doc.x - start.x, doc.y - start.y);
                let mut value = *start_origin;
                value.origin.x += delta.x;
                value.origin.y += delta.y;
                // Moving snaps to the canvas and the other layers; resizing
                // and rotating are left alone (EditorCanvas.dragTransform).
                let moving: Vec<uuid::Uuid> = {
                    let session = self.workspace.current_mut();
                    session
                        .session
                        .transform_edit
                        .as_ref()
                        .map(|edit| match &edit.group {
                            Some(group) => group.originals.iter().map(|(id, _)| *id).collect(),
                            None => vec![edit.layer_id],
                        })
                        .unwrap_or_default()
                };
                let session = self.workspace.current_mut();
                let value = session.session.snapped_move(value, &moving, 10.0);
                session.session.preview_transform(value);
                // The inspector's fields follow the drag live
                // (TransformInspector's binding to editedTransform).
                self.sync_transform_inputs(window, cx);
            }
            Some(CanvasDrag::PixelMove { start }) => {
                self.workspace
                    .current_mut()
                    .session
                    .drag_pixel_move(CGSize::new(doc.x - start.x, doc.y - start.y));
            }
            Some(CanvasDrag::SelectionMove { start }) => {
                self.workspace
                    .current_mut()
                    .session
                    .move_selection_outline(CGSize::new(doc.x - start.x, doc.y - start.y));
            }
            Some(CanvasDrag::Crop) => {
                let ratio = self.workspace.current_mut().session.crop_ratio();
                self.workspace
                    .current_mut()
                    .session
                    .drag_crop(doc, ratio, false);
            }
            Some(CanvasDrag::Guide) => {
                // The guide follows the pointer's document coordinate on its
                // own axis (EditorCanvas.mouseDragged's guideDragging branch).
                if let Some(drag) = self.workspace.current_mut().session.guide_drag.as_ref() {
                    let axis = drag.axis;
                    self.workspace
                        .current_mut()
                        .session
                        .move_guide_drag(guide_axis_position(axis, doc));
                }
            }
            None => return,
        }
        self.touch(cx);
    }

    pub fn canvas_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let drag = self.drag.take();
        self.workspace.current_mut().busy.has_brush_stroke = false;
        if let Some(drag) = drag {
            match drag {
                CanvasDrag::Stroke(mut edit, id) => {
                    edit.flush();
                    self.commit_stroke(edit, id, cx);
                }
                CanvasDrag::Gradient(edit, start, id) => {
                    let local = self.local_position(event.position);
                    self.gradient_cursor = Some(self.screen_to_doc(local.0, local.1));
                    let result = self.gradient_result(&edit, start);
                    self.commit_pixels(
                        compositor_core::raster::ImportedImage::new(result, String::new()),
                        EditTarget {
                            layer_id: id,
                            is_mask: edit.is_mask,
                        },
                        edit.paint_transform,
                        "Gradient",
                        cx,
                    );
                }
                CanvasDrag::Warp(stroke) => {
                    self.finish_warp(stroke, cx);
                }
                CanvasDrag::Shape { .. } => {
                    self.finish_shape(cx);
                }
                CanvasDrag::MarqueeLasso => {
                    self.tab().session.finish_lasso();
                }
                CanvasDrag::PixelMove { .. } => {
                    self.tab().session.end_pixel_move();
                }
                CanvasDrag::SelectionMove { .. } => {
                    self.tab().session.end_selection_move();
                }
                CanvasDrag::Crop => {
                    self.tab().session.end_crop_drag();
                }
                CanvasDrag::MoveLayer { .. } => {
                    self.tab().session.commit_transform();
                    self.sync_transform_inputs(window, cx);
                }
                CanvasDrag::Guide => {
                    // Releasing on a ruler cancels a new guide or deletes an
                    // existing one (`finishGuideDrag(delete:)`).
                    let local = self.local_position(event.position);
                    let delete = guide_release_deletes(self.tab().session.shows_rulers, local);
                    self.tab().session.finish_guide_drag(delete);
                }
                CanvasDrag::Pan { .. } => {}
                // The box lays its text after the match (`finishTextGesture`).
                CanvasDrag::TextBox { .. } => {}
                // The resize ends with the draft as it stands; the input
                // keeps the focus (`mouseUp` returns it to the text view).
                CanvasDrag::TextResize { .. } => {}
            }
            self.gradient_cursor = None;
            self.touch(cx);
        }
        // The Type tool's box drag decides here (`finishTextGesture`), even
        // when the drag was empty: the release lays the text either way.
        if self.drag.is_none() && self.text_box_rect.is_some() {
            self.type_mouse_up(window, cx);
        }
    }

    pub fn canvas_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sheet.is_some() {
            return;
        }
        let local = self.local_position(event.position);
        if local.0 < 0.0
            || local.1 < 0.0
            || local.0 > f32::from(self.viewport.size.width)
            || local.1 > f32::from(self.viewport.size.height)
        {
            return;
        }
        let dy = match event.delta {
            ScrollDelta::Pixels(pt) => f32::from(pt.y),
            ScrollDelta::Lines(pt) => pt.y * 40.0,
        };
        if dy != 0.0 {
            let factor = (-dy / 400.0).exp();
            self.zoom_at(local, factor as f64);
            self.touch(cx);
        }
    }

    /// Starts/ends a canvas drag, keeping the tab's `has_brush_stroke` busy
    /// flag in step (Swift keeps `brushStroke`/`warpStroke` on the session;
    /// the port mirrors it app-side so `can_switch` blocks tab changes
    /// mid-stroke).
    pub(crate) fn set_drag(&mut self, drag: Option<CanvasDrag>) {
        self.drag = drag;
        self.workspace.current_mut().busy.has_brush_stroke = matches!(
            self.drag,
            Some(CanvasDrag::Stroke(..))
                | Some(CanvasDrag::Warp(_))
                | Some(CanvasDrag::Gradient(..))
                | Some(CanvasDrag::Shape { .. })
        );
    }

    pub(crate) fn local_position(&self, position: Point<Pixels>) -> (f32, f32) {
        (
            f32::from(position.x) - f32::from(self.viewport.origin.x),
            f32::from(position.y) - f32::from(self.viewport.origin.y),
        )
    }

    // MARK: Strokes

    fn begin_stroke(&mut self, doc: CGPoint, alt: bool, cx: &mut Context<Self>) {
        let Some(document) = self.tab().session.document.clone() else {
            return;
        };
        let Some(id) = self.tab().session.active_layer_id else {
            return;
        };
        let Some(layer) = document.layer(id) else {
            return;
        };
        // canPaint (EditorSession+Brush.swift:5-12) splits into the surface
        // rules and the state guards, both ahead of everything else —
        // Swift's guard runs before the clone-source handling too.
        let Some(target) = paint_target(&self.tab().session, &document, layer) else {
            return;
        };
        if !can_paint(&self.tab().session, &document) {
            return;
        }
        let mask_target = target.is_mask;
        // Spot Healing and Clone Stamp rework image pixels; they have
        // nothing to do on a mask (EditorSession+Brush.swift:31).
        if mask_target
            && matches!(
                self.tab().session.tool,
                NavigationTool::SpotHealing | NavigationTool::CloneStamp
            )
        {
            return;
        }
        // Clone Stamp: option-click sets the source; strokes copy from it.
        if self.tab().session.tool == NavigationTool::CloneStamp {
            if alt {
                self.tab().session.clone_source = Some(doc);
                self.set_status(
                    format!("Clone source set at {:.0}, {:.0}", doc.x, doc.y),
                    cx,
                );
                return;
            }
            if self.tab().session.clone_source.is_none() {
                self.set_status("Option-click to set a clone source first", cx);
                return;
            }
        }
        let mut settings = self.tab().session.brush_settings;
        settings.erasing = self.tab().session.brush_mode == BrushToolMode::Erase;
        settings.healing = self.tab().session.tool == NavigationTool::SpotHealing;
        // Smoothing trails the pointer on a string of `smoothing` screen
        // points — in document pixels the radius divides by the zoom. Only
        // the Brush tool smooths (EditorSession+Brush.swift:78).
        if self.tab().session.tool == NavigationTool::Brush {
            settings.smoothing /= self.zoom.max(0.0001);
        } else {
            settings.smoothing = 0.0;
        }
        // A mask stroke paints white (reveal) or black (hide).
        if mask_target {
            let v = if self.tab().session.mask_paint_white {
                1.0
            } else {
                0.0
            };
            settings.red = v;
            settings.green = v;
            settings.blue = v;
        }
        let clip = document.selection.as_ref().map(|s| s.clip(document.size()));
        let mut edit = RasterEdit::new(
            target.source,
            target.base_transform,
            mask_target,
            settings,
            CGSize::new(document.width as f64, document.height as f64),
            clip,
        );
        // Clone Stamp: aligned strokes keep the first stroke's offset, unaligned
        // ones run from the brush to the source each time (CloneStamp.swift:23-27
        // — `cloneStrokeOffset` plus the `cloneOffset = offset` fixup done only
        // once the sample succeeded). This used to recompute source-minus-brush
        // every stroke, which made aligned stamping drift.
        if self.tab().session.tool == NavigationTool::CloneStamp
            && let Some(offset) = self.tab().session.clone_stroke_offset(doc)
            && let Some(sample) = self.sample_for_clone()
        {
            if self.tab().session.clone_settings.aligned
                && self.tab().session.clone_offset.is_none()
            {
                self.tab().session.clone_offset = Some(offset);
            }
            edit.clone = Some((sample, offset));
        }
        // The Blur brush: the stroke's own target sampled at its start — the
        // layer as displayed, or the mask's own coverage when the mask is the
        // target — softened by an amount that follows the brush size, painted
        // in place through the tip (BlurTool.swift:8-40,
        // EditorSession+Brush.swift:42-46). This used to sample the layer even
        // on a mask target, stamping layer pixels into the mask.
        if self.tab().session.tool == NavigationTool::Blur {
            let displayed = self.tab().session.displayed_transform(layer);
            if let Some(sample) = blur_sample(&document, layer, &displayed, mask_target) {
                let blurred =
                    blur_sample_softened(&sample, self.tab().session.brush_settings.diameter);
                edit.clone = Some((blurred, CGSize::ZERO));
            }
        }
        edit.append_smoothed(doc);
        self.set_drag(Some(CanvasDrag::Stroke(edit, id)));
    }

    /// What the Clone Stamp samples: the composite or the active layer.
    fn sample_for_clone(&self) -> Option<Bitmap> {
        let document = self.current_tab().session.document.as_ref()?;
        if self.current_tab().session.clone_settings.sample_all_layers {
            return Some(compositor_core::render::draw_live_composite(document));
        }
        // The active layer as the canvas displays it: drawn through its
        // displayed transform at document size (CloneStamp.swift:39-48).
        // Swift's `cloneSample` does not multiply the sample by the layer's
        // mask coverage — a stamped stroke clones the layer's own pixels even
        // where the mask hides them — so neither do we (this used to darken
        // the sample under an enabled mask).
        let layer = self.current_tab().session.active_layer()?;
        let image = layer.asset.as_ref()?.image.as_ref().clone();
        let transform = self.current_tab().session.displayed_transform(layer);
        Some(clone_layer_sample(
            &image,
            &transform,
            document.width,
            document.height,
        ))
    }
}

/// Which surface a raster commit lands on: the layer captured when the
/// edit began — never whatever is active at mouse-up — or its mask.
pub(crate) struct EditTarget {
    pub layer_id: uuid::Uuid,
    pub is_mask: bool,
}

/// The paint-surface half of `canPaint` (EditorSession+Brush.swift:5-12),
/// shared by brush strokes and gradients: a selected, enabled mask is the
/// surface; groups have no pixels of their own and an adjustment layer's
/// pixels are not paintable.
struct PaintTarget {
    source: Option<Bitmap>,
    base_transform: compositor_core::transform::LayerTransform,
    is_mask: bool,
}

fn paint_target(
    session: &EditorSession,
    _document: &CanvasDocument,
    layer: &ImageLayer,
) -> Option<PaintTarget> {
    // A selected mask is the ONLY surface, and it must be enabled (:11) —
    // a disabled or missing mask under a selected mask paints nothing at
    // all, it does not fall through to the layer's pixels.
    if session.is_mask_selected {
        let m = layer.mask.as_ref()?;
        if !m.is_enabled {
            return None;
        }
        return Some(PaintTarget {
            source: Some(m.asset.image.as_ref().clone()),
            base_transform: layer.mask_transform(),
            is_mask: true,
        });
    }
    // Groups have no pixels of their own; only their mask is paintable (:7).
    if layer.is_group {
        return None;
    }
    // An adjustment layer's pixels are not paintable (:12).
    if layer.adjustment.is_some() {
        return None;
    }
    Some(PaintTarget {
        source: layer.asset.as_ref().map(|a| a.image.as_ref().clone()),
        base_transform: layer.transform,
        is_mask: false,
    })
}

/// The rest of `canPaint` (:5-12) beyond the surface rules: the session's
/// UI-idle conjuncts (`canEditLayers`, EditorSession.swift:600-602 — a
/// text draft, a pending adjustment or crop, a pixel move all block a
/// stroke), exactly one layer selected, the active layer still
/// effectively visible, and an explicitly empty selection leaves nothing
/// paintable. The conjuncts that live app-side here: a brush or warp
/// stroke IS the drag, the project-busy/import flags ride the tab, and
/// a pending core gradient never coexists with a fresh press (Swift's
/// beginGradient bypasses canEditLayers for its re-anchor too).
fn can_paint(session: &EditorSession, document: &CanvasDocument) -> bool {
    if session.selection_amount_operation.is_some()
        || session.text_draft.is_some()
        || session.shows_new_document
        || session.transform_edit.is_some()
        || session.crop_rect.is_some()
        || session.pixel_move.is_some()
        || session.hue_saturation.is_some()
        || session.levels.is_some()
        || session.filter_edit.is_some()
        || session.adjustment_editing_id.is_some()
    {
        return false;
    }
    if session.selected_layer_ids.len() != 1 {
        return false;
    }
    let Some(id) = session.active_layer_id else {
        return false;
    };
    if !document.effective_visible_ids().contains(&id) {
        return false;
    }
    !(document
        .selection
        .as_ref()
        .map(|s| s.path.is_empty())
        .unwrap_or(false))
}

/// The Clone Stamp's layer sample (`cloneSample`'s active-layer half,
/// CloneStamp.swift:39-48): the layer's own pixels drawn through the given
/// transform at document size. The layer's mask does not darken the sample —
/// Swift copies the layer's pixels, masked or not.
pub(crate) fn clone_layer_sample(
    image: &Bitmap,
    transform: &compositor_core::transform::LayerTransform,
    width: usize,
    height: usize,
) -> Bitmap {
    let mut canvas = Bitmap::new(width, height);
    compositor_core::render::draw_layer_pixels(image, transform, &mut canvas);
    canvas
}

/// The blur strength for a brush diameter: `min(30, max(1.5, diameter / 10))`
/// (BlurTool.swift:10).
pub(crate) fn blur_sigma(diameter: f64) -> f64 {
    (diameter / 10.0).clamp(1.5, 30.0)
}

/// `blurSample_softened` — the sampled frame Gaussian-softened by the blur
/// strength the brush diameter implies.
pub(crate) fn blur_sample_softened(sample: &Bitmap, diameter: f64) -> Bitmap {
    compositor_core::raster::gaussian_blur_rgba(sample, blur_sigma(diameter))
}

/// What a Blur stroke paints, taken when the stroke starts (BlurTool.swift:8-40):
/// with the mask as the target, the mask's own coverage in the document grid —
/// its edge tone kept past its pixels —; otherwise the active layer as the
/// canvas displays it. The sample is painted in place through the tip.
pub(crate) fn blur_sample(
    document: &CanvasDocument,
    layer: &ImageLayer,
    displayed: &compositor_core::transform::LayerTransform,
    mask_target: bool,
) -> Option<Bitmap> {
    if mask_target {
        blur_mask_sample(document, layer)
    } else {
        let image = layer.asset.as_ref()?.image.as_ref().clone();
        let mut canvas = Bitmap::new(document.width, document.height);
        compositor_core::render::draw_layer_pixels(&image, displayed, &mut canvas);
        Some(canvas)
    }
}

/// The mask half of `blurSample(mask: true)` (BlurTool.swift:11-28): the
/// document grid filled with the mask's background tone — past its pixels a
/// mask keeps its edge tone, so blurring near its edge doesn't pull in the
/// wrong one — with the mask's own coverage drawn over it.
pub(crate) fn blur_mask_sample(document: &CanvasDocument, layer: &ImageLayer) -> Option<Bitmap> {
    let mask = layer.mask.as_ref()?;
    if !mask.is_enabled {
        return None;
    }
    let mut canvas = Bitmap::new(document.width, document.height);
    let background = (compositor_core::document::LayerMask::background_of(&mask.asset.thumbnail)
        * 255.0)
        .round() as u8;
    for px in canvas.data.chunks_exact_mut(4) {
        px.copy_from_slice(&[background, background, background, 255]);
    }
    let transform = layer.mask_transform();
    let image = mask.asset.image.as_ref().clone();
    compositor_core::render::draw_layer_pixels(&image, &transform, &mut canvas);
    Some(canvas)
}

impl AppState {
    fn commit_stroke(
        &mut self,
        mut edit: RasterEdit,
        layer_id: uuid::Uuid,
        cx: &mut Context<Self>,
    ) {
        if !edit.has_paint() {
            return;
        }
        // Spot Healing: the wash was only ever a preview; the stroke ends by
        // rebuilding the painted area from nearby texture (HealPixels.c).
        if edit.settings.healing {
            let mode = match self.tab().session.spot_healing_mode {
                SpotHealingMode::ContentAware => 0,
                SpotHealingMode::CreateTexture => 1,
                SpotHealingMode::ProximityMatch => 2,
            };
            // Swift reseeds every edit (BrushStroke.swift:793).
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() ^ (d.as_secs() as u32))
                .unwrap_or(0x6d2b79f5);
            edit.heal(mode, seed);
            let (snapshot, transform, _) = edit.paint_snapshot();
            let target = EditTarget {
                layer_id,
                is_mask: edit.is_mask,
            };
            self.commit_pixels(snapshot, target, transform, "Spot Healing", cx);
            return;
        }
        let erasing = edit.settings.erasing;
        let (snapshot, transform, _) = edit.paint_snapshot();
        // Stroke names follow the tool (EditorSession+Brush.swift:141).
        let name = if edit.is_mask {
            "Paint Mask"
        } else if erasing {
            "Erase"
        } else if self.tab().session.tool == NavigationTool::Blur {
            "Blur"
        } else if self.tab().session.tool == NavigationTool::CloneStamp {
            "Clone Stamp"
        } else {
            "Brush Stroke"
        };
        let target = EditTarget {
            layer_id,
            is_mask: edit.is_mask,
        };
        self.commit_pixels(snapshot, target, transform, name, cx);
    }

    // MARK: Smudge / Liquify

    /// `beginWarp`: the stroke works on the active layer as the canvas shows
    /// it, at document size.
    fn begin_warp(&mut self, doc: CGPoint) {
        let Some(document) = self.tab().session.document.clone() else {
            return;
        };
        let Some(id) = self.tab().session.active_layer_id else {
            return;
        };
        let Some(layer) = document.layer(id) else {
            return;
        };
        let Some(image) = layer.asset.as_ref().map(|a| a.image.as_ref().clone()) else {
            return;
        };
        let mut surface = Bitmap::new(document.width.max(1), document.height.max(1));
        compositor_core::render::draw_layer_pixels(&image, &layer.transform, &mut surface);
        let mut stroke = WarpStroke::new(
            surface,
            self.tab().session.blur_mode,
            &self.tab().session.brush_settings,
        );
        stroke.layer_asset = layer.asset.as_ref().map(|a| a.image.clone());
        stroke.layer_transform = layer.transform;
        stroke.append(doc);
        self.set_drag(Some(CanvasDrag::Warp(stroke)));
    }

    /// `finishWarp`: paints the finished result into the layer's own pixels
    /// along the stroke, as one undo step.
    fn finish_warp(&mut self, stroke: WarpStroke, cx: &mut Context<Self>) {
        let name = match stroke.mode {
            BlurToolMode::Smudge => "Smudge",
            BlurToolMode::Liquify => "Liquify",
            BlurToolMode::Blur => "Blur",
        };
        let Some(document) = self.tab().session.document.clone() else {
            return;
        };
        let Some(id) = self.tab().session.active_layer_id else {
            return;
        };
        let Some(layer) = document.layer(id) else {
            return;
        };
        // The stroke captured the layer it started on; if it changed under
        // the warp, the result is dropped (finishWarp's identity guard).
        if stroke.layer_asset.as_deref() != layer.asset.as_ref().map(|a| a.image.as_ref())
            || stroke.layer_transform != layer.transform
        {
            return;
        }
        let source = layer.asset.as_ref().map(|a| a.image.as_ref().clone());
        let selection_clip = document.selection.as_ref().map(|s| s.clip(document.size()));
        let diameter = stroke.diameter;
        let points = stroke.points.clone();
        let result = stroke.into_image();
        if let Some((snapshot, transform, _)) = compositor_core::tools::warp::finish_warp_into_layer(
            &result,
            &points,
            diameter,
            source,
            layer.transform,
            document.size(),
            name,
            selection_clip,
        ) {
            // The warp always targets the layer it started on.
            self.commit_pixels(
                snapshot,
                EditTarget {
                    layer_id: id,
                    is_mask: false,
                },
                transform,
                name,
                cx,
            );
        }
    }

    // MARK: Shape

    /// The shape the current drag places, from the tool's settings.
    fn build_shape_for_drag(&self, _rect: CGRect) -> compositor_core::document::LayerShape {
        let kind = self.current_tab().session.shape_kind;
        let color = self.current_tab().session.palette_color(false);
        compositor_core::document::LayerShape {
            kind,
            corner_radius: self.current_tab().session.shape_corner_radius,
            line_width: self.current_tab().session.shape_line_width,
            red: color.red,
            green: color.green,
            blue: color.blue,
            filled: kind != compositor_core::document::ShapeKind::Line,
            start: CGPoint::ZERO,
            end: CGPoint::new(1.0, 1.0),
        }
    }

    fn begin_shape(&mut self, doc: CGPoint) {
        self.set_drag(Some(CanvasDrag::Shape { start: doc }));
        self.shape_rect = Some(CGRect::new(doc.x, doc.y, 0.0, 0.0));
        self.shape_shape = Some(self.build_shape_for_drag(CGRect::new(doc.x, doc.y, 0.0, 0.0)));
    }

    /// `finishShape`: one undo step; a click without a drag makes nothing.
    fn finish_shape(&mut self, cx: &mut Context<Self>) {
        self.shape_shape = None;
        let Some(rect) = self.shape_rect.take() else {
            return;
        };
        if rect.width() < 1.0 || rect.height() < 1.0 {
            return;
        }
        let kind = self.tab().session.shape_kind;
        let color = self.tab().session.palette_color(false);
        let shape = compositor_core::document::LayerShape {
            kind,
            corner_radius: self.tab().session.shape_corner_radius,
            line_width: self.tab().session.shape_line_width,
            red: color.red,
            green: color.green,
            blue: color.blue,
            filled: kind != compositor_core::document::ShapeKind::Line,
            start: CGPoint::ZERO,
            end: CGPoint::new(1.0, 1.0),
        };
        let name = self.tab().session.next_shape_name(kind);
        self.tab().session.add_shape_layer(shape, rect, name);
        self.touch(cx);
    }

    /// Stores new pixels into the active layer (or its mask, when the stroke
    /// painted one) as one undoable step. A mask commit replaces the mask's
    /// pixels only — enabled/placement/link state survives
    /// (`LayerMask.replacing`).
    pub(crate) fn commit_pixels(
        &mut self,
        image: compositor_core::raster::ImportedImage,
        target: EditTarget,
        transform: compositor_core::transform::LayerTransform,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        self.history_edit(name, move |session| {
            if let Some(doc) = session.document.as_mut() {
                // The edit lands on the layer captured when it began —
                // never on whatever happens to be active at mouse-up
                // (EditorSession+Brush.swift:130-133).
                if let Some(layer) = doc.layer_mut(target.layer_id) {
                    let mut image = image;
                    if target.is_mask {
                        if let Some(m) = layer.mask.as_mut() {
                            image.name = m.asset.name.clone();
                            m.asset = image;
                        }
                    } else {
                        image.name = layer.name.clone();
                        layer.asset = Some(image);
                        layer.transform = transform;
                    }
                }
            }
        });
        self.touch(cx);
    }

    /// Wraps a plain bitmap before committing it.
    pub(crate) fn commit_bitmap(
        &mut self,
        bitmap: Bitmap,
        layer_id: uuid::Uuid,
        transform: compositor_core::transform::LayerTransform,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        self.commit_pixels(
            compositor_core::raster::ImportedImage::new(bitmap, String::new()),
            EditTarget {
                layer_id,
                is_mask: false,
            },
            transform,
            name,
            cx,
        );
    }

    // MARK: Gradient

    fn begin_gradient(&mut self, doc: CGPoint) {
        let Some(document) = self.tab().session.document.clone() else {
            return;
        };
        let Some(id) = self.tab().session.active_layer_id else {
            return;
        };
        let Some(layer) = document.layer(id) else {
            return;
        };
        // Gradients land on the same surface under the same canPaint gate
        // as a brush stroke (Gradient.swift:36-38, EditorSession+Brush.swift:
        // 5-12) — a drag that would refuse to paint never opens a draft.
        let Some(target) = paint_target(&self.tab().session, &document, layer) else {
            return;
        };
        if !can_paint(&self.tab().session, &document) {
            return;
        }
        let clip = document.selection.as_ref().map(|s| s.clip(document.size()));
        let edit = RasterEdit::new(
            target.source,
            target.base_transform,
            target.is_mask,
            self.tab().session.brush_settings,
            CGSize::new(document.width as f64, document.height as f64),
            clip,
        );
        self.gradient_cursor = Some(doc);
        self.set_drag(Some(CanvasDrag::Gradient(edit, doc, id)));
    }

    // MARK: Move

    fn begin_move(&mut self, doc: CGPoint, command: bool, cx: &mut Context<Self>) {
        // A picked layer becomes the transform target before the drag starts.
        if self.tab().session.transform_edit.is_none() {
            let active_id = self.tab().session.active_layer_id;
            let auto_select = self.transform_auto_select;
            if let Some(document) = self.tab().session.document.as_ref()
                && let Some((id, picked)) =
                    transform_press_layer(document, active_id, auto_select, command, true, doc)
                && picked
            {
                self.tab().session.select_layer(Some(id));
            }
        }
        if self.tab().session.transform_edit.is_none() {
            if self.tab().session.can_transform() {
                // The drag begins a transient transform (persistent false,
                // EditorCanvas.swift:2129).
                self.tab().session.begin_transform_persistent(false);
            } else {
                return;
            }
        }
        let Some(draft) = self
            .tab()
            .session
            .transform_edit
            .as_ref()
            .map(|edit| edit.draft)
        else {
            return;
        };
        self.set_drag(Some(CanvasDrag::MoveLayer {
            start_origin: draft,
            start: doc,
        }));
        self.touch(cx);
    }

    // MARK: Crop

    fn begin_crop(&mut self, doc: CGPoint, cx: &mut Context<Self>) {
        let mode = if let Some(rect) = self.tab().session.crop_rect {
            match handle_at(rect, doc) {
                Some(index) => CropDragMode::Resize(index),
                None if rect.contains_point(doc) => CropDragMode::Move,
                None => CropDragMode::Create,
            }
        } else {
            CropDragMode::Create
        };
        self.tab().session.begin_crop_drag(doc, mode);
        self.set_drag(Some(CanvasDrag::Crop));
        self.touch(cx);
    }

    // MARK: Type

    /// The Type tool's press (`beginTextGesture`): a press on a visible text
    /// layer edits it inline; anywhere else the press anchors a text box
    /// drag, and the release decides between point text and a paragraph box.
    /// Any open draft commits first, so clicking away applies the text
    /// (empty drafts are a quiet no-op, as in Swift).
    fn type_mouse_down(&mut self, doc: CGPoint, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab().session.document.is_none() {
            return;
        }
        if !self.tab().session.finish_text() {
            self.touch(cx);
            return;
        }
        self.sync_inline_text(window, cx);
        let document = self.tab().session.document.clone().unwrap();
        let visible = document.effective_visible_ids();
        if let Some(layer) = document.layers.iter().rev().find(|l| {
            visible.contains(&l.id) && l.live_text().is_some() && l.transform.contains(doc)
        }) {
            let id = layer.id;
            self.tab().session.select_layer(Some(id));
            self.tab().session.edit_active_text();
            self.sync_inline_text(window, cx);
            self.touch(cx);
            return;
        }
        self.text_box_rect = Some(CGRect::new(doc.x, doc.y, 0.0, 0.0));
        self.set_drag(Some(CanvasDrag::TextBox { start: doc }));
    }

    /// The Type tool's release (`finishTextGesture`): a click without a drag
    /// lays point text at the click; a drag sets the paragraph bounds.
    fn type_mouse_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rect) = self.text_box_rect.take() else {
            return;
        };
        if rect.width() < 4.0 && rect.height() < 4.0 {
            self.tab().session.begin_text(rect.origin, true);
        } else {
            self.tab().session.begin_text_in(rect);
        }
        self.sync_inline_text(window, cx);
        self.touch(cx);
    }

    // MARK: Keys

    /// Tab cycles the active tool's mode.
    pub fn cycle_tool_mode(&mut self, cx: &mut Context<Self>) {
        if self.tab().session.tool == NavigationTool::Shape {
            self.tab().session.toggle_shape_kind();
            self.touch(cx);
            return;
        }
        match self.tab().session.tool {
            NavigationTool::Marquee => self.tab().session.toggle_marquee_kind(),
            NavigationTool::Lasso => self.tab().session.toggle_lasso_kind(),
            NavigationTool::Wand => {
                self.tab().session.wand_mode = match self.tab().session.wand_mode {
                    compositor_core::selection::WandMode::Wand => {
                        compositor_core::selection::WandMode::Object
                    }
                    compositor_core::selection::WandMode::Object => {
                        compositor_core::selection::WandMode::Wand
                    }
                };
            }
            NavigationTool::Brush => {
                self.tab().session.brush_mode = match self.tab().session.brush_mode {
                    BrushToolMode::Paint => BrushToolMode::Erase,
                    BrushToolMode::Erase => BrushToolMode::Paint,
                };
            }
            NavigationTool::Gradient => {
                self.tab().session.gradient_settings.shape =
                    match self.tab().session.gradient_settings.shape {
                        GradientShape::Linear => GradientShape::Radial,
                        GradientShape::Radial => GradientShape::Linear,
                    };
            }
            _ => {}
        }
        self.touch(cx);
    }

    /// Escape: cancel the active edit, close the topmost panel, or close the
    /// sheet. The order mirrors Swift's cancel chain: the color picker first,
    /// then the floating panels (each panel's cancel is its `onClose`
    /// semantics), then a modal sheet, then the canvas edits.
    pub fn cancel_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab().session.color_picker.is_some() {
            self.close_color_picker_ui(false, cx);
            return;
        }
        if let Some(name) = self.panels.top_most() {
            self.cancel_panel(name, cx);
            return;
        }
        if self.sheet.is_some() {
            // Route through cancel_sheet so sheet-specific cancel semantics
            // run (the Effects sheet restores the edited effect).
            self.cancel_sheet(window, cx);
            return;
        }
        if self.tab().session.lasso_draft.take().is_some() {
            self.touch(cx);
            return;
        }
        // The Type tool's box drag preview dies with the gesture.
        if self.text_box_rect.take().is_some() {
            self.touch(cx);
            return;
        }
        if self.tab().session.crop_drag.take().is_some() {
            self.tab().session.crop_rect = None;
            self.touch(cx);
            return;
        }
        // Escape kills a guide drag before it reaches the transform edit
        // (EditorCanvas.keyDown's `guideDrag != nil` branch).
        if self.tab().session.guide_drag.take().is_some() {
            self.touch(cx);
            return;
        }
        if self.tab().session.pixel_move.take().is_some() {
            self.tab().session.cancel_pixel_move();
            self.touch(cx);
            return;
        }
        if self.tab().session.selection_move_origin.take().is_some() {
            self.tab().session.end_selection_move();
            self.touch(cx);
            return;
        }
        if self.tab().session.transform_edit.take().is_some() {
            self.touch(cx);
            return;
        }
        if self.tab().session.crop_rect.take().is_some() {
            self.touch(cx);
            return;
        }
        if self.rename.take().is_some() {
            self.rename_input = None;
            self.touch(cx);
        }
    }
}

/// The guide axis a viewport-local point would drag from a ruler strip, or
/// None off the rulers — the corner square between them is inert, as in
/// Swift, where only the two ruler views handle presses
/// (`CanvasRulerNSView.mouseDown`; `CanvasRulerCorner` has no gesture).
fn ruler_axis_at(local: (f32, f32)) -> Option<GuideAxis> {
    let on_top = local.1 < RULER_SIZE;
    let on_left = local.0 < RULER_SIZE;
    match (on_top, on_left) {
        (true, false) => Some(GuideAxis::Horizontal),
        (false, true) => Some(GuideAxis::Vertical),
        _ => None,
    }
}

/// The document coordinate a guide on `axis` follows: x for a vertical
/// guide, y for a horizontal one (`EditorCanvas.documentPosition(axis:at:)`).
fn guide_axis_position(axis: GuideAxis, doc: CGPoint) -> f64 {
    if axis == GuideAxis::Vertical {
        doc.x
    } else {
        doc.y
    }
}

/// Whether releasing at a viewport-local point deletes the dragged guide:
/// onto the ruler strip (`EditorCanvas.isOverRuler` — top or left, only when
/// the rulers are showing).
fn guide_release_deletes(shows_rulers: bool, local: (f32, f32)) -> bool {
    shows_rulers && (local.0 < RULER_SIZE || local.1 < RULER_SIZE)
}

/// The drag rect for the shape tool (`ShapeTool.dragShape`): Shift squares
/// non-line shapes and snaps a line's angle to eighths of a turn, Option
/// grows a non-line shape symmetrically from its anchor. A line ignores both
/// beyond the snap — it is stored as its bounding box.
/// `transformPressLayer` (EditorCanvas.swift:1317-1350): the layer a
/// transform press drags, and whether it was picked from under the
/// pointer. Cmd picks the layer under the pointer; otherwise the active
/// layer, unless auto-select finds another layer there — including one
/// stacked above a selected background that also contains the press. A
/// press on empty canvas still drags the active layer.
fn transform_press_layer(
    document: &compositor_core::document::CanvasDocument,
    active_id: Option<uuid::Uuid>,
    auto_select: bool,
    command: bool,
    can_pick: bool,
    pixel: compositor_core::geometry::CGPoint,
) -> Option<(uuid::Uuid, bool)> {
    let order = document.render_layers();
    let under = order
        .iter()
        .rev()
        .find(|l| l.asset.is_some() && l.transform.contains(pixel))
        .map(|l| l.id);
    let active = active_id
        .and_then(|id| order.iter().find(|l| l.id == id))
        .copied();
    // Cmd picks the layer under the pointer on its own.
    if command
        && can_pick
        && let Some(id) = under
    {
        return Some((id, true));
    }
    if let Some(active) = active
        && active.transform.contains(pixel)
    {
        // `render_layers` is bottom to top, so a later index paints
        // above. Prefer that layer when auto-select is on; a
        // full-canvas background contains every press, and keeping it
        // would hide a foreground layer stacked on top of it.
        if can_pick
            && auto_select
            && let (Some(under), Some(active)) = (under, active_id)
            && under != active
        {
            let top = order.iter().position(|l| l.id == under);
            let current = order.iter().position(|l| l.id == active);
            if let (Some(top), Some(current)) = (top, current)
                && top > current
            {
                return Some((under, true));
            }
        }
        return Some((active.id, false));
    }
    if can_pick
        && (auto_select || command)
        && let Some(id) = under
    {
        return Some((id, true));
    }
    active.map(|l| (l.id, false))
}

fn shape_drag_rect(
    kind: compositor_core::document::ShapeKind,
    anchor: CGPoint,
    end: CGPoint,
    square: bool,
    from_center: bool,
) -> CGRect {
    let is_line = kind == compositor_core::document::ShapeKind::Line;
    let mut end = end;
    if is_line && square {
        // Shift on a line snaps its angle to eighths of a turn.
        let dx = end.x - anchor.x;
        let dy = end.y - anchor.y;
        let angle =
            ((dy.atan2(dx)) / std::f64::consts::FRAC_PI_4).round() * std::f64::consts::FRAC_PI_4;
        let length = dx.hypot(dy);
        end = CGPoint::new(
            anchor.x + angle.cos() * length,
            anchor.y + angle.sin() * length,
        );
    }
    let mut rect = compositor_core::selection::drag_box_rect(
        anchor,
        end,
        !is_line && square,
        !is_line && from_center,
    );
    if !is_line && from_center {
        // Option grows the shape from its center.
        rect = CGRect::new(
            2.0 * anchor.x - end.x,
            2.0 * anchor.y - end.y,
            (end.x - anchor.x) * 2.0,
            (end.y - anchor.y) * 2.0,
        );
    }
    rect
}

/// Which of the crop rect's 8 handles is under `doc`, if any.
fn handle_at(rect: CGRect, doc: CGPoint) -> Option<usize> {
    const HANDLE: f64 = 8.0;
    let corners = [
        (rect.min_x(), rect.min_y()),
        (rect.mid_x(), rect.min_y()),
        (rect.max_x(), rect.min_y()),
        (rect.max_x(), rect.mid_y()),
        (rect.max_x(), rect.max_y()),
        (rect.mid_x(), rect.max_y()),
        (rect.min_x(), rect.max_y()),
        (rect.min_x(), rect.mid_y()),
    ];
    for (index, (x, y)) in corners.iter().enumerate() {
        if (doc.x - x).abs() <= HANDLE && (doc.y - y).abs() <= HANDLE {
            return Some(index);
        }
    }
    None
}

// MARK: Painting

fn paint_scene(scene: &CanvasScene, bounds: Bounds<Pixels>, window: &mut Window) {
    // Viewport background.
    window.paint_quad(gpui_kit::fill(bounds, rgb(0x232326)));

    // The scene is viewport-local; painting happens in window coordinates.
    let scene = scene.translated(f32::from(bounds.origin.x), f32::from(bounds.origin.y));

    let image_bounds = scene.image_rect;

    // Checkerboard behind the document.
    if f32::from(image_bounds.size.width) > 0.0 {
        paint_checkerboard(window, bounds, image_bounds);
    }

    // The composite itself.
    if let Some(image) = &scene.image {
        let _ = window.paint_image(
            bounds,
            image_bounds,
            Corners::default(),
            image.clone(),
            0,
            false,
        );
        paint_rect_outline(window, image_bounds, rgba(0x00000066));
    }

    // Layout grid.
    for (position, major, vertical) in &scene.grid {
        let color = if *major {
            rgba(0x8a8a8a55)
        } else {
            rgba(0x8a8a8a2a)
        };
        paint_line(window, bounds, *position, *vertical, color);
    }

    // Guides.
    for (vertical, position) in &scene.guides {
        paint_line(window, bounds, *position, *vertical, rgba(0x00c8ffcc));
    }

    // Selection outline: a white solid line under an animated black dash
    // (TransformOverlay.drawSelection's marching ants).
    for ring in &scene.selection {
        if ring.len() >= 3 {
            paint_polygon(window, ring, rgba(0xffffffff), false);
            paint_polygon(window, ring, rgba(0x000000ff), true);
        }
    }

    // Lasso draft.
    if let Some(rings) = &scene.lasso {
        for ring in rings {
            if ring.len() >= 2 {
                paint_polygon(window, ring, rgba(0xffffffff), false);
                paint_polygon(window, ring, rgba(0x000000ff), true);
            }
        }
    }

    // Transform outline with corner handles.
    if let Some(corners) = &scene.transform
        && corners.len() >= 3
    {
        paint_polygon(window, corners, rgba(0x4da6ffdd), false);
        for corner in corners {
            paint_handle(window, point(px(corner.0), px(corner.1)));
        }
    }

    // Crop overlay.
    if let Some(((x0, y0), (x1, y1))) = scene.crop {
        let rect = Bounds::<Pixels>::new(
            point(px(x0), px(y0)),
            size(px((x1 - x0).max(1.0)), px((y1 - y0).max(1.0))),
        );
        paint_darken_outside(window, bounds, rect);
        paint_rect_outline(window, rect, rgba(0xffffffff));
        let w = f32::from(rect.size.width);
        let h = f32::from(rect.size.height);
        for fraction in [1.0 / 3.0, 2.0 / 3.0] {
            paint_line(
                window,
                bounds,
                f32::from(rect.origin.x) + w * fraction,
                true,
                rgba(0xffffff44),
            );
            paint_line(
                window,
                bounds,
                f32::from(rect.origin.y) + h * fraction,
                false,
                rgba(0xffffff44),
            );
        }
        for corner in [
            (x0, y0),
            ((x0 + x1) / 2.0, y0),
            (x1, y0),
            (x1, (y0 + y1) / 2.0),
            (x1, y1),
            ((x0 + x1) / 2.0, y1),
            (x0, y1),
            (x0, (y0 + y1) / 2.0),
        ] {
            paint_handle(window, point(px(corner.0), px(corner.1)));
        }
    }

    // The Type tool's box draft: a thin accent outline (`drawTextBoxDraft`).
    if let Some(((x0, y0), (x1, y1))) = scene.text_box {
        let rect = Bounds::<Pixels>::new(
            point(px(x0), px(y0)),
            size(px((x1 - x0).max(1.0)), px((y1 - y0).max(1.0))),
        );
        paint_rect_outline(window, rect, rgba(0x4da6ffdd));
    }

    // The inline text editor's frame and its eight resize handles
    // (`InlineTextEditor.draw`): an accent outline plus white squares with
    // accent borders at the corners and the edge midpoints.
    if let Some(corners) = &scene.inline_text_frame {
        paint_polygon(window, corners, rgba(0x4da6ffdd), false);
        for index in 0..4 {
            let a = corners[index];
            let b = corners[(index + 1) % 4];
            paint_handle(window, point(px((a.0 + b.0) / 2.0), px((a.1 + b.1) / 2.0)));
        }
        for corner in corners {
            paint_handle(window, point(px(corner.0), px(corner.1)));
        }
        // Text that doesn't fit is marked by a black plus on the
        // bottom-right handle, as in Photoshop (`draw`'s overflow marker,
        // InlineTextEditor.swift:311-326: handles[4], the bottom-right
        // corner; arm 0.42 of the handle size, a stroke a sixth as wide).
        if scene.inline_text_overflow {
            paint_overflow_plus(window, corners[2]);
        }
    }

    // Rulers on top.
    if scene.rulers {
        paint_rulers(window, bounds, &scene);
    }
}

impl CanvasScene {
    /// The same scene shifted into window coordinates.
    fn translated(&self, ox: f32, oy: f32) -> CanvasScene {
        let shift = |points: &[(f32, f32)]| -> Vec<(f32, f32)> {
            points.iter().map(|(x, y)| (x + ox, y + oy)).collect()
        };
        let shift_rings = |rings: &Vec<Vec<(f32, f32)>>| -> Vec<Vec<(f32, f32)>> {
            rings.iter().map(|ring| shift(ring)).collect()
        };
        let mut out = self.clone();
        let r = &mut out.image_rect;
        r.origin = point(
            px(f32::from(r.origin.x) + ox),
            px(f32::from(r.origin.y) + oy),
        );
        out.guides = out
            .guides
            .iter()
            .map(|(vertical, position)| {
                (
                    *vertical,
                    if *vertical {
                        position + ox
                    } else {
                        position + oy
                    },
                )
            })
            .collect();
        out.grid = out
            .grid
            .iter()
            .map(|(position, major, vertical)| {
                (
                    if *vertical {
                        position + ox
                    } else {
                        position + oy
                    },
                    *major,
                    *vertical,
                )
            })
            .collect();
        out.selection = shift_rings(&self.selection);
        out.lasso = self.lasso.as_ref().map(shift_rings);
        out.crop = self
            .crop
            .map(|((x0, y0), (x1, y1))| ((x0 + ox, y0 + oy), (x1 + ox, y1 + oy)));
        out.transform = self.transform.as_ref().map(|corners| shift(corners));
        out.text_box = self
            .text_box
            .map(|((x0, y0), (x1, y1))| ((x0 + ox, y0 + oy), (x1 + ox, y1 + oy)));
        out.inline_text_frame = self.inline_text_frame.map(|corners| -> [(f32, f32); 4] {
            let mut shifted = corners;
            for (x, y) in &mut shifted {
                *x += ox;
                *y += oy;
            }
            shifted
        });
        out
    }
}

fn paint_checkerboard(window: &mut Window, viewport: Bounds<Pixels>, image: Bounds<Pixels>) {
    const CELL: f32 = 8.0;
    let light = rgb(0xffffff);
    let dark = rgb(0xcccccc);
    let x0 = f32::from(image.origin.x).max(f32::from(viewport.origin.x));
    let y0 = f32::from(image.origin.y).max(f32::from(viewport.origin.y));
    let x1 = (f32::from(image.origin.x) + f32::from(image.size.width))
        .min(f32::from(viewport.origin.x) + f32::from(viewport.size.width));
    let y1 = (f32::from(image.origin.y) + f32::from(image.size.height))
        .min(f32::from(viewport.origin.y) + f32::from(viewport.size.height));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let cols = (((x1 - x0) / CELL).ceil() as usize).min(500);
    let rows = (((y1 - y0) / CELL).ceil() as usize).min(500);
    for row in 0..=rows {
        for col in 0..=cols {
            let cx = x0 + col as f32 * CELL;
            let cy = y0 + row as f32 * CELL;
            if cx >= x1 || cy >= y1 {
                continue;
            }
            let w = CELL.min(x1 - cx);
            let h = CELL.min(y1 - cy);
            let color = if (row + col) % 2 == 0 { light } else { dark };
            window.paint_quad(gpui_kit::fill(
                Bounds::<Pixels>::new(point(px(cx), px(cy)), size(px(w), px(h))),
                color,
            ));
        }
    }
}

fn paint_line(
    window: &mut Window,
    viewport: Bounds<Pixels>,
    position: f32,
    vertical: bool,
    color: gpui_kit::Rgba,
) {
    let (origin, line_size) = if vertical {
        (
            point(px(position), viewport.origin.y),
            size(px(1.0), viewport.size.height),
        )
    } else {
        (
            point(px(f32::from(viewport.origin.x)), px(position)),
            size(px(f32::from(viewport.size.width)), px(1.0)),
        )
    };
    window.paint_quad(gpui_kit::fill(
        Bounds::<Pixels>::new(origin, line_size),
        color,
    ));
}

fn paint_rect_outline(window: &mut Window, rect: Bounds<Pixels>, color: gpui_kit::Rgba) {
    let thickness = 1.0;
    let edges = [
        Bounds::new(
            rect.origin,
            size(px(f32::from(rect.size.width)), px(thickness)),
        ),
        Bounds::new(
            point(
                px(f32::from(rect.origin.x)),
                px(f32::from(rect.origin.y) + f32::from(rect.size.height) - thickness),
            ),
            size(px(f32::from(rect.size.width)), px(thickness)),
        ),
        Bounds::new(
            rect.origin,
            size(px(thickness), px(f32::from(rect.size.height))),
        ),
        Bounds::new(
            point(
                px(f32::from(rect.origin.x) + f32::from(rect.size.width) - thickness),
                px(f32::from(rect.origin.y)),
            ),
            size(px(thickness), px(f32::from(rect.size.height))),
        ),
    ];
    for edge in edges {
        window.paint_quad(gpui_kit::fill(edge, color));
    }
}

fn paint_polygon(window: &mut Window, points: &[(f32, f32)], color: gpui_kit::Rgba, dashed: bool) {
    let screen: Vec<Point<Pixels>> = points.iter().map(|(x, y)| point(px(*x), px(*y))).collect();
    let mut builder = PathBuilder::stroke(px(1.0));
    if dashed {
        builder = builder.dash_array(&[px(4.0), px(4.0)]);
    }
    builder.add_polygon(&screen, true);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn paint_handle(window: &mut Window, at: Point<Pixels>) {
    const SIZE: f32 = 8.0;
    let rect = Bounds::<Pixels>::new(
        point(
            px(f32::from(at.x) - SIZE / 2.0),
            px(f32::from(at.y) - SIZE / 2.0),
        ),
        size(px(SIZE), px(SIZE)),
    );
    window.paint_quad(gpui_kit::fill(rect, rgb(0xffffff)));
    paint_rect_outline(window, rect, rgba(0x000000cc));
}

/// The overflow plus: a black cross centered on the bottom-right handle,
/// `arm` half its length and the handle's stroke a sixth of its size wide
/// (`draw`'s overflow marker, InlineTextEditor.swift:316-325 — the same
/// proportions at the port's fixed 8-pixel handle).
fn paint_overflow_plus(window: &mut Window, at: (f32, f32)) {
    const SIZE: f32 = 8.0;
    let arm = SIZE * 0.42;
    let stroke = SIZE / 6.0;
    let color = rgba(0x000000ff);
    let horizontal = Bounds::<Pixels>::new(
        point(px(at.0 - arm), px(at.1 - stroke / 2.0)),
        size(px(arm * 2.0), px(stroke)),
    );
    let vertical = Bounds::<Pixels>::new(
        point(px(at.0 - stroke / 2.0), px(at.1 - arm)),
        size(px(stroke), px(arm * 2.0)),
    );
    window.paint_quad(gpui_kit::fill(horizontal, color));
    window.paint_quad(gpui_kit::fill(vertical, color));
}

fn paint_darken_outside(window: &mut Window, viewport: Bounds<Pixels>, rect: Bounds<Pixels>) {
    let color = rgba(0x00000077);
    let vw = f32::from(viewport.origin.x);
    let vh = f32::from(viewport.origin.y);
    let strips = [
        Bounds::new(
            point(px(vw), px(vh)),
            size(
                px(f32::from(viewport.size.width)),
                px((f32::from(rect.origin.y) - vh).max(0.0)),
            ),
        ),
        Bounds::new(
            point(
                px(vw),
                px(f32::from(rect.origin.y) + f32::from(rect.size.height)),
            ),
            size(
                px(f32::from(viewport.size.width)),
                px((vh + f32::from(viewport.size.height)
                    - f32::from(rect.origin.y)
                    - f32::from(rect.size.height))
                .max(0.0)),
            ),
        ),
        Bounds::new(
            point(px(vw), px(f32::from(rect.origin.y))),
            size(
                px((f32::from(rect.origin.x) - vw).max(0.0)),
                px(f32::from(rect.size.height)),
            ),
        ),
        Bounds::new(
            point(
                px(f32::from(rect.origin.x) + f32::from(rect.size.width)),
                px(f32::from(rect.origin.y)),
            ),
            size(
                px((vw + f32::from(viewport.size.width)
                    - f32::from(rect.origin.x)
                    - f32::from(rect.size.width))
                .max(0.0)),
                px(f32::from(rect.size.height)),
            ),
        ),
    ];
    for strip in strips {
        if f32::from(strip.size.width) > 0.0 && f32::from(strip.size.height) > 0.0 {
            window.paint_quad(gpui_kit::fill(strip, color));
        }
    }
}

fn paint_rulers(window: &mut Window, bounds: Bounds<Pixels>, scene: &CanvasScene) {
    let thickness = RULER_SIZE;
    let top = Bounds::new(
        bounds.origin,
        size(px(f32::from(bounds.size.width)), px(thickness)),
    );
    let left = Bounds::new(
        bounds.origin,
        size(px(thickness), px(f32::from(bounds.size.height))),
    );
    let background = rgb(0x2b2b2b);
    window.paint_quad(gpui_kit::fill(top, background));
    window.paint_quad(gpui_kit::fill(left, background));

    // Ticks every `tick` document pixels, chosen for roughly 50 px on screen.
    let tick = nice_tick(70.0 / scene.zoom.max(0.001));
    let color = rgba(0xb4b4b4aa);
    let mut value = 0.0f64;
    while value * scene.zoom <= scene.doc.0 * scene.zoom {
        let screen = scene.pan.0 + value * scene.zoom;
        if screen >= thickness as f64 && screen <= f32::from(bounds.size.width) as f64 {
            let x = f32::from(bounds.origin.x) + screen as f32;
            window.paint_quad(gpui_kit::fill(
                Bounds::new(
                    point(px(x), px(f32::from(bounds.origin.y) + thickness - 6.0)),
                    size(px(1.0), px(6.0)),
                ),
                color,
            ));
        }
        value += tick;
    }
    let mut value = 0.0f64;
    while value * scene.zoom <= scene.doc.1 * scene.zoom {
        let screen = scene.pan.1 + value * scene.zoom;
        if screen >= thickness as f64 && screen <= f32::from(bounds.size.height) as f64 {
            let y = f32::from(bounds.origin.y) + screen as f32;
            window.paint_quad(gpui_kit::fill(
                Bounds::new(
                    point(px(f32::from(bounds.origin.x) + thickness - 6.0), px(y)),
                    size(px(6.0), px(1.0)),
                ),
                color,
            ));
        }
        value += tick;
    }
    window.paint_quad(gpui_kit::fill(
        Bounds::new(bounds.origin, size(px(thickness), px(thickness))),
        rgb(0x333333),
    ));
}

/// Numbered ticks about 70 points apart, using 1-2-5 steps in document
/// pixels (`CanvasRulers.majorStep`).
fn nice_tick(rough: f64) -> f64 {
    let candidates = [
        1.0, 2.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0, 200.0, 250.0, 500.0, 1_000.0, 2_000.0,
        2_500.0, 5_000.0, 10_000.0, 20_000.0, 25_000.0,
    ];
    candidates
        .iter()
        .copied()
        .find(|candidate| *candidate >= rough)
        .unwrap_or(50_000.0)
}

/// Whole-pixel tick label (`CanvasRulers.label`).
fn ruler_label(value: f64) -> String {
    let rounded = value.round();
    if rounded == 0.0 {
        "0".to_string()
    } else {
        (rounded as i64).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        blur_mask_sample, blur_sample, blur_sample_softened, blur_sigma, can_paint,
        clone_layer_sample, guide_axis_position, guide_release_deletes, handle_at, nice_tick,
        paint_target, ruler_axis_at, ruler_label, shape_drag_rect, transform_press_layer,
    };
    use crate::state::RULER_SIZE;
    use compositor_core::document::{CanvasDocument, GuideAxis, ImageLayer, LayerMask, ShapeKind};
    use compositor_core::geometry::{CGPoint, CGRect, CGSize};
    use compositor_core::raster::{Bitmap, ImportedImage};
    use compositor_core::selection::{DocumentSelection, SelectionPath};
    use compositor_core::session::EditorSession;
    use compositor_core::transform::LayerTransform;
    use uuid::Uuid;

    fn opaque_layer(width: usize, height: usize, rgba: [u8; 4], origin: CGPoint) -> ImageLayer {
        let mut image = Bitmap::new(width, height);
        for px in image.data.chunks_exact_mut(4) {
            px.copy_from_slice(&rgba);
        }
        ImageLayer::with_asset(ImportedImage::new(image, "layer"), origin)
    }

    #[test]
    fn blur_sigma_follows_the_brush_diameter_with_swifts_clamp() {
        // sigma = min(30, max(1.5, diameter / 10)) (BlurTool.swift:10).
        assert_eq!(blur_sigma(5.0), 1.5);
        assert_eq!(blur_sigma(200.0), 20.0);
        assert_eq!(blur_sigma(2000.0), 30.0);
    }

    /// `canPaint` (EditorSession+Brush.swift:5-12) split across
    /// [`paint_target`] (the surface) and [`can_paint`] (the state guards):
    /// single selection, effective visibility, explicit-empty selection,
    /// enabled mask, group/adjustment pixel rules.
    #[test]
    fn paint_target_and_can_paint_match_swift_canpaint() {
        let mut session = EditorSession::new();
        session.create_document(40, 40, false);
        session.add_blank_layer();
        let document = session.document.clone().unwrap();
        let id = session.active_layer_id.unwrap();
        let layer = document.layer(id).unwrap().clone();

        // A plain visible layer under a single selection: its own pixels.
        let target = paint_target(&session, &document, &layer).expect("paintable");
        assert!(!target.is_mask);
        assert!(can_paint(&session, &document));

        // A second selected layer breaks canPaint (:6).
        session.selected_layer_ids.push(Uuid::new_v4());
        assert!(!can_paint(&session, &document));
        session.selected_layer_ids.truncate(1);

        // An invisible layer loses effective visibility (:10).
        session
            .document
            .as_mut()
            .unwrap()
            .layer_mut(id)
            .unwrap()
            .is_visible = false;
        let document = session.document.clone().unwrap();
        assert!(!can_paint(&session, &document));
        session
            .document
            .as_mut()
            .unwrap()
            .layer_mut(id)
            .unwrap()
            .is_visible = true;
        let document = session.document.clone().unwrap();

        // An explicitly empty selection paints nothing; a absent selection
        // paints everywhere (:6).
        session.document.as_mut().unwrap().selection = Some(DocumentSelection {
            path: SelectionPath::default(),
            antialiased: true,
            feather: 0.0,
        });
        let document = session.document.clone().unwrap();
        assert!(!can_paint(&session, &document));
        session.document.as_mut().unwrap().selection = None;

        // A group has no pixels of its own: only a mask is a surface (:7).
        let mut group = opaque_layer(10, 10, [255, 0, 0, 255], CGPoint::ZERO);
        group.is_group = true;
        assert!(paint_target(&session, &document, &group).is_none());

        // An enabled, selected mask IS a surface; a disabled one is not
        // (:11), and with the mask deselected the layer's pixels are.
        let mut masked = opaque_layer(40, 40, [255, 0, 0, 255], CGPoint::ZERO);
        let mut mask_image = Bitmap::new(40, 40);
        for px in mask_image.data.chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 255, 255, 255]);
        }
        masked.mask = Some(LayerMask {
            asset: ImportedImage::new(mask_image, "mask"),
            is_enabled: true,
            placement: None,
            is_linked: true,
        });
        session.is_mask_selected = true;
        let target = paint_target(&session, &document, &masked).expect("mask paintable");
        assert!(target.is_mask);
        masked.mask.as_mut().unwrap().is_enabled = false;
        assert!(paint_target(&session, &document, &masked).is_none());
        session.is_mask_selected = false;
        assert!(
            !paint_target(&session, &document, &masked)
                .expect("layer pixels paintable again")
                .is_mask
        );

        // The canEditLayers conjuncts (EditorSession.swift:600-602): an
        // open adjustment editor or inline text draft blocks a stroke
        // even with a perfect target.
        let document = session.document.clone().unwrap();
        assert!(can_paint(&session, &document));
        session.adjustment_editing_id = Some(Uuid::new_v4());
        assert!(!can_paint(&session, &document));
        session.adjustment_editing_id = None;
        session.text_draft = Some(compositor_core::session::TextDraft {
            document_id: Uuid::new_v4(),
            layer_id: None,
            origin: CGPoint::ZERO,
            transform: None,
            style: Default::default(),
        });
        assert!(!can_paint(&session, &document));
    }

    #[test]
    fn blur_sample_on_the_layer_draws_it_as_displayed() {
        // blurSample(mask: false): the layer's own pixels through the given
        // transform, at document size (BlurTool.swift:31-40).
        let document = CanvasDocument::new(40, 40);
        let layer = opaque_layer(20, 20, [255, 0, 0, 255], CGPoint::new(10.0, 10.0));
        let sample = blur_sample(&document, &layer, &layer.transform, false).unwrap();
        assert_eq!(
            (sample.width, sample.height),
            (document.width, document.height)
        );
        assert_eq!(sample.pixel(15, 15), [255, 0, 0, 255]);
        // Outside the layer the sample is empty.
        assert_eq!(sample.pixel(5, 5), [0, 0, 0, 0]);
    }

    #[test]
    fn blur_sample_on_a_mask_takes_the_masks_own_coverage() {
        // blurSample(mask: true): the mask's coverage in the document grid,
        // not the layer's masked pixels — and a disabled mask blocks the tool
        // (BlurTool.swift:11-28; beginBrush's guard).
        let mut document = CanvasDocument::new(40, 40);
        let mut layer = opaque_layer(40, 40, [255, 0, 0, 255], CGPoint::ZERO);
        let mut mask_image = Bitmap::new(40, 40);
        for px in mask_image.data.chunks_exact_mut(4) {
            px.copy_from_slice(&[200, 200, 200, 255]);
        }
        layer.mask = Some(LayerMask {
            asset: ImportedImage::new(mask_image, "mask"),
            is_enabled: true,
            placement: None,
            is_linked: true,
        });
        let sample = blur_mask_sample(&document, &layer).unwrap();
        assert_eq!(
            (sample.width, sample.height),
            (document.width, document.height)
        );
        // The coverage's gray, drawn over the background tone.
        assert_eq!(sample.pixel(10, 10), [200, 200, 200, 255]);
        // A disabled mask samples nothing, like Swift's `guard let owned` on
        // an enabled-only mask path.
        layer.mask = Some(LayerMask {
            is_enabled: false,
            ..layer.mask.take().unwrap()
        });
        assert!(blur_mask_sample(&document, &layer).is_none());
    }

    #[test]
    fn blur_sample_softened_blurs_the_sample_by_the_sigma() {
        // A white square softened into its neighborhood; the exact spread
        // follows the diameter's sigma (a bigger diameter blurs further, so
        // the square's interior dimmed more).
        let mut sample = Bitmap::new(41, 41);
        for y in 15..26 {
            for x in 15..26 {
                sample.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        let small = blur_sample_softened(&sample, 30.0);
        let large = blur_sample_softened(&sample, 300.0);
        // The center dimmed: the blur pulled the surrounding emptiness in.
        assert!(small.pixel(20, 20)[0] < 255);
        assert!(large.pixel(20, 20)[0] < small.pixel(20, 20)[0]);
    }

    #[test]
    fn clone_layer_sample_does_not_darken_under_a_mask() {
        // cloneSample's active-layer half: the layer's pixels as displayed,
        // NOT multiplied by the mask coverage (CloneStamp.swift:39-48 — a
        // masked-over area still clones at full strength).
        let mut image = Bitmap::new(20, 20);
        for px in image.data.chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }
        let transform = LayerTransform::new(CGPoint::new(10.0, 10.0), CGSize::new(20.0, 20.0));
        let sample = clone_layer_sample(&image, &transform, 40, 40);
        // Inside the layer: full-strength pixels; an earlier version of this
        // port multiplied the sample by an enabled mask and dimmed them.
        assert_eq!(sample.pixel(15, 15), [255, 0, 0, 255]);
        // Outside the layer the sample is empty.
        assert_eq!(sample.pixel(5, 5), [0, 0, 0, 0]);
    }

    #[test]
    fn shape_drag_rect_uses_the_requested_drag_mode() {
        // Keep a direct assertion on the rect helper the canvas drags use.
        let start = CGPoint::ZERO;
        let rect = shape_drag_rect(
            ShapeKind::Rectangle,
            start,
            CGPoint::new(30.0, 20.0),
            false,
            false,
        );
        assert_eq!(rect, CGRect::new(0.0, 0.0, 30.0, 20.0));
    }

    #[test]
    fn transform_press_auto_selects_the_layer_above() {
        let mut doc = CanvasDocument::new(100, 100);
        let mut background = Bitmap::new(100, 100);
        for px in background.data.chunks_exact_mut(4) {
            px.copy_from_slice(&[240, 240, 240, 255]);
        }
        let mut foreground = Bitmap::new(40, 40);
        for px in foreground.data.chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }
        let background =
            ImageLayer::with_asset(ImportedImage::new(background, "bg"), CGPoint::ZERO);
        let foreground = ImageLayer::with_asset(
            ImportedImage::new(foreground, "fg"),
            CGPoint::new(30.0, 30.0),
        );
        let (bg, fg) = (background.id, foreground.id);
        doc.layers = vec![background, foreground];
        // Auto-select picks the foreground stacked above the background.
        assert_eq!(
            transform_press_layer(&doc, Some(bg), true, false, true, CGPoint::new(50.0, 50.0)),
            Some((fg, true))
        );
        // Without auto-select the press drags the active layer.
        assert_eq!(
            transform_press_layer(&doc, Some(bg), false, false, true, CGPoint::new(50.0, 50.0)),
            Some((bg, false))
        );
        // Outside the foreground the press still drags the active layer.
        assert_eq!(
            transform_press_layer(&doc, Some(bg), true, false, true, CGPoint::new(10.0, 10.0)),
            Some((bg, false))
        );
        // Cmd picks the layer under the pointer on its own.
        assert_eq!(
            transform_press_layer(&doc, Some(bg), false, true, true, CGPoint::new(50.0, 50.0)),
            Some((fg, true))
        );
        // A press outside the active layer falls back to dragging it.
        assert_eq!(
            transform_press_layer(&doc, Some(fg), false, false, true, CGPoint::new(5.0, 5.0)),
            Some((fg, false))
        );
    }

    #[test]
    fn nice_tick_rounds_up_to_a_ruler_step() {
        assert_eq!(nice_tick(0.4), 1.0);
        assert_eq!(nice_tick(3.0), 5.0);
        assert_eq!(nice_tick(7.5), 10.0);
        assert_eq!(nice_tick(15.0), 20.0);
        assert_eq!(nice_tick(120.0), 200.0);
        // Beyond the last candidate it saturates.
        assert_eq!(nice_tick(99_999.0), 50_000.0);
    }

    #[test]
    fn ruler_labels_are_whole_pixels() {
        assert_eq!(ruler_label(0.0), "0");
        assert_eq!(ruler_label(99.6), "100");
        assert_eq!(ruler_label(-2.5), "-3");
    }

    #[test]
    fn ruler_axis_at_splits_the_two_strips_and_spares_the_corner() {
        // The top strip is the horizontal ruler, the left strip the vertical
        // one, and the corner square between them starts nothing.
        assert_eq!(ruler_axis_at((80.0, 10.0)), Some(GuideAxis::Horizontal));
        assert_eq!(ruler_axis_at((10.0, 80.0)), Some(GuideAxis::Vertical));
        assert_eq!(ruler_axis_at((10.0, 10.0)), None, "the corner is inert");
        assert_eq!(
            ruler_axis_at((80.0, 80.0)),
            None,
            "the canvas body is no ruler"
        );
        // The strip edges themselves belong to the rulers.
        assert_eq!(
            ruler_axis_at((RULER_SIZE + 1.0, RULER_SIZE - 1.0)),
            Some(GuideAxis::Horizontal)
        );
    }

    #[test]
    fn guide_axis_position_reads_its_own_coordinate() {
        let doc = CGPoint::new(33.0, 47.0);
        // A vertical guide sits at a document x, a horizontal one at a y.
        assert_eq!(guide_axis_position(GuideAxis::Vertical, doc), 33.0);
        assert_eq!(guide_axis_position(GuideAxis::Horizontal, doc), 47.0);
    }

    #[test]
    fn guide_position_converts_through_the_view_transform() {
        // The ruler reports the document coordinate under the cursor
        // (CanvasRulerNSView.documentPosition), so a cursor at 120 viewport
        // pixels with the document's origin panned 20 px at 2× sits at
        // document x = 50.
        let doc = crate::state::screen_to_doc_at(2.0, (20.0, 10.0), 120.0, 60.0);
        assert_eq!(guide_axis_position(GuideAxis::Vertical, doc), 50.0);
        assert_eq!(guide_axis_position(GuideAxis::Horizontal, doc), 25.0);
    }

    #[test]
    fn guide_release_deletes_only_on_a_shown_ruler() {
        // Released over the top or left strip: cancel a new guide, delete an
        // existing one (isOverRuler).
        assert!(guide_release_deletes(true, (10.0, 80.0)));
        assert!(guide_release_deletes(true, (80.0, 10.0)));
        // Released on the canvas itself: the guide lands (or just moves).
        assert!(!guide_release_deletes(true, (80.0, 80.0)));
        // With the rulers hidden there is no strip to release on.
        assert!(!guide_release_deletes(false, (10.0, 10.0)));
    }

    #[test]
    fn handle_at_hits_corners_and_edges_but_not_center() {
        let rect = CGRect::new(0.0, 0.0, 100.0, 60.0);
        // Top-left corner handle (index 0) ...
        assert_eq!(handle_at(rect, CGPoint::new(2.0, -3.0)), Some(0));
        // ... and the right-middle edge handle (index 3).
        assert_eq!(handle_at(rect, CGPoint::new(103.0, 30.0)), Some(3));
        // Well inside the rect there is no handle.
        assert_eq!(handle_at(rect, CGPoint::new(50.0, 30.0)), None);
        // Far outside any handle either.
        assert_eq!(handle_at(rect, CGPoint::new(60.0, 90.0)), None);
    }

    #[test]
    fn handle_at_walks_the_eight_handles_clockwise() {
        let rect = CGRect::new(0.0, 0.0, 100.0, 60.0);
        // Corners, then edge midpoints, clockwise from the top-left.
        let expected = [
            (0.0, 0.0, 0),
            (50.0, 0.0, 1),
            (100.0, 0.0, 2),
            (100.0, 30.0, 3),
            (100.0, 60.0, 4),
            (50.0, 60.0, 5),
            (0.0, 60.0, 6),
            (0.0, 30.0, 7),
        ];
        for (x, y, index) in expected {
            assert_eq!(
                handle_at(rect, CGPoint::new(x, y)),
                Some(index),
                "at {x},{y}"
            );
        }
    }

    #[test]
    fn shape_drag_rect_squares_and_centers() {
        let anchor = CGPoint::new(10.0, 10.0);
        // No modifiers: the plain anchor→point box.
        let rect = shape_drag_rect(
            ShapeKind::Rectangle,
            anchor,
            CGPoint::new(23.0, 27.0),
            false,
            false,
        );
        assert_eq!(rect, CGRect::new(10.0, 10.0, 13.0, 17.0));
        // Shift squares it to the longer side, keeping the drag direction.
        let rect = shape_drag_rect(
            ShapeKind::Rectangle,
            anchor,
            CGPoint::new(23.0, 27.0),
            true,
            false,
        );
        assert_eq!(rect, CGRect::new(10.0, 10.0, 17.0, 17.0));
        // Option grows a symmetric shape around the anchor.
        let rect = shape_drag_rect(
            ShapeKind::Rectangle,
            anchor,
            CGPoint::new(23.0, 27.0),
            false,
            true,
        );
        assert_eq!(rect, CGRect::new(-3.0, -7.0, 26.0, 34.0));
        // Backwards drags normalize to positive sizes.
        let rect = shape_drag_rect(
            ShapeKind::Ellipse,
            anchor,
            CGPoint::new(-5.0, 4.0),
            false,
            false,
        );
        assert_eq!(rect, CGRect::new(-5.0, 4.0, 15.0, 6.0));
    }

    #[test]
    fn shape_drag_line_snaps_to_eighths_and_ignores_modifiers() {
        let anchor = CGPoint::new(0.0, 0.0);
        // Shift snaps a shallow drag to exactly horizontal.
        let rect = shape_drag_rect(
            ShapeKind::Line,
            anchor,
            CGPoint::new(100.0, 7.0),
            true,
            false,
        );
        assert_eq!(rect, CGRect::new(0.0, 0.0, 100.0, 0.0));
        // A 3-4-5-ish drag snaps to the 45° diagonal, keeping its length
        // (drag_box_rect rounds the snapped end to whole pixels).
        let rect = shape_drag_rect(
            ShapeKind::Line,
            anchor,
            CGPoint::new(10.0, 8.0),
            true,
            false,
        );
        let snapped = (164.0f64.sqrt() * std::f64::consts::FRAC_1_SQRT_2).round();
        assert_eq!(rect.size.width, snapped);
        assert_eq!(rect.size.height, snapped);
        // Without shift, no snap.
        let rect = shape_drag_rect(
            ShapeKind::Line,
            anchor,
            CGPoint::new(10.0, 8.0),
            false,
            false,
        );
        assert_eq!(rect, CGRect::new(0.0, 0.0, 10.0, 8.0));
        // A line ignores Option centering: it stays the plain box.
        let rect = shape_drag_rect(
            ShapeKind::Line,
            anchor,
            CGPoint::new(10.0, 8.0),
            false,
            true,
        );
        assert_eq!(rect, CGRect::new(0.0, 0.0, 10.0, 8.0));
    }
}

// MARK: Drag events

/// Registers the window-level listeners that continue an in-progress drag
/// when the pointer leaves the canvas. Called during paint; listeners are
/// re-registered every frame.
fn install_drag_events(
    app_move: &gpui_kit::WeakEntity<AppState>,
    app_up: &gpui_kit::WeakEntity<AppState>,
    window: &mut Window,
) {
    let app_move = app_move.clone();
    let app_up = app_up.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, _phase, window, cx| {
        app_move
            .update(cx, |state, cx| {
                if state.drag.is_some() {
                    state.canvas_mouse_move(event, window, cx);
                }
            })
            .ok();
    });
    window.on_mouse_event(move |event: &MouseUpEvent, _phase, window, cx| {
        app_up
            .update(cx, |state, cx| state.canvas_mouse_up(event, window, cx))
            .ok();
    });
}

/// Registers the window-level key listeners for the canvas keys the action
/// system can't express: Space's hold-to-pan (needs keyUp), the opacity
/// digits, Option-arrow text spacing and ⌘Return finishing text
/// (`EditorCanvas.keyDown`/`keyUp`). Listeners run each frame, so they must
/// be re-registered during paint like the drag events above.
fn install_key_events(app: &gpui_kit::WeakEntity<AppState>, window: &mut Window) {
    let app_down = app.clone();
    window.on_key_event(move |event: &KeyDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        app_down
            .update(cx, |state, cx| state.canvas_key_down(event, window, cx))
            .ok();
    });
    let app_up = app.clone();
    window.on_key_event(move |event: &KeyUpEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        app_up
            .update(cx, |state, cx| state.canvas_key_up(event, window, cx))
            .ok();
    });
}
