//! The canvas viewport: paints the composite, checkerboard, rulers, guides,
//! grid, selection outline, lasso draft, crop box and transform box; owns all
//! pointer interaction with the document.

use std::sync::Arc;

use gpui_kit::{
    canvas, div, px, point, rgb, rgba, size, Bounds, Context, Corners, DispatchPhase,
    IntoElement, InteractiveElement, KeyDownEvent, KeyUpEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder, Pixels, Point, RenderImage,
    ScrollWheelEvent, ScrollDelta, Window,
};
use gpui_kit::Styled;

use compositor_core::canvas::CropDragMode;
use compositor_core::document::LayoutGrid;
use compositor_core::document::{GuideAxis, ImageLayer, NavigationTool};
use compositor_core::geometry::{CGPoint, CGRect, CGSize};
use compositor_core::raster::Bitmap;
use compositor_core::selection::LassoKind;
use compositor_core::session::{BlurToolMode, BrushToolMode, SpotHealingMode};
use compositor_core::tools::warp::WarpStroke;
use compositor_core::tools::brush::{GradientShape, RasterEdit};

use crate::state::{AppState, CanvasDrag, RULER_SIZE};

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
}

impl AppState {
    pub fn render_canvas(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let composite = self.composite();
        let image = composite.as_ref().map(|composite| self.frame_image(composite));
        let scene = self.build_scene(image);
        let app_move = cx.entity().downgrade();
        let app_up = cx.entity().downgrade();
        let app_prepaint = cx.entity().downgrade();

        div()
            .id("canvas-area")
            .relative()
            .flex_1()
            .overflow_hidden()
            .bg(rgb(0x232326))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, window, cx| {
                this.canvas_mouse_down(event, window, cx);
            }))
            .on_scroll_wheel(cx.listener(
                move |this, event: &ScrollWheelEvent, window, cx| {
                    this.canvas_scroll(event, window, cx);
                },
            ))
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
                                    (w as usize, h as usize)
                                };
                                if dims != state.fit_for && dims.0 > 0 && vw > 10.0 && vh > 10.0 {
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

        if let Some(document) = self.session.document.as_ref() {
            for guide in &document.guides {
                let vertical = guide.axis == GuideAxis::Vertical;
                let position = if vertical {
                    self.doc_to_screen(guide.position, 0.0).0
                } else {
                    self.doc_to_screen(0.0, guide.position).1
                };
                guides.push((vertical, position));
            }
            if self.session.shows_grid {
                for x in LayoutGrid::lines(document.width as f64) {
                    grid.push((self.doc_to_screen(x, 0.0).0, LayoutGrid::is_major(x), true));
                }
                for y in LayoutGrid::lines(document.height as f64) {
                    grid.push((self.doc_to_screen(0.0, y).1, LayoutGrid::is_major(y), false));
                }
            }
            if let Some(selection_state) = &document.selection {
                for ring in &selection_state.path.rings {
                    selection.push(
                        ring.iter()
                            .map(|p| self.doc_to_screen(p.x, p.y))
                            .collect(),
                    );
                }
            }
            if let Some(draft) = &self.session.lasso_draft {
                let mut rings = vec![draft
                    .points
                    .iter()
                    .map(|p| self.doc_to_screen(p.x, p.y))
                    .collect::<Vec<(f32, f32)>>()];
                if let Some(cursor) = draft.cursor {
                    if !rings[0].is_empty() {
                        let (x, y) = self.doc_to_screen(cursor.x, cursor.y);
                        rings[0].push((x, y));
                    }
                }
                lasso = Some(rings);
            }
            if let Some(rect) = self.session.crop_rect {
                let min = self.doc_to_screen(rect.min_x(), rect.min_y());
                let max = self.doc_to_screen(rect.max_x(), rect.max_y());
                crop = Some((min, max));
            }
            if let Some(edit) = &self.session.transform_edit {
                // TransformOverlay.swift: handles hide with ⌘H unless an edit must stay visible.
                if edit.draft.is_valid()
                    && (self.session.shows_transform_controls || edit.persistent)
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
        }

        CanvasScene {
            image,
            image_rect,
            rulers: self.session.shows_rulers,
            zoom: self.zoom,
            pan: self.pan,
            doc: self.doc_size(),
            guides,
            grid,
            selection,
            lasso,
            crop,
            transform,
        }
    }

    // MARK: Pointer handling

    pub fn canvas_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
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
            if self.session.tool == NavigationTool::Lasso
                && self.session.lasso_kind == LassoKind::Polygonal
                && self.session.lasso_draft.is_some()
            {
                self.session.finish_lasso();
                self.touch(cx);
            }
            return;
        }
        let local = self.local_position(event.position);
        let doc = self.screen_to_doc(local.0, local.1);
        let alt = event.modifiers.alt;
        let shift = event.modifiers.shift;

        match self.session.tool {
            NavigationTool::Hand => {
                self.drag = Some(CanvasDrag::Pan {
                    start: event.position,
                    start_pan: self.pan,
                });
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
                let polygonal = self.session.tool == NavigationTool::Lasso
                    && self.session.lasso_kind == LassoKind::Polygonal;
                if polygonal && self.session.lasso_draft.is_some() {
                    self.session.extend_lasso(doc);
                    self.touch(cx);
                    return;
                }
                let mode = self.session.selection_mode(shift, alt);
                self.session.begin_lasso(doc, mode);
                self.drag = Some(CanvasDrag::MarqueeLasso);
            }
            NavigationTool::Wand => {
                let mode = self.session.selection_mode(shift, alt);
                self.session.magic_wand(doc, mode);
            }
            NavigationTool::Brush
            | NavigationTool::SpotHealing
            | NavigationTool::CloneStamp
            | NavigationTool::Blur => {
                if self.session.tool == NavigationTool::Blur {
                    match self.session.blur_mode {
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
                self.begin_move(doc, cx);
            }
            NavigationTool::Crop => {
                self.begin_crop(doc, cx);
            }
            NavigationTool::Eyedropper => {
                if let Some(document) = self.session.document.as_ref() {
                    if let Some(color) =
                        compositor_core::render::sample_composite_color(document, doc)
                    {
                        self.session.brush_settings.red = color.red;
                        self.session.brush_settings.green = color.green;
                        self.session.brush_settings.blue = color.blue;
                        self.set_status(format!("Foreground #{}", color.hex()), cx);
                    }
                }
            }
            NavigationTool::Type => {
                self.create_text_layer(doc, cx);
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
        _window: &mut Window,
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
            Some(CanvasDrag::Stroke(edit)) => {
                edit.append(doc);
            }
            Some(CanvasDrag::Warp(stroke)) => {
                stroke.append(doc);
            }
            Some(CanvasDrag::Shape { start }) => {
                let rect = shape_drag_rect(self.session.shape_kind, *start, doc, shift, alt);
                let shape = self.build_shape_for_drag(rect);
                self.shape_rect = Some(rect);
                self.shape_shape = Some(shape);
            }
            Some(CanvasDrag::Gradient(_, _)) => {
                self.gradient_cursor = Some(doc);
            }
            Some(CanvasDrag::MarqueeLasso) => {
                let kind = self.session.lasso_draft.as_ref().map(|d| d.kind);
                if matches!(kind, Some(LassoKind::Rectangle) | Some(LassoKind::Ellipse)) {
                    self.session.drag_marquee(doc, shift, false);
                } else {
                    self.session.extend_lasso(doc);
                }
            }
            Some(CanvasDrag::MoveLayer { start_origin, start }) => {
                let delta = CGPoint::new(doc.x - start.x, doc.y - start.y);
                let mut value = *start_origin;
                value.origin.x += delta.x;
                value.origin.y += delta.y;
                // Moving snaps to the canvas and the other layers; resizing
                // and rotating are left alone (EditorCanvas.dragTransform).
                let moving: Vec<uuid::Uuid> = self
                    .session
                    .transform_edit
                    .as_ref()
                    .map(|edit| match &edit.group {
                        Some(group) => group.originals.iter().map(|(id, _)| *id).collect(),
                        None => vec![edit.layer_id],
                    })
                    .unwrap_or_default();
                let value = self.session.snapped_move(value, &moving, 10.0);
                self.session.preview_transform(value);
            }
            Some(CanvasDrag::Crop) => {
                let ratio = self.session.crop_ratio();
                self.session.drag_crop(doc, ratio, false);
            }
            None => return,
        }
        self.touch(cx);
    }

    pub fn canvas_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(drag) = self.drag.take() {
            match drag {
                CanvasDrag::Stroke(mut edit) => {
                    edit.flush();
                    self.commit_stroke(edit, cx);
                }
                CanvasDrag::Gradient(edit, start) => {
                    let local = self.local_position(event.position);
                    self.gradient_cursor = Some(self.screen_to_doc(local.0, local.1));
                    let result = self.gradient_result(&edit, start);
                    self.commit_bitmap(result, edit.paint_transform, "Gradient", cx);
                }
                CanvasDrag::Warp(stroke) => {
                    self.finish_warp(stroke, cx);
                }
                CanvasDrag::Shape { .. } => {
                    self.finish_shape(cx);
                }
                CanvasDrag::MarqueeLasso => {
                    self.session.finish_lasso();
                }
                CanvasDrag::Crop => {
                    self.session.end_crop_drag();
                }
                CanvasDrag::MoveLayer { .. } => {
                    self.session.commit_transform();
                }
                CanvasDrag::Pan { .. } => {}
            }
            self.gradient_cursor = None;
            self.touch(cx);
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
            ScrollDelta::Lines(pt) => f32::from(pt.y) * 40.0,
        };
        if dy != 0.0 {
            let factor = (-dy / 400.0).exp();
            self.zoom_at(local, factor as f64);
            self.touch(cx);
        }
    }

    fn local_position(&self, position: Point<Pixels>) -> (f32, f32) {
        (
            f32::from(position.x) - f32::from(self.viewport.origin.x),
            f32::from(position.y) - f32::from(self.viewport.origin.y),
        )
    }

    // MARK: Strokes

    fn begin_stroke(&mut self, doc: CGPoint, alt: bool, cx: &mut Context<Self>) {
        let Some(document) = self.session.document.clone() else {
            return;
        };
        let Some(id) = self.session.active_layer_id else {
            return;
        };
        let Some(layer) = document.layer(id) else {
            return;
        };
        if layer.is_group {
            return;
        }
        // Clone Stamp: option-click sets the source; strokes copy from it.
        if self.session.tool == NavigationTool::CloneStamp {
            if alt {
                self.session.clone_source = Some(doc);
                self.set_status(format!("Clone source set at {:.0}, {:.0}", doc.x, doc.y), cx);
                return;
            }
            if self.session.clone_source.is_none() {
                self.set_status("Option-click to set a clone source first", cx);
                return;
            }
        }
        let mut settings = self.session.brush_settings;
        settings.erasing = self.session.brush_mode == BrushToolMode::Erase;
        settings.healing = self.session.tool == NavigationTool::SpotHealing;
        let source = layer.asset.as_ref().map(|a| a.image.as_ref().clone());
        let clip = document.selection.as_ref().map(|s| s.clip(document.size()));
        let mut edit = RasterEdit::new(
            source,
            layer.transform,
            false,
            settings,
            CGSize::new(document.width as f64, document.height as f64),
            clip,
        );
        if let (Some(source_point), Some(sample)) =
            (self.session.clone_source, self.sample_for_clone())
        {
            if self.session.tool == NavigationTool::CloneStamp {
                let offset = CGSize::new(source_point.x - doc.x, source_point.y - doc.y);
                edit.clone = Some((sample, offset));
            }
        }
        // The Blur brush: the canvas as shown (displayed transform, mask
        // applied), softened by an amount that follows the brush size,
        // painted in place through the tip (BlurTool.swift:8-40).
        if self.session.tool == NavigationTool::Blur {
            if let Some(sample) = self.sample_for_clone() {
                let diameter = self.session.brush_settings.diameter;
                let sigma = (diameter / 10.0).clamp(1.5, 30.0);
                let blurred = compositor_core::raster::gaussian_blur_rgba(&sample, sigma);
                edit.clone = Some((blurred, CGSize::ZERO));
            }
        }
        edit.append(doc);
        self.drag = Some(CanvasDrag::Stroke(edit));
    }

    /// What the Clone Stamp samples: the composite or the active layer.
    fn sample_for_clone(&self) -> Option<Bitmap> {
        let document = self.session.document.as_ref()?;
        if self.session.clone_settings.sample_all_layers {
            return Some(compositor_core::render::draw_live_composite(document));
        }
        // The active layer as the canvas displays it: drawn through its
        // displayed transform at document size (CloneStamp.swift:39-48).
        let layer = self.session.active_layer()?;
        let image = layer.asset.as_ref()?.image.as_ref().clone();
        let transform = self.session.displayed_transform(layer);
        let mut canvas = Bitmap::new(document.width, document.height);
        compositor_core::render::draw_layer_pixels(&image, &transform, &mut canvas);
        // An enabled mask shows through: the sampled content is multiplied by
        // the mask coverage (the canvas displays the masked result).
        if let Some(mask) = layer.mask.as_ref().filter(|m| m.is_enabled) {
            if let Some(coverage) = compositor_core::render::mask_clip_image(
                mask,
                &layer.mask_transform(),
                document.width,
                document.height,
            ) {
                for y in 0..canvas.height {
                    for x in 0..canvas.width {
                        let o = (y * canvas.width + x) * 4;
                        let m = coverage.pixel(x, y)[0] as f32 / 255.0;
                        for c in 0..4 {
                            canvas.data[o + c] = (canvas.data[o + c] as f32 * m) as u8;
                        }
                    }
                }
            }
        }
        Some(canvas)
    }

    fn commit_stroke(&mut self, mut edit: RasterEdit, cx: &mut Context<Self>) {
        if !edit.has_paint() {
            return;
        }
        // Spot Healing: the wash was only ever a preview; the stroke ends by
        // rebuilding the painted area from nearby texture (HealPixels.c).
        if edit.settings.healing {
            let mode = match self.session.spot_healing_mode {
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
            self.commit_pixels(snapshot, transform, "Spot Healing", cx);
            return;
        }
        let erasing = edit.settings.erasing;
        let (snapshot, transform, _) = edit.paint_snapshot();
        // Stroke names follow the tool (EditorSession+Brush.swift:141).
        let name = if edit.is_mask {
            "Paint Mask"
        } else if erasing {
            "Erase"
        } else if self.session.tool == NavigationTool::Blur {
            "Blur"
        } else if self.session.tool == NavigationTool::CloneStamp {
            "Clone Stamp"
        } else {
            "Brush Stroke"
        };
        self.commit_pixels(snapshot, transform, name, cx);
    }

    // MARK: Smudge / Liquify

    /// `beginWarp`: the stroke works on the active layer as the canvas shows
    /// it, at document size.
    fn begin_warp(&mut self, doc: CGPoint) {
        let Some(document) = self.session.document.clone() else {
            return;
        };
        let Some(id) = self.session.active_layer_id else {
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
        let mut stroke =
            WarpStroke::new(surface, self.session.blur_mode, &self.session.brush_settings);
        stroke.layer_asset = layer.asset.as_ref().map(|a| a.image.clone());
        stroke.layer_transform = layer.transform;
        stroke.append(doc);
        self.drag = Some(CanvasDrag::Warp(stroke));
    }

    /// `finishWarp`: paints the finished result into the layer's own pixels
    /// along the stroke, as one undo step.
    fn finish_warp(&mut self, stroke: WarpStroke, cx: &mut Context<Self>) {
        let name = match stroke.mode {
            BlurToolMode::Smudge => "Smudge",
            BlurToolMode::Liquify => "Liquify",
            BlurToolMode::Blur => "Blur",
        };
        let Some(document) = self.session.document.clone() else {
            return;
        };
        let Some(id) = self.session.active_layer_id else {
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
        let selection_clip = document
            .selection
            .as_ref()
            .map(|s| s.clip(document.size()));
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
            self.commit_pixels(snapshot, transform, name, cx);
        }
    }

    // MARK: Shape

    /// The shape the current drag places, from the tool's settings.
    fn build_shape_for_drag(&self, rect: CGRect) -> compositor_core::document::LayerShape {
        let kind = self.session.shape_kind;
        let color = self.session.palette_color(false);
        compositor_core::document::LayerShape {
            kind,
            corner_radius: self.session.shape_corner_radius,
            line_width: self.session.shape_line_width,
            red: color.red,
            green: color.green,
            blue: color.blue,
            filled: kind != compositor_core::document::ShapeKind::Line,
            start: CGPoint::ZERO,
            end: CGPoint::new(1.0, 1.0),
        }
    }

    fn begin_shape(&mut self, doc: CGPoint) {
        self.drag = Some(CanvasDrag::Shape { start: doc });
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
        let kind = self.session.shape_kind;
        let color = self.session.palette_color(false);
        let shape = compositor_core::document::LayerShape {
            kind,
            corner_radius: self.session.shape_corner_radius,
            line_width: self.session.shape_line_width,
            red: color.red,
            green: color.green,
            blue: color.blue,
            filled: kind != compositor_core::document::ShapeKind::Line,
            start: CGPoint::ZERO,
            end: CGPoint::new(1.0, 1.0),
        };
        let name = self.session.next_shape_name(kind);
        self.session.add_shape_layer(shape, rect, name);
        self.touch(cx);
    }

    /// Stores new pixels into the active layer as one undoable step.
    pub(crate) fn commit_pixels(
        &mut self,
        image: compositor_core::raster::ImportedImage,
        transform: compositor_core::transform::LayerTransform,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.session.active_layer_id else {
            return;
        };
        self.session
            .history
            .begin(name, &self.session.document, self.session.active_layer_id);
        if let Some(doc) = self.session.document.as_mut() {
            if let Some(layer) = doc.layer_mut(id) {
                let mut image = image;
                image.name = layer.name.clone();
                layer.asset = Some(image);
                layer.transform = transform;
            }
        }
        self.session
            .history
            .end(&self.session.document, self.session.active_layer_id);
        self.touch(cx);
    }

    /// Wraps a plain bitmap before committing it.
    pub(crate) fn commit_bitmap(
        &mut self,
        bitmap: Bitmap,
        transform: compositor_core::transform::LayerTransform,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        self.commit_pixels(
            compositor_core::raster::ImportedImage::new(bitmap, String::new()),
            transform,
            name,
            cx,
        );
    }

    // MARK: Gradient

    fn begin_gradient(&mut self, doc: CGPoint) {
        let Some(document) = self.session.document.clone() else {
            return;
        };
        let Some(id) = self.session.active_layer_id else {
            return;
        };
        let Some(layer) = document.layer(id) else {
            return;
        };
        if layer.is_group {
            return;
        }
        let source = layer.asset.as_ref().map(|a| a.image.as_ref().clone());
        let clip = document.selection.as_ref().map(|s| s.clip(document.size()));
        let edit = RasterEdit::new(
            source,
            layer.transform,
            false,
            self.session.brush_settings,
            CGSize::new(document.width as f64, document.height as f64),
            clip,
        );
        self.gradient_cursor = Some(doc);
        self.drag = Some(CanvasDrag::Gradient(edit, doc));
    }

    // MARK: Move

    fn begin_move(&mut self, doc: CGPoint, cx: &mut Context<Self>) {
        if self.session.transform_edit.is_none() {
            if self.session.can_transform() {
                self.session.begin_transform();
            } else {
                return;
            }
        }
        let Some(edit) = self.session.transform_edit.as_ref() else {
            return;
        };
        self.drag = Some(CanvasDrag::MoveLayer {
            start_origin: edit.draft,
            start: doc,
        });
        self.touch(cx);
    }

    // MARK: Crop

    fn begin_crop(&mut self, doc: CGPoint, cx: &mut Context<Self>) {
        let mode = if let Some(rect) = self.session.crop_rect {
            match handle_at(rect, doc) {
                Some(index) => CropDragMode::Resize(index),
                None if rect.contains_point(doc) => CropDragMode::Move,
                None => CropDragMode::Create,
            }
        } else {
            CropDragMode::Create
        };
        self.session.begin_crop_drag(doc, mode);
        self.drag = Some(CanvasDrag::Crop);
        self.touch(cx);
    }

    // MARK: Type

    fn create_text_layer(&mut self, doc: CGPoint, cx: &mut Context<Self>) {
        // The session's beginText/applyText flow, with real-font typesetting
        // (TypeTool.swift). Without a text input UI a click lays down the
        // default sample content, editable through the same live-text path.
        if self.session.document.is_none() {
            return;
        }
        self.session.begin_text(doc, true);
        let mut draft = match self.session.text_draft.clone() {
            Some(draft) => draft,
            None => return,
        };
        if draft.style.content.is_empty() {
            draft.style.content = self.session.text_defaults.content.clone();
        }
        self.session.apply_text(draft);
        self.set_status(format!("Text layer at {:.0}, {:.0}", doc.x, doc.y), cx);
    }

    // MARK: Keys

    /// Tab cycles the active tool's mode.
    pub fn cycle_tool_mode(&mut self, cx: &mut Context<Self>) {
        if self.session.tool == NavigationTool::Shape {
            self.session.toggle_shape_kind();
            self.touch(cx);
            return;
        }
        match self.session.tool {
            NavigationTool::Marquee => self.session.toggle_marquee_kind(),
            NavigationTool::Lasso => self.session.toggle_lasso_kind(),
            NavigationTool::Wand => {
                self.session.wand_mode = match self.session.wand_mode {
                    compositor_core::selection::WandMode::Wand => {
                        compositor_core::selection::WandMode::Object
                    }
                    compositor_core::selection::WandMode::Object => {
                        compositor_core::selection::WandMode::Wand
                    }
                };
            }
            NavigationTool::Brush => {
                self.session.brush_mode = match self.session.brush_mode {
                    BrushToolMode::Paint => BrushToolMode::Erase,
                    BrushToolMode::Erase => BrushToolMode::Paint,
                };
            }
            NavigationTool::Gradient => {
                self.session.gradient_settings.shape = match self.session.gradient_settings.shape {
                    GradientShape::Linear => GradientShape::Radial,
                    GradientShape::Radial => GradientShape::Linear,
                };
            }
            _ => {}
        }
        self.touch(cx);
    }

    /// Escape: cancel the active edit, or close the sheet.
    pub fn cancel_active(&mut self, cx: &mut Context<Self>) {
        if self.session.color_picker.is_some() {
            self.close_color_picker_ui(false, cx);
            return;
        }
        if self.sheet.is_some() {
            // Route through cancel_sheet so sheet-specific cancel semantics
            // run (the Effects sheet restores the edited effect).
            self.cancel_sheet(cx);
            return;
        }
        if self.session.lasso_draft.take().is_some() {
            self.touch(cx);
            return;
        }
        if self.session.crop_drag.take().is_some() {
            self.session.crop_rect = None;
            self.touch(cx);
            return;
        }
        if self.session.transform_edit.take().is_some() {
            self.touch(cx);
            return;
        }
        if self.session.crop_rect.take().is_some() {
            self.touch(cx);
            return;
        }
        if self.rename.take().is_some() {
            self.rename_input = None;
            self.touch(cx);
        }
    }
}

/// The drag rect for the shape tool (`ShapeTool.dragShape`): Shift squares
/// non-line shapes and snaps a line's angle to eighths of a turn, Option
/// grows a non-line shape symmetrically from its anchor. A line ignores both
/// beyond the snap — it is stored as its bounding box.
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
        let angle = ((dy.atan2(dx)) / std::f64::consts::FRAC_PI_4).round()
            * std::f64::consts::FRAC_PI_4;
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
fn handle_at(rect: CGRect, doc: CGPoint) -> Option<usize> {    const HANDLE: f64 = 8.0;
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
    let scene = scene.translated(
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
    );

    let image_bounds = scene.image_rect;

    // Checkerboard behind the document.
    if f32::from(image_bounds.size.width) > 0.0 {
        paint_checkerboard(window, bounds, image_bounds);
    }

    // The composite itself.
    if let Some(image) = &scene.image {
        let _ = window.paint_image(bounds, image_bounds, Corners::default(), image.clone(), 0, false);
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

    // Selection outline: dark under-stroke + white dashes ("marching ants").
    for ring in &scene.selection {
        if ring.len() >= 3 {
            paint_polygon(window, ring, rgba(0x000000aa), false);
            paint_polygon(window, ring, rgba(0xffffffff), true);
        }
    }

    // Lasso draft.
    if let Some(rings) = &scene.lasso {
        for ring in rings {
            if ring.len() >= 2 {
                paint_polygon(window, ring, rgba(0x000000aa), false);
                paint_polygon(window, ring, rgba(0xffffffff), true);
            }
        }
    }

    // Transform outline with corner handles.
    if let Some(corners) = &scene.transform {
        if corners.len() >= 3 {
            paint_polygon(window, corners, rgba(0x4da6ffdd), false);
            for corner in corners {
                paint_handle(window, point(px(corner.0), px(corner.1)));
            }
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
            paint_line(window, bounds, f32::from(rect.origin.x) + w * fraction, true, rgba(0xffffff44));
            paint_line(window, bounds, f32::from(rect.origin.y) + h * fraction, false, rgba(0xffffff44));
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
        r.origin = point(px(f32::from(r.origin.x) + ox), px(f32::from(r.origin.y) + oy));
        out.guides = out
            .guides
            .iter()
            .map(|(vertical, position)| {
                (
                    *vertical,
                    if *vertical { position + ox } else { position + oy },
                )
            })
            .collect();
        out.grid = out
            .grid
            .iter()
            .map(|(position, major, vertical)| {
                (
                    if *vertical { position + ox } else { position + oy },
                    *major,
                    *vertical,
                )
            })
            .collect();
        out.selection = shift_rings(&self.selection);
        out.lasso = self.lasso.as_ref().map(|rings| shift_rings(rings));
        out.crop = self
            .crop
            .map(|((x0, y0), (x1, y1))| ((x0 + ox, y0 + oy), (x1 + ox, y1 + oy)));
        out.transform = self
            .transform
            .as_ref()
            .map(|corners| shift(corners));
        out
    }
}

fn paint_checkerboard(window: &mut Window, viewport: Bounds<Pixels>, image: Bounds<Pixels>) {
    const CELL: f32 = 8.0;
    let light = rgb(0xffffff);
    let dark = rgb(0xcccccc);
    let x0 = f32::from(image.origin.x).max(f32::from(viewport.origin.x));
    let y0 = f32::from(image.origin.y).max(f32::from(viewport.origin.y));
    let x1 = (f32::from(image.origin.x) + f32::from(image.size.width)).min(f32::from(viewport.origin.x) + f32::from(viewport.size.width));
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
    window.paint_quad(gpui_kit::fill(Bounds::<Pixels>::new(origin, line_size), color));
}

fn paint_rect_outline(window: &mut Window, rect: Bounds<Pixels>, color: gpui_kit::Rgba) {
    let thickness = 1.0;
    let edges = [
        Bounds::new(rect.origin, size(px(f32::from(rect.size.width)), px(thickness))),
        Bounds::new(
            point(
                px(f32::from(rect.origin.x)),
                px(f32::from(rect.origin.y) + f32::from(rect.size.height) - thickness),
            ),
            size(px(f32::from(rect.size.width)), px(thickness)),
        ),
        Bounds::new(rect.origin, size(px(thickness), px(f32::from(rect.size.height)))),
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
        point(px(f32::from(at.x) - SIZE / 2.0), px(f32::from(at.y) - SIZE / 2.0)),
        size(px(SIZE), px(SIZE)),
    );
    window.paint_quad(gpui_kit::fill(rect, rgb(0xffffff)));
    paint_rect_outline(window, rect, rgba(0x000000cc));
}

fn paint_darken_outside(window: &mut Window, viewport: Bounds<Pixels>, rect: Bounds<Pixels>) {
    let color = rgba(0x00000077);
    let vw = f32::from(viewport.origin.x);
    let vh = f32::from(viewport.origin.y);
    let strips = [
        Bounds::new(
            point(px(vw), px(vh)),
            size(px(f32::from(viewport.size.width)), px((f32::from(rect.origin.y) - vh).max(0.0))),
        ),
        Bounds::new(
            point(px(vw), px(f32::from(rect.origin.y) + f32::from(rect.size.height))),
            size(
                px(f32::from(viewport.size.width)),
                px((vh + f32::from(viewport.size.height) - f32::from(rect.origin.y) - f32::from(rect.size.height)).max(0.0)),
            ),
        ),
        Bounds::new(
            point(px(vw), px(f32::from(rect.origin.y))),
            size(px((f32::from(rect.origin.x) - vw).max(0.0)), px(f32::from(rect.size.height))),
        ),
        Bounds::new(
            point(px(f32::from(rect.origin.x) + f32::from(rect.size.width)), px(f32::from(rect.origin.y))),
            size(
                px((vw + f32::from(viewport.size.width) - f32::from(rect.origin.x) - f32::from(rect.size.width)).max(0.0)),
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
    let top = Bounds::new(bounds.origin, size(px(f32::from(bounds.size.width)), px(thickness)));
    let left = Bounds::new(bounds.origin, size(px(thickness), px(f32::from(bounds.size.height))));
    let background = rgb(0x2b2b2b);
    window.paint_quad(gpui_kit::fill(top, background));
    window.paint_quad(gpui_kit::fill(left, background));

    // Ticks every `tick` document pixels, chosen for roughly 50 px on screen.
    let tick = nice_tick(50.0 / scene.zoom.max(0.001));
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

fn nice_tick(rough: f64) -> f64 {
    let candidates = [
        1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0,
    ];
    candidates
        .iter()
        .copied()
        .find(|candidate| *candidate >= rough)
        .unwrap_or(10000.0)
}

#[cfg(test)]
mod tests {
    use super::{handle_at, nice_tick, shape_drag_rect};
    use compositor_core::document::ShapeKind;
    use compositor_core::geometry::{CGPoint, CGRect};

    #[test]
    fn nice_tick_rounds_up_to_a_ruler_step() {
        assert_eq!(nice_tick(0.4), 1.0);
        assert_eq!(nice_tick(3.0), 5.0);
        assert_eq!(nice_tick(7.5), 10.0);
        assert_eq!(nice_tick(120.0), 250.0);
        // Beyond the last candidate it saturates.
        assert_eq!(nice_tick(99_999.0), 10_000.0);
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
            assert_eq!(handle_at(rect, CGPoint::new(x, y)), Some(index), "at {x},{y}");
        }
    }

    #[test]
    fn shape_drag_rect_squares_and_centers() {
        let anchor = CGPoint::new(10.0, 10.0);
        // No modifiers: the plain anchor→point box.
        let rect = shape_drag_rect(ShapeKind::Rectangle, anchor, CGPoint::new(23.0, 27.0), false, false);
        assert_eq!(rect, CGRect::new(10.0, 10.0, 13.0, 17.0));
        // Shift squares it to the longer side, keeping the drag direction.
        let rect = shape_drag_rect(ShapeKind::Rectangle, anchor, CGPoint::new(23.0, 27.0), true, false);
        assert_eq!(rect, CGRect::new(10.0, 10.0, 17.0, 17.0));
        // Option grows a symmetric shape around the anchor.
        let rect = shape_drag_rect(ShapeKind::Rectangle, anchor, CGPoint::new(23.0, 27.0), false, true);
        assert_eq!(rect, CGRect::new(-3.0, -7.0, 26.0, 34.0));
        // Backwards drags normalize to positive sizes.
        let rect = shape_drag_rect(ShapeKind::Ellipse, anchor, CGPoint::new(-5.0, 4.0), false, false);
        assert_eq!(rect, CGRect::new(-5.0, 4.0, 15.0, 6.0));
    }

    #[test]
    fn shape_drag_line_snaps_to_eighths_and_ignores_modifiers() {
        let anchor = CGPoint::new(0.0, 0.0);
        // Shift snaps a shallow drag to exactly horizontal.
        let rect = shape_drag_rect(ShapeKind::Line, anchor, CGPoint::new(100.0, 7.0), true, false);
        assert_eq!(rect, CGRect::new(0.0, 0.0, 100.0, 0.0));
        // A 3-4-5-ish drag snaps to the 45° diagonal, keeping its length
        // (drag_box_rect rounds the snapped end to whole pixels).
        let rect = shape_drag_rect(ShapeKind::Line, anchor, CGPoint::new(10.0, 8.0), true, false);
        let snapped = (164.0f64.sqrt() * std::f64::consts::FRAC_1_SQRT_2).round();
        assert_eq!(rect.size.width, snapped);
        assert_eq!(rect.size.height, snapped);
        // Without shift, no snap.
        let rect = shape_drag_rect(ShapeKind::Line, anchor, CGPoint::new(10.0, 8.0), false, false);
        assert_eq!(rect, CGRect::new(0.0, 0.0, 10.0, 8.0));
        // A line ignores Option centering: it stays the plain box.
        let rect = shape_drag_rect(ShapeKind::Line, anchor, CGPoint::new(10.0, 8.0), false, true);
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
