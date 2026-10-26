//! The canvas viewport: paints the composite, checkerboard, rulers, guides,
//! grid, selection outline, lasso draft, crop box and transform box; owns all
//! pointer interaction with the document.
//!
//! This is the view entity mirroring Swift's `CanvasView` (ADR 0003 step B,
//! ticket 39): one `EditorCanvasView` entity per window holds the gesture
//! state machines and the inline text editor, exactly where the Swift NSView
//! keeps its plain private vars (EditorCanvas.swift:35-97). The window shell
//! (`ProjectWorkspaceView`) keeps the commit/preview/status
//! pipelines and the frame cache; the canvas reaches them through its
//! `app: WeakEntity<ProjectWorkspaceView>` seam (renamed with the root,
//! ticket 43).

use std::sync::Arc;

use gpui_kit::Styled;
use gpui_kit::component::WindowExt;
use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::{
    App, AppContext, Bounds, Context, Corners, DispatchPhase, Entity, InteractiveElement,
    IntoElement, KeyDownEvent, KeyUpEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, PathBuilder, Pixels, Point, Render, RenderImage, ScrollDelta,
    ScrollWheelEvent, Subscription, WeakEntity, Window, canvas, div, point, px, rgb, rgba, size,
};
use gpui_kit::{DragMoveEvent, ExternalPaths};

use compositor_core::canvas::CropDragMode;
use compositor_core::canvas_viewport::CanvasViewport;
use compositor_core::document::LayoutGrid;
use compositor_core::document::{CanvasDocument, GuideAxis, ImageLayer, NavigationTool};
use compositor_core::geometry::{CGPoint, CGRect, CGSize};
use compositor_core::raster::Bitmap;
use compositor_core::selection::LassoKind;
use compositor_core::session::{BlurToolMode, BrushToolMode, EditorSession, SpotHealingMode};
use compositor_core::tools::brush::{GradientShape, RasterEdit};
use compositor_core::tools::warp::WarpStroke;

use crate::canvas::inline_text_editor::{
    inline_text_box, inline_text_box_corners, inline_text_key, inline_text_measured_size,
    inline_text_overflows, inline_text_resize,
};
use crate::state::{ProjectWorkspaceView, RULER_SIZE};

/// What the pointer is currently doing on the canvas. Canvas-private state,
/// mirroring `CanvasView`'s plain private drag vars (they die with the
/// view, which the per-tab rebuild replaces — ticket 43).
pub(crate) enum CanvasDrag {
    /// Hand tool or middle-mouse panning. `last` is the previous event
    /// position: each move translates the session viewport by the delta
    /// (Swift's incremental `viewport.translate(by:)`).
    Pan { last: Point<Pixels> },
    /// A brush-family stroke in progress on the layer captured at
    /// `begin_stroke` (EditorSession+Brush.swift:130-133 commits to the
    /// captured layer, never to whatever is active at mouse-up).
    Stroke(RasterEdit, uuid::Uuid),
    /// Gradient drag: the edit plus the document-space start point, on the
    /// captured layer.
    Gradient(RasterEdit, CGPoint, uuid::Uuid),
    /// Marquee / freehand lasso drag; the draft lives in the session.
    MarqueeLasso,
    /// A Smudge or Liquify stroke working on a document-size copy.
    Warp(WarpStroke),
    /// The Shape tool dragging a shape out; the draft rect grows live.
    Shape { start: CGPoint },
    /// Move tool drag. `duplicate` is the Option modifier at the press: the
    /// first drag tick duplicates the layer and drags the copy
    /// (`duplicatesTransformOnDrag`).
    MoveLayer {
        start_origin: compositor_core::transform::LayerTransform,
        start: CGPoint,
        duplicate: bool,
    },
    /// Crop drag; the drag lives in the session.
    Crop,
    /// Guide drag (create from a ruler or move an existing line); the drag
    /// lives in the session's `guide_drag`.
    Guide,
    /// Cmd-drag moving the selected pixels; the lifted raster lives in the
    /// session's `pixel_move` (`EditorCanvas.pixelDragStart`).
    PixelMove { start: CGPoint },
    /// Plain drag moving the selection outline only; the origin lives in the
    /// session's `selection_move_origin`.
    SelectionMove { start: CGPoint },
    /// The Type tool dragging a paragraph box out (`textBoxRect`); the rect
    /// lives on this view in `text_box_rect`.
    TextBox { start: CGPoint },
    /// An inline text editor resize-handle drag (`InlineTextEditor.mouseDown`
    /// /`mouseDragged`'s resize branch): which handle, where the drag began,
    /// the transform the box showed when it began and the box size the
    /// resize divides by; the draft itself lives in the session.
    TextResize {
        handle: usize,
        start: CGPoint,
        old: compositor_core::transform::LayerTransform,
        source: CGSize,
        draft: compositor_core::session::TextDraft,
    },
}

/// The canvas-side gesture snapshot the app-side preview pipeline reads
/// while compositing the displayed frame ([`ProjectWorkspaceView::composite`]'s drag
/// branches; borrowed per frame — the drag itself never leaves the canvas
/// entity, ticket 39's field audit).
#[derive(Clone, Copy)]
pub(crate) struct CanvasGesture<'a> {
    pub drag: &'a Option<CanvasDrag>,
    pub shape_shape: &'a Option<compositor_core::document::LayerShape>,
    pub shape_rect: Option<CGRect>,
    pub gradient_cursor: Option<CGPoint>,
}

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
    /// The frame's view transform snapshot (`CanvasViewport` is `Copy`).
    pub viewport: CanvasViewport,
    pub doc: (f64, f64),
    pub guides: Vec<(bool, f32)>,
    pub grid: Vec<(f32, bool, bool)>,
    /// The document pixel grid from 800% zoom (`drawPixelGrid`): hairline
    /// positions over the image, with their axis.
    pub pixel_grid: Vec<(f32, bool)>,
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

/// The canvas view (`CanvasView`): paints the document and owns every
/// pointer/keyboard gesture with it, plus the inline text editor overlay.
/// Created by the per-tab content build (`ContentView::new`, ticket 43);
/// a tab switch rebuilds the whole content, so the canvas's session and
/// drag state are always the active tab's.
pub struct EditorCanvasView {
    /// The active tab's session — the view transform, the document and the
    /// gesture state machines all read and write it (Swift `CanvasView`
    /// holds its session reference the same way).
    pub(crate) session: Entity<EditorSession>,
    /// The root view (the window shell): commit/preview/status
    /// pipelines and the frame cache stay there (ADR 0003 step B). No
    /// ProjectWorkspaceView dependencies beyond what the moved handlers already used.
    pub(crate) app: WeakEntity<ProjectWorkspaceView>,
    /// The gesture in progress (`CanvasView`'s plain private drag vars).
    pub(crate) drag: Option<CanvasDrag>,
    /// The Shift stroke axis (`brushAxisAnchor`/`brushAxisHorizontal`,
    /// EditorCanvas.swift:49-51): while Shift is held the stroke locks to a
    /// horizontal or vertical line from the press point; the axis settles
    /// after the first few pixels so it doesn't flip mid-line.
    brush_axis_anchor: Option<CGPoint>,
    brush_axis_horizontal: Option<bool>,
    /// The last pixel a brush stroke applied (`brushLastPixel`,
    /// EditorCanvas.swift:48) — the axis anchor falls back to it when Shift
    /// is pressed mid-stroke.
    brush_last_pixel: Option<CGPoint>,
    /// The tool saved while Space temporarily turns the canvas into the Hand
    /// tool (`spaceHeld` in EditorCanvas.swift).
    space_saved_tool: Option<NavigationTool>,
    /// The gradient tool's drag end, in document coordinates.
    pub(crate) gradient_cursor: Option<CGPoint>,
    /// The Shape tool's drag: the shape being placed and its current rect
    /// (`shapeDraft` lives on the session in Swift; the port carries the
    /// same data in the `CanvasDrag::Shape` gesture family — recorded
    /// deviation, ticket 39's field audit). `pub(crate)` alongside `drag`
    /// for the ticket-50 shape scenario, which drives the production move
    /// branch.
    pub(crate) shape_shape: Option<compositor_core::document::LayerShape>,
    pub(crate) shape_rect: Option<CGRect>,
    /// The Type tool's in-progress box drag (`textBoxRect`).
    text_box_rect: Option<CGRect>,
    /// Where an external file drag hovers, in document coordinates — the
    /// import lands there on drop (`ImageFileDrop`'s drop point).
    pub(crate) drop_point_doc: Option<CGPoint>,
    /// The inline canvas text editor (`InlineTextEditor.swift`): one
    /// persistent multi-line input, shown as an overlay while a text draft
    /// is open (see `inline_text`).
    pub(crate) inline_text_input: Entity<TextareaState>,
    /// Whether the inline editor is bound to the current draft.
    pub(crate) inline_text_open: bool,
    /// The last text pushed into the inline input; keeps the two-way
    /// binding from echoing the user's own keystrokes back.
    pub(crate) inline_text_pushed: Option<String>,
    /// The inline caret's blink phase anchor: any keystroke or caret move
    /// resets it, so the caret holds solid while the user is working
    /// (Swift's `caretBlinkTimer` invalidation on text change).
    pub(crate) inline_text_caret_phase: std::time::Instant,
    /// The canvas paint area in window coordinates, updated by every frame
    /// (`CanvasView.bounds`; the document view transform is the session's
    /// `CanvasViewport`).
    pub(crate) canvas_bounds: Bounds<Pixels>,
    /// Subscriptions owned here: the inline editor's two-way binding watches
    /// the input entity that lives on this view.
    subscriptions: Vec<Subscription>,
}

impl EditorCanvasView {
    pub(crate) fn new(
        session: Entity<EditorSession>,
        app: WeakEntity<ProjectWorkspaceView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The inline editor's NSTextView counterpart: built here because an
        // input state needs the window — the same shape `ProjectWorkspaceView::new` used
        // when the field lived there (moved with the canvas, ticket 39).
        let inline_text_input = cx.new(|cx| TextareaState::new(window, cx).default_value(""));
        let mut canvas = EditorCanvasView {
            session,
            app,
            drag: None,
            brush_axis_anchor: None,
            brush_axis_horizontal: None,
            brush_last_pixel: None,
            space_saved_tool: None,
            gradient_cursor: None,
            shape_shape: None,
            shape_rect: None,
            text_box_rect: None,
            drop_point_doc: None,
            inline_text_input,
            inline_text_open: false,
            inline_text_pushed: None,
            inline_text_caret_phase: std::time::Instant::now(),
            canvas_bounds: Bounds::new(Point::new(px(0.), px(0.)), gpui_kit::size(px(0.), px(0.))),
            subscriptions: Vec::new(),
        };
        canvas.subscribe_inline_text(cx);
        canvas
    }

    /// The inline editor's two-way binding, input → session: typing writes
    /// into the draft (`textDidChange`). Mirrors the subscription
    /// `ProjectWorkspaceView::subscribe_inputs` used to carry; it moved here because the
    /// input entity did (ticket 39).
    fn subscribe_inline_text(&mut self, cx: &mut Context<Self>) {
        let inline = self.inline_text_input.clone();
        let subscription = cx.subscribe(
            &inline,
            move |this: &mut Self, _input, event: &InputEvent, cx: &mut Context<Self>| {
                if let InputEvent::Change = event {
                    let text = this.inline_text_input.read(cx).value().to_string();
                    this.inline_text_changed(&text, cx);
                }
            },
        );
        self.subscriptions.push(subscription);
    }

    /// The canvas's route to the app-side repaint epoch (`touch`): the
    /// frame cache and `revision` stay on ProjectWorkspaceView (ticket 39's field
    /// audit), so a gesture that changed the session asks the shell through
    /// the seam.
    pub(crate) fn touch(&mut self, cx: &mut Context<Self>) {
        self.app.update(cx, |app, cx| app.touch(cx)).ok();
    }

    /// The live-preview variant (ticket 51): the move handler's per-tick
    /// repaint bumps the shell's live epoch — the frame rebuilds, the
    /// document revision and the composite caches stay put until the
    /// gesture commits through `touch` on mouse-up.
    pub(crate) fn touch_live(&mut self, cx: &mut Context<Self>) {
        self.app.update(cx, |app, cx| app.touch_live(cx)).ok();
    }

    /// The active tab's view transform (`CanvasViewport` is `Copy`). Reads
    /// this view's session directly, mirroring Swift's `session.viewport`
    /// reads; the app-side twins (`ProjectWorkspaceView`'s `canvas_viewport` family)
    /// serve the app callers on the same session entity.
    pub(crate) fn canvas_viewport(&self, cx: &App) -> CanvasViewport {
        self.session.read(cx).viewport
    }

    pub(crate) fn doc_size(&self, cx: &App) -> (f64, f64) {
        self.session
            .read(cx)
            .document
            .as_ref()
            .map(|d| (d.width as f64, d.height as f64))
            .unwrap_or((0.0, 0.0))
    }

    pub(crate) fn screen_to_doc(&self, cx: &App, x: f32, y: f32) -> CGPoint {
        let viewport = self.canvas_viewport(cx);
        let (w, h) = self.doc_size(cx);
        viewport.document_point(CGPoint::new(x as f64, y as f64), CGSize::new(w, h))
    }

    /// The document rect in canvas pixels.
    fn image_rect(&self, cx: &App) -> Bounds<Pixels> {
        let viewport = self.canvas_viewport(cx);
        let (w, h) = self.doc_size(cx);
        let per_pixel = viewport.points_per_pixel();
        let origin = viewport.view_point(CGPoint::ZERO, CGSize::new(w, h));
        Bounds::<Pixels>::new(
            Point::new(px(origin.x as f32), px(origin.y as f32)),
            gpui_kit::size(px((w * per_pixel) as f32), px((h * per_pixel) as f32)),
        )
    }
}

impl Render for EditorCanvasView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The retired frame textures leave the sprite atlas here (ticket
        // 50, ADR 0004): gpui frees atlas tiles only through `drop_image`,
        // and during a window update the current window is absent from
        // `App::windows` — so the drop goes through this `Window` at the
        // top of the render, before the new frame's scene references its
        // own texture.
        let retired = self
            .app
            .update(cx, |app, _| app.take_retired_frame_images())
            .unwrap_or_default();
        for image in retired {
            window.drop_image(image).ok();
        }
        // An empty tab shows the welcome form in place of a canvas (Swift's
        // `if session.document == nil { welcome }`, ContentView.swift:99-110 —
        // the tab-embedded NewCanvasSheet). The form is its own view entity
        // (ticket 42); the canvas clones it exactly where `render_canvas`
        // used to branch.
        if self.session.read(cx).document.is_none() {
            return self
                .app
                .update(cx, |app, cx| {
                    app.render_welcome(cx)
                        .map(|welcome| welcome.into_any_element())
                        .unwrap_or_else(|| div().into_any_element())
                })
                .unwrap_or_else(|_| div().into_any_element());
        }
        let gesture = CanvasGesture {
            drag: &self.drag,
            shape_shape: &self.shape_shape,
            shape_rect: self.shape_rect,
            gradient_cursor: self.gradient_cursor,
        };
        let composite = self
            .app
            .update(cx, |app, cx| app.composite(gesture, cx))
            .ok()
            .flatten();
        let image = composite.as_ref().and_then(|composite| {
            self.app
                .update(cx, |app, cx| Some(app.frame_image(composite, cx)))
                .ok()
                .flatten()
        });
        let scene = self.build_scene(image, cx);
        // The inline text editor rides on the canvas while a draft is open
        // (InlineTextEditor.synchronize runs on every display sync in Swift;
        // here the overlay re-renders with the scene, so zoom and pan keep
        // it anchored).
        let inline_overlay = self.render_inline_text_overlay(cx);
        let canvas_move = cx.entity().downgrade();
        let canvas_up = cx.entity().downgrade();
        let canvas_prepaint = cx.entity().downgrade();
        let app = self.app.clone();

        div()
            .id("canvas-area")
            .relative()
            .flex_1()
            .overflow_hidden()
            .bg(rgb(crate::ui::BG_CANVAS_AREA))
            .drag_over::<ExternalPaths>(|style, _, _, _| {
                // File drags tint the canvas (Swift highlights the drop area).
                style.bg(gpui_kit::rgb(crate::ui::BG_DROP_AREA))
            })
            .on_drag_move::<ExternalPaths>(cx.listener(
                |this, event: &DragMoveEvent<ExternalPaths>, _window, cx| {
                    // Remember the hover point in document coordinates: the
                    // drop lands its imports there (ImageFileDrop's point).
                    let local = this.local_position(event.event.position);
                    this.drop_point_doc = Some(this.screen_to_doc(cx, local.0, local.1));
                },
            ))
            .on_drop::<ExternalPaths>(cx.listener(|this, dropped: &ExternalPaths, _window, cx| {
                let paths = dropped.paths().to_vec();
                this.app
                    .update(cx, |app, cx| app.receive_dropped_files(paths, cx))
                    .ok();
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
                        canvas_prepaint
                            .update(cx, |this, cx| {
                                this.canvas_bounds = bounds;
                                // `syncGeometry` (EditorCanvas.swift:719-728):
                                // the canvas size lives on the session
                                // viewport; a change resizes it, which refits
                                // while the viewport still follows fit.
                                let vw = f32::from(bounds.size.width);
                                let vh = f32::from(bounds.size.height);
                                if vw <= 10.0 || vh <= 10.0 {
                                    return;
                                }
                                let needs_sync = this.session.read_with(cx, |session, _| {
                                    session.viewport.view_size != CGSize::new(vw as f64, vh as f64)
                                });
                                if needs_sync {
                                    this.session.update(cx, |session, _| {
                                        let document_size = session.document_size();
                                        session.viewport.resize(
                                            CGSize::new(vw as f64, vh as f64),
                                            1.0,
                                            document_size,
                                        );
                                    });
                                }
                            })
                            .ok();
                    },
                    move |bounds, _prepaint, window, _cx| {
                        paint_scene(&scene, bounds, window);
                        install_drag_events(&canvas_move, &canvas_up, &app, window);
                        install_key_events(&canvas_move, &app, window);
                    },
                )
                .absolute()
                .inset_0(),
            )
            .children(inline_overlay)
            .into_any_element()
    }
}

impl EditorCanvasView {
    fn build_scene(&self, image: Option<Arc<RenderImage>>, cx: &App) -> CanvasScene {
        let session = self.session.read(cx);
        let image_rect = self.image_rect(cx);
        // The frame's view transform; `to_screen` mirrors Swift's direct
        // `session.viewport.viewPoint(from:documentSize:)` reads.
        let viewport = session.viewport;
        let doc_size = session
            .document
            .as_ref()
            .map(|d| CGSize::new(d.width as f64, d.height as f64))
            .unwrap_or(CGSize::ZERO);
        let to_screen = |x: f64, y: f64| {
            let p = viewport.view_point(CGPoint::new(x, y), doc_size);
            (p.x as f32, p.y as f32)
        };
        let mut guides = Vec::new();
        let mut grid = Vec::new();
        let mut pixel_grid = Vec::new();
        let mut selection = Vec::new();
        let mut lasso = None;
        let mut crop = None;
        let mut transform = None;
        let mut text_box = None;
        let mut inline_text_frame = None;
        let mut inline_text_overflow = false;

        if let Some(document) = session.document.as_ref() {
            // `displayedGuides`: a drag in progress replaces its own guide's
            // position live; a new guide joins the list while it is pulled
            // from the ruler.
            let mut drag_drawn = false;
            for guide in &document.guides {
                let (axis, position) = match &session.guide_drag {
                    Some(drag) if drag.id == guide.id => (drag.axis, drag.position),
                    _ => (guide.axis, guide.position),
                };
                drag_drawn |= session.guide_drag.as_ref().map(|d| d.id) == Some(guide.id);
                let vertical = axis == GuideAxis::Vertical;
                let position = if vertical {
                    to_screen(position, 0.0).0
                } else {
                    to_screen(0.0, position).1
                };
                guides.push((vertical, position));
            }
            if let Some(drag) = &session.guide_drag
                && (drag.is_new || !drag_drawn)
            {
                let vertical = drag.axis == GuideAxis::Vertical;
                let position = if vertical {
                    to_screen(drag.position, 0.0).0
                } else {
                    to_screen(0.0, drag.position).1
                };
                guides.push((vertical, position));
            }
            if session.shows_grid {
                for x in LayoutGrid::lines(document.width as f64) {
                    grid.push((to_screen(x, 0.0).0, LayoutGrid::is_major(x), true));
                }
                for y in LayoutGrid::lines(document.height as f64) {
                    grid.push((to_screen(0.0, y).1, LayoutGrid::is_major(y), false));
                }
            }
            // The document pixel grid from 800% (EditorCanvas.swift:778-779).
            if session.shows_pixel_grid && viewport.zoom() >= PIXEL_GRID_ZOOM {
                pixel_grid = pixel_grid_lines(viewport, doc_size, image_rect);
            }
            // The ants follow a pixel move or outline drag (`displayedSelection`).
            if let Some(selection_state) = session.displayed_selection() {
                for ring in &selection_state.path.rings {
                    selection.push(ring.iter().map(|p| to_screen(p.x, p.y)).collect());
                }
            } else if let Some(selection_state) = &document.selection {
                for ring in &selection_state.path.rings {
                    selection.push(ring.iter().map(|p| to_screen(p.x, p.y)).collect());
                }
            }
            if let Some(draft) = &session.lasso_draft {
                let mut rings = vec![
                    draft
                        .points
                        .iter()
                        .map(|p| to_screen(p.x, p.y))
                        .collect::<Vec<(f32, f32)>>(),
                ];
                if let Some(cursor) = draft.cursor
                    && !rings[0].is_empty()
                {
                    let (x, y) = to_screen(cursor.x, cursor.y);
                    rings[0].push((x, y));
                }
                lasso = Some(rings);
            }
            if let Some(rect) = session.crop_rect {
                let min = to_screen(rect.min_x(), rect.min_y());
                let max = to_screen(rect.max_x(), rect.max_y());
                crop = Some((min, max));
            }
            if let Some(edit) = &session.transform_edit {
                // TransformOverlay.swift: handles hide with ⌘H unless an edit must stay visible.
                if edit.draft.is_valid() && (session.shows_transform_controls || edit.persistent) {
                    transform = Some(
                        edit.draft
                            .corners()
                            .iter()
                            .map(|p| to_screen(p.x, p.y))
                            .collect(),
                    );
                }
            }
            // The Type tool's box drag preview (`drawTextBoxDraft`).
            if let Some(rect) = self.text_box_rect {
                text_box = Some((
                    to_screen(rect.min_x(), rect.min_y()),
                    to_screen(rect.max_x(), rect.max_y()),
                ));
            }
            // The inline text editor's frame and handles: the draft's shown
            // frame (`synchronize`'s shownTransform — for point text on a
            // layer, grown with the fresh measurement and anchored back at
            // its top-left corner) rotated about its center.
            if let Some(draft) = &session.text_draft {
                let measured = inline_text_measured_size(&draft.style);
                let asset_width = self.inline_text_asset_width(draft, cx);
                let ((ox, oy), (w, h)) = inline_text_box(
                    draft.origin,
                    draft.transform,
                    draft.style.box_size,
                    measured,
                    asset_width,
                    viewport,
                    doc_size,
                );
                let rotation = draft.transform.map(|t| t.rotation).unwrap_or(0.0);
                inline_text_frame = Some(inline_text_box_corners((ox, oy), (w, h), rotation));
                inline_text_overflow = inline_text_overflows(&draft.style);
            }
        }

        CanvasScene {
            image,
            image_rect,
            rulers: session.shows_rulers,
            viewport,
            doc: self.doc_size(cx),
            guides,
            grid,
            pixel_grid,
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
        // The sheet flag and the Camera Raw eyedropper arm are shell state
        // (ticket 39's field audit); read them through the seam.
        let (sheet_open, samples_defringe) = self
            .app
            .read_with(cx, |app, _| {
                (app.sheet.is_some(), app.camera_raw_samples_defringe)
            })
            .unwrap_or((false, false));
        if sheet_open {
            // The Camera Raw defringe eyedropper samples through the open
            // sheet instead of touching the tools.
            if samples_defringe {
                let local = self.local_position(event.position);
                let doc = self.screen_to_doc(cx, local.0, local.1);
                self.app
                    .update(cx, |app, cx| app.sample_camera_raw_defringe(doc, cx))
                    .ok();
            }
            return;
        }
        if event.click_count > 1 {
            // Double click closes a polygonal lasso instead of starting a drag.
            if self.session.read(cx).tool == NavigationTool::Lasso
                && self.session.read(cx).lasso_kind == LassoKind::Polygonal
                && self.session.read(cx).lasso_draft.is_some()
            {
                self.session.update(cx, |session, _| session.finish_lasso());
                self.touch(cx);
            }
            return;
        }
        let local = self.local_position(event.position);
        let doc = self.screen_to_doc(cx, local.0, local.1);
        let alt = event.modifiers.alt;
        let shift = event.modifiers.shift;

        // The inline text editor's resize handles take a press before any
        // tool (`InlineTextEditor.hitTest`/`mouseDown`); the editor itself
        // swallows presses inside the overlay, so only these reach us.
        if self.session.read(cx).text_draft.is_some()
            && let Some(handle) = self.inline_text_handle_hit(doc, cx)
        {
            self.begin_inline_text_resize(doc, handle, cx);
            return;
        }

        // A press on a ruler strip starts a guide at the pointer's document
        // coordinate, whatever the tool (`CanvasRulerNSView.mouseDown`); the
        // corner square is inert. On the canvas itself, only a Move press that
        // lands on a guide takes it — before any layer drag
        // (EditorCanvas.mouseDown's `beginGuideDrag`).
        if self.session.read(cx).shows_rulers
            && let Some(axis) = ruler_axis_at(local)
        {
            self.session.update(cx, |session, _| {
                session.begin_guide_creation(axis, guide_axis_position(axis, doc))
            });
            self.set_drag(Some(CanvasDrag::Guide), cx);
            self.touch(cx);
            return;
        }
        if self.session.read(cx).tool == NavigationTool::Move {
            // `guideHitDistance` is 5 screen points; the core measures in
            // document pixels.
            let zoom = self.session.read(cx).viewport.zoom();
            let tolerance = GUIDE_HIT_DISTANCE / zoom.max(0.0001);
            if let Some(guide_id) = self.session.read(cx).hit_guide(doc, tolerance) {
                self.session
                    .update(cx, |session, _| session.begin_guide_move(guide_id));
                self.set_drag(Some(CanvasDrag::Guide), cx);
                self.touch(cx);
                return;
            }
        }

        match self.session.read(cx).tool {
            NavigationTool::Hand => {
                self.set_drag(
                    Some(CanvasDrag::Pan {
                        last: event.position,
                    }),
                    cx,
                );
            }
            NavigationTool::Zoom => {
                if alt {
                    self.zoom_at(local, 0.5, cx);
                } else {
                    self.zoom_at(local, 2.0, cx);
                }
            }
            NavigationTool::Marquee | NavigationTool::Lasso => {
                // The polygonal lasso accumulates one point per click; Enter
                // (or double click) closes the outline.
                let polygonal = self.session.read(cx).tool == NavigationTool::Lasso
                    && self.session.read(cx).lasso_kind == LassoKind::Polygonal;
                if polygonal && self.session.read(cx).lasso_draft.is_some() {
                    self.session
                        .update(cx, |session, _| session.extend_lasso(doc));
                    self.touch(cx);
                    return;
                }
                // Cmd-drag inside the selection cuts and moves its pixels
                // (Photoshop's temporary Move tool, Option duplicates); a
                // plain Replace-mode drag inside it moves the outline
                // (EditorCanvas.lassoMouseDown).
                let ctrl = event.modifiers.control;
                let inside = self
                    .session
                    .read(cx)
                    .selection()
                    .map(|s| s.path.contains(doc))
                    .unwrap_or(false);
                if ctrl && inside {
                    if self
                        .session
                        .update(cx, |session, _| session.begin_pixel_move(alt))
                    {
                        self.set_drag(Some(CanvasDrag::PixelMove { start: doc }), cx);
                        self.touch(cx);
                    }
                    return;
                }
                if !ctrl
                    && !shift
                    && !alt
                    && inside
                    && self
                        .session
                        .update(cx, |session, _| session.begin_selection_move())
                {
                    self.set_drag(Some(CanvasDrag::SelectionMove { start: doc }), cx);
                    self.touch(cx);
                    return;
                }
                let mode = self.session.read(cx).selection_mode(shift, alt);
                self.session
                    .update(cx, |session, _| session.begin_lasso(doc, mode));
                self.set_drag(Some(CanvasDrag::MarqueeLasso), cx);
            }
            NavigationTool::Wand => {
                let mode = self.session.read(cx).selection_mode(shift, alt);
                self.session
                    .update(cx, |session, _| session.magic_wand(doc, mode));
            }
            NavigationTool::Brush
            | NavigationTool::SpotHealing
            | NavigationTool::CloneStamp
            | NavigationTool::Blur => {
                // Shift paints a straight line on from where the last stroke
                // ended, as in Photoshop (EditorCanvas.swift:1449-1451):
                // `beginBrush` at the old end, then `continueBrush` at the
                // click — warp strokes ride the same branch.
                if shift && let Some(from) = self.session.read(cx).shift_line_start() {
                    self.begin_brush_at(from, alt, cx);
                    match self.drag.as_mut() {
                        Some(CanvasDrag::Stroke(edit, _)) => {
                            edit.append_smoothed(doc);
                        }
                        Some(CanvasDrag::Warp(stroke)) => {
                            stroke.append(doc);
                        }
                        _ => {}
                    }
                    self.session.update(cx, |session, _| {
                        if let Some(last) = session.last_brush_point.as_mut() {
                            last.point = doc;
                        }
                    });
                } else {
                    self.begin_brush_at(doc, alt, cx);
                }
                // Shift keeps the stroke straight from the press point
                // (EditorCanvas.swift:1455-1456).
                self.brush_axis_anchor = shift.then_some(doc);
                self.brush_axis_horizontal = None;
                self.brush_last_pixel = Some(doc);
            }
            NavigationTool::Gradient => {
                self.begin_gradient(doc, cx);
            }
            NavigationTool::Move => {
                self.begin_move(doc, event.modifiers.control, event.modifiers.alt, cx);
            }
            NavigationTool::Crop => {
                self.begin_crop(doc, cx);
            }
            NavigationTool::Eyedropper => {
                // The picker open: the sample loads into its working color
                // (`sampleIntoColorPicker`, EditorCanvas.swift:2042).
                // Otherwise the foreground swatch, behind `canEditPalette`
                // (:2043).
                if self.session.read(cx).color_picker.is_some() {
                    self.session
                        .update(cx, |session, _| session.sample_into_color_picker(doc));
                } else if self.session.read(cx).can_edit_palette()
                    && let Some(document) = self.session.read(cx).document.as_ref()
                    && let Some(color) =
                        compositor_core::render::sample_composite_color(document, doc)
                {
                    self.session.update(cx, |session, _| {
                        session.brush_settings.red = color.red;
                        session.brush_settings.green = color.green;
                        session.brush_settings.blue = color.blue;
                    });
                    self.set_status(format!("Foreground #{}", color.hex()), cx);
                }
            }
            NavigationTool::Type => {
                self.type_mouse_down(doc, window, cx);
            }
            NavigationTool::Shape => {
                self.begin_shape(doc, cx);
            }
            _ => {}
        }
        self.touch(cx);
    }

    /// The status line lives on the shell (ticket 39's split): gestures
    /// that report through it route via the seam.
    fn set_status(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.app
            .update(cx, |app, cx| app.set_status(message, cx))
            .ok();
    }

    pub fn canvas_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // The sheet flag is shell state; a drag ticks nothing while a sheet
        // is up, exactly as before (read through the seam).
        let sheet_open = self
            .app
            .read_with(cx, |app, _| app.sheet.is_some())
            .unwrap_or(false);
        if sheet_open {
            return false;
        }
        let local = self.local_position(event.position);
        let doc = self.screen_to_doc(cx, local.0, local.1);
        let shift = event.modifiers.shift;
        let alt = event.modifiers.alt;
        // The Move-layer drag pushes its preview into the inspector's fields
        // (the fields are shell state and the sync needs the window), so the
        // tick is reported to the window listener which runs it (ticket 39's
        // seam; the sync ran inside this method before the split).
        let mut sync_fields = false;

        // Swift pans incrementally (`viewport.translate(by:)`,
        // EditorCanvas.swift:1616): each move applies the delta since the
        // last position and ends the fit follow. Handled before the borrow
        // of `self.drag` below, since the translate updates the session.
        let pan_delta = match self.drag.as_ref() {
            Some(CanvasDrag::Pan { last }) => Some((
                (f32::from(event.position.x) - f32::from(last.x)) as f64,
                (f32::from(event.position.y) - f32::from(last.y)) as f64,
            )),
            _ => None,
        };
        if let Some((dx, dy)) = pan_delta {
            self.session.update(cx, |session, _| {
                session.viewport.translate(CGSize::new(dx, dy))
            });
            if let Some(CanvasDrag::Pan { last }) = self.drag.as_mut() {
                *last = event.position;
            }
        }

        // The brush strokes' Shift axis runs on this view's own fields, so it
        // goes before the drag borrow (EditorCanvas.swift:1551-1561).
        let stroke_drag = matches!(
            self.drag,
            Some(CanvasDrag::Stroke(..)) | Some(CanvasDrag::Warp(..))
        );
        let doc = if stroke_drag {
            self.constrain_stroke_axis(doc, shift)
        } else {
            doc
        };

        match self.drag.as_mut() {
            Some(CanvasDrag::Pan { .. }) => {}
            Some(CanvasDrag::Stroke(edit, _)) => {
                self.brush_last_pixel = Some(doc);
                edit.append_smoothed(doc);
                // `lastBrushPoint?.point = painted`
                // (EditorSession+Brush.swift:70).
                self.session.update(cx, |session, _| {
                    if let Some(last) = session.last_brush_point.as_mut() {
                        last.point = doc;
                    }
                });
            }
            Some(CanvasDrag::Warp(stroke)) => {
                stroke.append(doc);
                // `lastBrushPoint?.point = point` — warp continues update the
                // line anchor too (EditorSession+Brush.swift:66).
                self.session.update(cx, |session, _| {
                    if let Some(last) = session.last_brush_point.as_mut() {
                        last.point = doc;
                    }
                });
            }
            Some(CanvasDrag::Shape { start }) => {
                let rect =
                    shape_drag_rect(self.session.read(cx).shape_kind, *start, doc, shift, alt);
                let shape = match self
                    .app
                    .update(cx, |app, cx| app.build_shape_for_drag(rect, cx))
                {
                    Ok(shape) => shape,
                    Err(_) => return false,
                };
                self.shape_rect = Some(rect);
                self.shape_shape = Some(shape);
            }
            Some(CanvasDrag::Gradient(..)) => {
                self.gradient_cursor = Some(doc);
            }
            Some(CanvasDrag::MarqueeLasso) => {
                let kind = self.session.read(cx).lasso_draft.as_ref().map(|d| d.kind);
                if matches!(kind, Some(LassoKind::Rectangle) | Some(LassoKind::Ellipse)) {
                    self.session
                        .update(cx, |session, _| session.drag_marquee(doc, shift, false));
                } else {
                    self.session
                        .update(cx, |session, _| session.extend_lasso(doc));
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
                    self.session
                        .update(cx, |session, _| session.text_draft = Some(draft));
                }
            }
            Some(CanvasDrag::MoveLayer {
                start_origin,
                start,
                duplicate,
            }) => {
                let (start_origin, start, duplicate) = (*start_origin, *start, *duplicate);
                if duplicate {
                    // Option turns the first drag tick into a duplicate: the
                    // copy becomes the transform target and follows the rest
                    // of the drag (duplicatesTransformOnDrag,
                    // EditorCanvas.swift:1589). The flag clears so later
                    // ticks only move the copy.
                    if let Some(CanvasDrag::MoveLayer {
                        duplicate: flag, ..
                    }) = self.drag.as_mut()
                    {
                        *flag = false;
                    }
                    self.session
                        .update(cx, |session, _| session.begin_duplicate_transform());
                }
                let delta = CGPoint::new(doc.x - start.x, doc.y - start.y);
                let mut value = start_origin;
                value.origin.x += delta.x;
                value.origin.y += delta.y;
                // Moving snaps to the canvas and the other layers; resizing
                // and rotating are left alone (EditorCanvas.dragTransform),
                // and Control drags freely (EditorCanvas.swift:1602). The
                // tolerance is TransformSnap.distance (10) as document
                // pixels; Swift divides by pointsPerPixel, so it is exact
                // only at zoom 1 (deviation recorded in the diff report).
                let moving: Vec<uuid::Uuid> = {
                    let session = self.session.read(cx);
                    session
                        .transform_edit
                        .as_ref()
                        .map(|edit| match &edit.group {
                            Some(group) => group.originals.iter().map(|(id, _)| *id).collect(),
                            None => vec![edit.layer_id],
                        })
                        .unwrap_or_default()
                };
                let value = if event.modifiers.control {
                    value
                } else {
                    self.session
                        .update(cx, |session, _| session.snapped_move(value, &moving, 10.0))
                };
                self.session
                    .update(cx, |session, _| session.preview_transform(value));
                // The inspector's fields follow the drag live
                // (TransformInspector's binding to editedTransform).
                sync_fields = true;
            }
            Some(CanvasDrag::PixelMove { start }) => {
                let start = *start;
                self.session.update(cx, |session, _| {
                    session.drag_pixel_move(CGSize::new(doc.x - start.x, doc.y - start.y))
                });
            }
            Some(CanvasDrag::SelectionMove { start }) => {
                let start = *start;
                self.session.update(cx, |session, _| {
                    session.move_selection_outline(CGSize::new(doc.x - start.x, doc.y - start.y))
                });
            }
            Some(CanvasDrag::Crop) => {
                let ratio = self.session.read(cx).crop_ratio();
                self.session
                    .update(cx, |session, _| session.drag_crop(doc, ratio, false));
            }
            Some(CanvasDrag::Guide) => {
                // The guide follows the pointer's document coordinate on its
                // own axis (EditorCanvas.mouseDragged's guideDragging branch).
                if let Some(drag) = self.session.read(cx).guide_drag.as_ref() {
                    let axis = drag.axis;
                    self.session.update(cx, |session, _| {
                        session.move_guide_drag(guide_axis_position(axis, doc))
                    });
                }
            }
            None => return false,
        }
        // Every move arm funnels here: the tick is a live preview (the
        // drag's result stays uncommitted until mouse-up), so the shell's
        // live epoch — not the document revision — moves (ticket 51).
        self.touch_live(cx);
        sync_fields
    }

    /// Returns whether the shell's transform fields need their follow-up
    /// sync (`sync_transform_inputs` after a Move-layer drag; the fields and
    /// the window the sync needs live on the shell, so the window listener
    /// runs it — ticket 39's seam).
    pub fn canvas_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let drag = self.drag.take();
        self.session
            .update(cx, |session, _| session.has_brush_stroke = false);
        let mut sync_fields = false;
        if let Some(drag) = drag {
            match drag {
                CanvasDrag::Stroke(mut edit, id) => {
                    edit.flush();
                    self.app
                        .update(cx, |app, cx| app.commit_stroke(edit, id, cx))
                        .ok();
                }
                CanvasDrag::Gradient(edit, start, id) => {
                    let local = self.local_position(event.position);
                    let end = self.screen_to_doc(cx, local.0, local.1);
                    self.gradient_cursor = Some(end);
                    self.app
                        .update(cx, |app, cx| {
                            let result = app.gradient_result(&edit, start, Some(end), cx);
                            app.commit_pixels(
                                compositor_core::raster::ImportedImage::new(result, String::new()),
                                EditTarget {
                                    layer_id: id,
                                    is_mask: edit.is_mask,
                                },
                                edit.paint_transform,
                                "Gradient",
                                cx,
                            );
                        })
                        .ok();
                }
                CanvasDrag::Warp(stroke) => {
                    self.finish_warp(stroke, cx);
                }
                CanvasDrag::Shape { .. } => {
                    self.finish_shape(cx);
                }
                CanvasDrag::MarqueeLasso => {
                    self.session.update(cx, |session, _| session.finish_lasso());
                }
                CanvasDrag::PixelMove { .. } => {
                    self.session
                        .update(cx, |session, _| session.end_pixel_move());
                }
                CanvasDrag::SelectionMove { .. } => {
                    self.session
                        .update(cx, |session, _| session.end_selection_move());
                }
                CanvasDrag::Crop => {
                    self.session
                        .update(cx, |session, _| session.end_crop_drag());
                }
                CanvasDrag::MoveLayer { .. } => {
                    self.session
                        .update(cx, |session, _| session.commit_transform());
                    sync_fields = true;
                }
                CanvasDrag::Guide => {
                    // Releasing on a ruler cancels a new guide or deletes an
                    // existing one (`finishGuideDrag(delete:)`).
                    let local = self.local_position(event.position);
                    let delete = guide_release_deletes(self.session.read(cx).shows_rulers, local);
                    self.session
                        .update(cx, |session, _| session.finish_guide_drag(delete));
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
        sync_fields
    }

    pub fn canvas_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sheet_open = self
            .app
            .read_with(cx, |app, _| app.sheet.is_some())
            .unwrap_or(false);
        if sheet_open {
            return;
        }
        let local = self.local_position(event.position);
        if local.0 < 0.0
            || local.1 < 0.0
            || local.0 > f32::from(self.canvas_bounds.size.width)
            || local.1 > f32::from(self.canvas_bounds.size.height)
        {
            return;
        }
        let dy = match event.delta {
            ScrollDelta::Pixels(pt) => f32::from(pt.y),
            ScrollDelta::Lines(pt) => pt.y * 40.0,
        };
        if dy != 0.0 {
            let factor = (-dy / 400.0).exp();
            self.zoom_at(local, factor as f64, cx);
            self.touch(cx);
        }
    }

    /// Zoom by a factor with the document point under `screen` pinned
    /// (`session.zoom(to:anchor:)`, EditorSession.swift:913-916).
    fn zoom_at(&mut self, screen: (f32, f32), factor: f64, cx: &mut Context<Self>) {
        let current = self.session.read(cx).viewport.zoom();
        let anchor = CGPoint::new(screen.0 as f64, screen.1 as f64);
        self.session.update(cx, |session, _| {
            session.zoom(current * factor, Some(anchor))
        });
    }

    /// Starts/ends a canvas drag, keeping the tab's `has_brush_stroke`
    /// busy flag in step (Swift keeps `brushStroke`/`warpStroke` on the
    /// session; since stage-D ticket 34 the flag itself is a session
    /// field, so `can_switch` blocks tab changes mid-stroke through the
    /// session guard).
    pub(crate) fn set_drag(&mut self, drag: Option<CanvasDrag>, cx: &mut Context<Self>) {
        self.drag = drag;
        self.session.update(cx, |session, _| {
            session.has_brush_stroke = matches!(
                self.drag,
                Some(CanvasDrag::Stroke(..))
                    | Some(CanvasDrag::Warp(_))
                    | Some(CanvasDrag::Gradient(..))
                    | Some(CanvasDrag::Shape { .. })
            );
        });
    }

    pub(crate) fn local_position(&self, position: Point<Pixels>) -> (f32, f32) {
        (
            f32::from(position.x) - f32::from(self.canvas_bounds.origin.x),
            f32::from(position.y) - f32::from(self.canvas_bounds.origin.y),
        )
    }

    // MARK: Strokes

    /// The brush family's mouse-down dispatch (EditorCanvas.swift's
    /// `isBrushTool` branch): smudge/liquify ride the warp engine, everything
    /// else starts a raster stroke.
    fn begin_brush_at(&mut self, doc: CGPoint, alt: bool, cx: &mut Context<Self>) {
        if self.session.read(cx).tool == NavigationTool::Blur {
            match self.session.read(cx).blur_mode {
                BlurToolMode::Smudge | BlurToolMode::Liquify => {
                    self.begin_warp(doc, cx);
                }
                BlurToolMode::Blur => {
                    self.begin_stroke(doc, alt, cx);
                }
            }
        } else {
            self.begin_stroke(doc, alt, cx);
        }
    }

    /// Shift keeps the stroke straight, horizontal or vertical, from wherever
    /// it was pressed; letting go carries on freehand. The axis is settled by
    /// the first few pixels of movement, so it doesn't flip mid-line
    /// (EditorCanvas.swift:1551-1561): the anchor is the press point, falling
    /// back to the last applied pixel when Shift comes on mid-stroke, and the
    /// pointer rides the anchor until the axis settles.
    fn constrain_stroke_axis(&mut self, doc: CGPoint, shift: bool) -> CGPoint {
        if !shift {
            self.brush_axis_anchor = None;
            self.brush_axis_horizontal = None;
            return doc;
        }
        let anchor = self
            .brush_axis_anchor
            .or(self.brush_last_pixel)
            .unwrap_or(doc);
        if self.brush_axis_anchor.is_none() {
            self.brush_axis_anchor = Some(anchor);
            self.brush_axis_horizontal = None;
        }
        if self.brush_axis_horizontal.is_none() && (doc.x - anchor.x).hypot(doc.y - anchor.y) >= 3.0
        {
            self.brush_axis_horizontal = Some((doc.x - anchor.x).abs() >= (doc.y - anchor.y).abs());
        }
        match self.brush_axis_horizontal {
            Some(true) => CGPoint::new(doc.x, anchor.y),
            Some(false) => CGPoint::new(anchor.x, doc.y),
            None => anchor,
        }
    }

    /// `pub(crate)` for the ticket-49 stroke scenario: the harness drives
    /// the production stroke begin (paint target, guards, `RasterEdit::new`
    /// and all).
    pub(crate) fn begin_stroke(&mut self, doc: CGPoint, alt: bool, cx: &mut Context<Self>) {
        let Some(document) = self.session.read(cx).document.clone() else {
            return;
        };
        let Some(id) = self.session.read(cx).active_layer_id else {
            return;
        };
        let Some(layer) = document.layer(id) else {
            return;
        };
        // canPaint (EditorSession+Brush.swift:5-12) splits into the surface
        // rules and the state guards, both ahead of everything else —
        // Swift's guard runs before the clone-source handling too.
        let Some(target) = paint_target(self.session.read(cx), &document, layer) else {
            return;
        };
        if !can_paint(self.session.read(cx), &document) {
            return;
        }
        let mask_target = target.is_mask;
        // Spot Healing and Clone Stamp rework image pixels; they have
        // nothing to do on a mask (EditorSession+Brush.swift:31).
        if mask_target
            && matches!(
                self.session.read(cx).tool,
                NavigationTool::SpotHealing | NavigationTool::CloneStamp
            )
        {
            return;
        }
        // Clone Stamp: option-click sets the source; strokes copy from it.
        if self.session.read(cx).tool == NavigationTool::CloneStamp {
            if alt {
                self.session
                    .update(cx, |session, _| session.clone_source = Some(doc));
                self.set_status(
                    format!("Clone source set at {:.0}, {:.0}", doc.x, doc.y),
                    cx,
                );
                return;
            }
            if self.session.read(cx).clone_source.is_none() {
                self.set_status("Option-click to set a clone source first", cx);
                return;
            }
        }
        let mut settings = self.session.read(cx).brush_settings;
        settings.erasing = self.session.read(cx).brush_mode == BrushToolMode::Erase;
        settings.healing = self.session.read(cx).tool == NavigationTool::SpotHealing;
        // Smoothing trails the pointer on a string of `smoothing` screen
        // points — in document pixels the radius divides by the zoom. Only
        // the Brush tool smooths (EditorSession+Brush.swift:78).
        if self.session.read(cx).tool == NavigationTool::Brush {
            settings.smoothing /= self.session.read(cx).viewport.zoom().max(0.0001);
        } else {
            settings.smoothing = 0.0;
        }
        // A mask stroke paints white (reveal) or black (hide).
        if mask_target {
            let v = if self.session.read(cx).mask_paint_white {
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
        if self.session.read(cx).tool == NavigationTool::CloneStamp
            && let Some(offset) = self.session.read(cx).clone_stroke_offset(doc)
            && let Some(sample) = self.sample_for_clone(cx)
        {
            if self.session.read(cx).clone_settings.aligned
                && self.session.read(cx).clone_offset.is_none()
            {
                self.session
                    .update(cx, |session, _| session.clone_offset = Some(offset));
            }
            edit.clone = Some((sample, offset));
        }
        // The Blur brush: the stroke's own target sampled at its start — the
        // layer as displayed, or the mask's own coverage when the mask is the
        // target — softened by an amount that follows the brush size, painted
        // in place through the tip (BlurTool.swift:8-40,
        // EditorSession+Brush.swift:42-46). This used to sample the layer even
        // on a mask target, stamping layer pixels into the mask.
        if self.session.read(cx).tool == NavigationTool::Blur {
            let displayed = self.session.read(cx).displayed_transform(layer);
            if let Some(sample) = blur_sample(&document, layer, &displayed, mask_target) {
                let blurred =
                    blur_sample_softened(&sample, self.session.read(cx).brush_settings.diameter);
                edit.clone = Some((blurred, CGSize::ZERO));
            }
        }
        edit.append_smoothed(doc);
        // `lastBrushPoint = (point, layer.id, isMaskSelected)`
        // (EditorSession+Brush.swift:61) — the Shift-click line anchor.
        self.session.update(cx, |session, _| {
            session.last_brush_point = Some(compositor_core::session::LastBrushPoint {
                point: doc,
                layer_id: id,
                mask: mask_target,
            });
        });
        self.set_drag(Some(CanvasDrag::Stroke(edit, id)), cx);
    }

    /// What the Clone Stamp samples: the composite or the active layer.
    fn sample_for_clone(&self, cx: &App) -> Option<Bitmap> {
        let session = self.session.read(cx);
        let document = session.document.as_ref()?;
        if session.clone_settings.sample_all_layers {
            return Some(compositor_core::render::draw_live_composite(document));
        }
        // The active layer as the canvas displays it: drawn through its
        // displayed transform at document size (CloneStamp.swift:39-48).
        // Swift's `cloneSample` does not multiply the sample by the layer's
        // mask coverage — a stamped stroke clones the layer's own pixels even
        // where the mask hides them — so neither do we (this used to darken
        // the sample under an enabled mask).
        let layer = session.active_layer()?;
        let image = layer.asset.as_ref()?.image.as_ref().clone();
        let transform = session.displayed_transform(layer);
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
/// stroke IS the drag (the busy flags themselves are session fields since
/// stage-D ticket 34 — `has_brush_stroke` mirrors the drag), and a pending
/// core gradient never coexists with a fresh press (Swift's beginGradient
/// bypasses canEditLayers for its re-anchor too).
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

impl ProjectWorkspaceView {
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
            let mode = match self
                .workspace
                .read(cx)
                .current()
                .session
                .read(cx)
                .spot_healing_mode
            {
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
        } else if self.workspace.read(cx).current().session.read(cx).tool == NavigationTool::Blur {
            "Blur"
        } else if self.workspace.read(cx).current().session.read(cx).tool
            == NavigationTool::CloneStamp
        {
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

    // MARK: Shape

    /// The shape the current drag places, from the tool's settings.
    /// `pub(crate)` for the ticket-50 shape scenario (the `begin_stroke`
    /// precedent: the harness drives the production path).
    pub(crate) fn build_shape_for_drag(
        &self,
        _rect: CGRect,
        cx: &App,
    ) -> compositor_core::document::LayerShape {
        let session = self.current_tab(cx).session.read(cx);
        let kind = session.shape_kind;
        let color = session.palette_color(false);
        compositor_core::document::LayerShape {
            kind,
            corner_radius: session.shape_corner_radius,
            line_width: session.shape_line_width,
            red: color.red,
            green: color.green,
            blue: color.blue,
            filled: kind != compositor_core::document::ShapeKind::Line,
            start: CGPoint::ZERO,
            end: CGPoint::new(1.0, 1.0),
        }
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
        self.history_edit(
            name,
            move |session| {
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
            },
            cx,
        );
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

    // MARK: Keys

    /// Tab cycles the active tool's mode.
    pub fn cycle_tool_mode(&mut self, cx: &mut Context<Self>) {
        if self.workspace.read(cx).current().session.read(cx).tool == NavigationTool::Shape {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.toggle_shape_kind())
            });
            self.touch(cx);
            return;
        }
        match self.workspace.read(cx).current().session.read(cx).tool {
            NavigationTool::Marquee => self.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.toggle_marquee_kind())
            }),
            NavigationTool::Lasso => self.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.toggle_lasso_kind())
            }),
            NavigationTool::Wand => {
                self.workspace.update(cx, |ws, cx| {
                    ws.current_mut().session.update(cx, |session, _| {
                        session.wand_mode = match session.wand_mode {
                            compositor_core::selection::WandMode::Wand => {
                                compositor_core::selection::WandMode::Object
                            }
                            compositor_core::selection::WandMode::Object => {
                                compositor_core::selection::WandMode::Wand
                            }
                        };
                    })
                });
            }
            NavigationTool::Brush => {
                self.workspace.update(cx, |ws, cx| {
                    ws.current_mut().session.update(cx, |session, _| {
                        session.brush_mode = match session.brush_mode {
                            BrushToolMode::Paint => BrushToolMode::Erase,
                            BrushToolMode::Erase => BrushToolMode::Paint,
                        };
                    })
                });
            }
            NavigationTool::Gradient => {
                self.workspace.update(cx, |ws, cx| {
                    ws.current_mut().session.update(cx, |session, _| {
                        session.gradient_settings.shape = match session.gradient_settings.shape {
                            GradientShape::Linear => GradientShape::Radial,
                            GradientShape::Radial => GradientShape::Linear,
                        };
                    })
                });
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
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .color_picker
            .is_some()
        {
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
        if self
            .workspace
            .update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.lasso_draft.take())
            })
            .is_some()
        {
            self.touch(cx);
            return;
        }
        // The Type tool's box drag preview dies with the gesture (the box
        // rect is canvas state since ticket 39; the canvas is the active
        // tab content's child, ticket 43).
        let text_box_open = self.content.as_ref().and_then(|content| {
            let canvas = content.read(cx).canvas_view.clone();
            canvas.update(cx, |canvas, _| canvas.text_box_rect.take())
        });
        if text_box_open.is_some() {
            self.touch(cx);
            return;
        }
        if self.workspace.update(cx, |ws, cx| {
            ws.current_mut().session.update(cx, |session, _| {
                if session.crop_drag.take().is_some() {
                    session.crop_rect = None;
                    true
                } else {
                    false
                }
            })
        }) {
            self.touch(cx);
            return;
        }
        // Escape kills a guide drag before it reaches the transform edit
        // (EditorCanvas.keyDown's `guideDrag != nil` branch).
        if self
            .workspace
            .update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.guide_drag.take())
            })
            .is_some()
        {
            self.touch(cx);
            return;
        }
        if self.workspace.update(cx, |ws, cx| {
            ws.current_mut().session.update(cx, |session, _| {
                if session.pixel_move.take().is_some() {
                    session.cancel_pixel_move();
                    true
                } else {
                    false
                }
            })
        }) {
            self.touch(cx);
            return;
        }
        if self.workspace.update(cx, |ws, cx| {
            ws.current_mut().session.update(cx, |session, _| {
                if session.selection_move_origin.take().is_some() {
                    session.end_selection_move();
                    true
                } else {
                    false
                }
            })
        }) {
            self.touch(cx);
            return;
        }
        if self
            .workspace
            .update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.transform_edit.take())
            })
            .is_some()
        {
            self.touch(cx);
            return;
        }
        if self
            .workspace
            .update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.crop_rect.take())
            })
            .is_some()
        {
            self.touch(cx);
            return;
        }
        if self
            .workspace
            .update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.renaming_layer_id.take())
            })
            .is_some()
        {
            // The rename field is the layers panel's (ticket 40; the panel
            // is the active tab content's child, ticket 43).
            if let Some(content) = self.content.as_ref() {
                let panel = content.read(cx).layers_panel.clone();
                panel.update(cx, |panel, _| panel.discard_rename_input());
            }
            self.touch(cx);
        }
    }
}

impl EditorCanvasView {
    // MARK: Smudge / Liquify

    /// `beginWarp`: the stroke works on the active layer as the canvas shows
    /// it, at document size.
    fn begin_warp(&mut self, doc: CGPoint, cx: &mut Context<Self>) {
        let Some(document) = self.session.read(cx).document.clone() else {
            return;
        };
        let Some(id) = self.session.read(cx).active_layer_id else {
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
            self.session.read(cx).blur_mode,
            &self.session.read(cx).brush_settings,
        );
        stroke.layer_asset = layer.asset.as_ref().map(|a| a.image.clone());
        stroke.layer_transform = layer.transform;
        stroke.append(doc);
        self.set_drag(Some(CanvasDrag::Warp(stroke)), cx);
    }

    /// `finishWarp`: paints the finished result into the layer's own pixels
    /// along the stroke, as one undo step. The commit itself is shell-side
    /// (`commit_pixels`, ticket 39's split).
    fn finish_warp(&mut self, stroke: WarpStroke, cx: &mut Context<Self>) {
        let name = match stroke.mode {
            BlurToolMode::Smudge => "Smudge",
            BlurToolMode::Liquify => "Liquify",
            BlurToolMode::Blur => "Blur",
        };
        let Some(document) = self.session.read(cx).document.clone() else {
            return;
        };
        let Some(id) = self.session.read(cx).active_layer_id else {
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
            self.app
                .update(cx, |app, cx| {
                    app.commit_pixels(
                        snapshot,
                        EditTarget {
                            layer_id: id,
                            is_mask: false,
                        },
                        transform,
                        name,
                        cx,
                    );
                })
                .ok();
        }
    }

    // MARK: Shape

    fn begin_shape(&mut self, doc: CGPoint, cx: &mut Context<Self>) {
        self.set_drag(Some(CanvasDrag::Shape { start: doc }), cx);
        self.shape_rect = Some(CGRect::new(doc.x, doc.y, 0.0, 0.0));
        // The shape settings are session state; the builder is shell-side
        // (the preview pipeline, ticket 39's split).
        if let Ok(shape) = self.app.update(cx, |app, cx| {
            app.build_shape_for_drag(CGRect::new(doc.x, doc.y, 0.0, 0.0), cx)
        }) {
            self.shape_shape = Some(shape);
        }
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
        let kind = self.session.read(cx).shape_kind;
        let Ok(shape) = self
            .app
            .update(cx, |app, cx| app.build_shape_for_drag(rect, cx))
        else {
            return;
        };
        self.session.update(cx, |session, _| {
            let name = session.next_shape_name(kind);
            session.add_shape_layer(shape, rect, name);
        });
        self.touch(cx);
    }

    // MARK: Gradient

    fn begin_gradient(&mut self, doc: CGPoint, cx: &mut Context<Self>) {
        let Some(document) = self.session.read(cx).document.clone() else {
            return;
        };
        let Some(id) = self.session.read(cx).active_layer_id else {
            return;
        };
        let Some(layer) = document.layer(id) else {
            return;
        };
        // Gradients land on the same surface under the same canPaint gate
        // as a brush stroke (Gradient.swift:36-38, EditorSession+Brush.swift:
        // 5-12) — a drag that would refuse to paint never opens a draft.
        let Some(target) = paint_target(self.session.read(cx), &document, layer) else {
            return;
        };
        if !can_paint(self.session.read(cx), &document) {
            return;
        }
        let clip = document.selection.as_ref().map(|s| s.clip(document.size()));
        let edit = RasterEdit::new(
            target.source,
            target.base_transform,
            target.is_mask,
            self.session.read(cx).brush_settings,
            CGSize::new(document.width as f64, document.height as f64),
            clip,
        );
        self.gradient_cursor = Some(doc);
        self.set_drag(Some(CanvasDrag::Gradient(edit, doc, id)), cx);
    }

    // MARK: Move

    fn begin_move(&mut self, doc: CGPoint, command: bool, option: bool, cx: &mut Context<Self>) {
        // A picked layer becomes the transform target before the drag starts.
        if self.session.read(cx).transform_edit.is_none() {
            let active_id = self.session.read(cx).active_layer_id;
            let auto_select = self.session.read(cx).transform_auto_select;
            if let Some(document) = self.session.read(cx).document.as_ref()
                && let Some((id, picked)) =
                    transform_press_layer(document, active_id, auto_select, command, true, doc)
                && picked
            {
                self.session
                    .update(cx, |session, _| session.select_layer(Some(id)));
            }
        }
        if self.session.read(cx).transform_edit.is_none() {
            if self.session.read(cx).can_transform() {
                // The drag begins a transient transform (persistent false,
                // EditorCanvas.swift:2129).
                self.session
                    .update(cx, |session, _| session.begin_transform_persistent(false));
            } else {
                return;
            }
        }
        let Some(draft) = self
            .session
            .read(cx)
            .transform_edit
            .as_ref()
            .map(|edit| edit.draft)
        else {
            return;
        };
        self.set_drag(
            Some(CanvasDrag::MoveLayer {
                start_origin: draft,
                start: doc,
                // A Move drag duplicates when Option is held
                // (duplicatesTransformOnDrag, EditorCanvas.swift:2127).
                duplicate: option,
            }),
            cx,
        );
        self.touch(cx);
    }

    // MARK: Crop

    fn begin_crop(&mut self, doc: CGPoint, cx: &mut Context<Self>) {
        let mode = if let Some(rect) = self.session.read(cx).crop_rect {
            match handle_at(rect, doc) {
                Some(index) => CropDragMode::Resize(index),
                None if rect.contains_point(doc) => CropDragMode::Move,
                None => CropDragMode::Create,
            }
        } else {
            CropDragMode::Create
        };
        self.session
            .update(cx, |session, _| session.begin_crop_drag(doc, mode));
        self.set_drag(Some(CanvasDrag::Crop), cx);
        self.touch(cx);
    }

    // MARK: Type

    /// The Type tool's press (`beginTextGesture`): a press on a visible text
    /// layer edits it inline; anywhere else the press anchors a text box
    /// drag, and the release decides between point text and a paragraph box.
    /// Any open draft commits first, so clicking away applies the text
    /// (empty drafts are a quiet no-op, as in Swift).
    fn type_mouse_down(&mut self, doc: CGPoint, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.read(cx).document.is_none() {
            return;
        }
        if !self.session.update(cx, |session, _| session.finish_text()) {
            self.touch(cx);
            return;
        }
        self.sync_inline_text(window, cx);
        let document = self.session.read(cx).document.clone().unwrap();
        let visible = document.effective_visible_ids();
        if let Some(layer) = document.layers.iter().rev().find(|l| {
            visible.contains(&l.id) && l.live_text().is_some() && l.transform.contains(doc)
        }) {
            let id = layer.id;
            self.session.update(cx, |session, _| {
                session.select_layer(Some(id));
                session.edit_active_text();
            });
            self.sync_inline_text(window, cx);
            self.touch(cx);
            return;
        }
        self.text_box_rect = Some(CGRect::new(doc.x, doc.y, 0.0, 0.0));
        self.set_drag(Some(CanvasDrag::TextBox { start: doc }), cx);
    }

    /// The Type tool's release (`finishTextGesture`): a click without a drag
    /// lays point text at the click; a drag sets the paragraph bounds.
    fn type_mouse_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rect) = self.text_box_rect.take() else {
            return;
        };
        if rect.width() < 4.0 && rect.height() < 4.0 {
            self.session
                .update(cx, |session, _| session.begin_text(rect.origin, true));
        } else {
            self.session
                .update(cx, |session, _| session.begin_text_in(rect));
        }
        self.sync_inline_text(window, cx);
        self.touch(cx);
    }

    // MARK: Keys

    /// Space keyDown: the canvas pans until the key comes back up
    /// (EditorCanvas.swift:1825).
    fn space_down(&mut self, cx: &mut Context<Self>) {
        if self.space_saved_tool.is_some() {
            return;
        }
        let (has_document, has_draft, tool) = {
            let session = self.session.read(cx);
            (
                session.document.is_some(),
                session.text_draft.is_some(),
                session.tool,
            )
        };
        if !has_document || has_draft {
            return;
        }
        self.space_saved_tool = Some(tool);
        self.session
            .update(cx, |session, _| session.tool = NavigationTool::Hand);
        self.touch(cx);
    }

    /// Space keyUp: the saved tool comes back (EditorCanvas.swift:1866).
    fn space_up(&mut self, cx: &mut Context<Self>) {
        if let Some(tool) = self.space_saved_tool.take() {
            self.session.update(cx, |session, _| session.tool = tool);
            self.touch(cx);
        }
    }

    /// The raw canvas keys: Space hold-to-pan, opacity digits, Option-arrow
    /// text spacing and ⌘Return (`EditorCanvas.keyDown`). Runs on the bubble
    /// phase; keys that mean something here don't reach the action system.
    /// The keys that route into shell-side pipelines (the text-style edits,
    /// the tool sliders' opacity digits) do so through the seam; the digit
    /// arm needs the window for the slider writes, so it is reported back to
    /// the window listener, which runs `ProjectWorkspaceView::opacity_digit`.
    pub fn canvas_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<i32> {
        let key = event.keystroke.key.as_str();
        let modifiers = &event.keystroke.modifiers;
        let bare = !(modifiers.platform || modifiers.control || modifiers.alt || modifiers.shift);
        // While the inline canvas text editor owns the keyboard, its keys
        // route first (`CanvasTextView.keyDown`): ⌘Return finishes, Escape
        // cancels, Option-arrows step the spacing, everything else types.
        // The Option-arrows normally arrive as actions instead —
        // `keys::inline_text_bindings` binds them above the text system's
        // own alt-arrow bindings and the overlay div's handlers consume
        // them — so this gate's spacing arms are a fallback that only fires
        // when no binding matched (they can never double-step: a consumed
        // keystroke never reaches this listener).
        if self.inline_text_focused(window, cx) {
            use crate::canvas::inline_text_editor::InlineTextKey;
            if let Some(action) = inline_text_key(
                key,
                modifiers.control,
                modifiers.alt,
                modifiers.platform,
                modifiers.shift,
            ) {
                match action {
                    InlineTextKey::Finish => self.commit_inline_text(cx),
                    InlineTextKey::Cancel => self.cancel_inline_text(cx),
                    InlineTextKey::Tracking(delta) => {
                        self.app
                            .update(cx, |app, cx| app.adjust_text_tracking(delta, cx))
                            .ok();
                    }
                    InlineTextKey::Leading(down, step) => {
                        self.app
                            .update(cx, |app, cx| app.adjust_text_leading(down, step, cx))
                            .ok();
                    }
                }
            }
            cx.stop_propagation();
            return None;
        }
        // `ProjectWorkspaceView::canvas_keys_active`'s two conjuncts, evaluated here:
        // the sheet flag is shell state; the focused-input check reads the
        // window (the gate itself stays shell-side for the hotkey wrapper).
        let sheet_open = self
            .app
            .read_with(cx, |app, _| app.sheet.is_some())
            .unwrap_or(false);
        if sheet_open || window.has_focused_input(cx) {
            return None;
        }
        // ⌘Return finishes the text draft (⌘ maps to Ctrl on Windows).
        if modifiers.control && !modifiers.alt && key == "enter" {
            self.app.update(cx, |app, cx| app.finish_text_edit(cx)).ok();
            cx.stop_propagation();
            return None;
        }
        // Space holds the Hand tool; a text draft keeps typing into the box.
        if key == "space" && bare && self.session.read(cx).text_draft.is_none() {
            self.space_down(cx);
            cx.stop_propagation();
            return None;
        }
        // Option with the arrows sets the text's spacing: left and right the
        // tracking, up and down the leading; Shift makes each step ten.
        if modifiers.alt && !modifiers.control && !modifiers.platform {
            let step = if modifiers.shift { 10.0 } else { 1.0 };
            match key {
                "left" => {
                    self.app
                        .update(cx, |app, cx| app.adjust_text_tracking(-step, cx))
                        .ok();
                    cx.stop_propagation();
                    return None;
                }
                "right" => {
                    self.app
                        .update(cx, |app, cx| app.adjust_text_tracking(step, cx))
                        .ok();
                    cx.stop_propagation();
                    return None;
                }
                "up" => {
                    self.app
                        .update(cx, |app, cx| app.adjust_text_leading(false, step, cx))
                        .ok();
                    cx.stop_propagation();
                    return None;
                }
                "down" => {
                    self.app
                        .update(cx, |app, cx| app.adjust_text_leading(true, step, cx))
                        .ok();
                    cx.stop_propagation();
                    return None;
                }
                _ => {}
            }
        }
        // Number keys set opacity when the tool uses it (usesOpacityKeys).
        if bare
            && let Ok(digit) = key.parse::<i32>()
            && (0..=9).contains(&digit)
        {
            cx.stop_propagation();
            return Some(digit);
        }
        None
    }

    /// The raw canvas key releases: Space gives the saved tool back.
    pub fn canvas_key_up(
        &mut self,
        event: &KeyUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "space" {
            self.space_up(cx);
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
    // The active layer is only the drag target when it has pixels, isn't a
    // group, and is effectively visible (EditorCanvas.swift:1320-1322).
    let visible = document.effective_visible_ids();
    let active = active_id.and_then(|id| {
        order
            .iter()
            .find(|l| l.id == id)
            .filter(|l| l.asset.is_some() && !l.is_group && visible.contains(&l.id))
            .copied()
    });
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

/// `pixelGridZoom` (EditorCanvas.swift:800): the pixel grid appears from 800%.
const PIXEL_GRID_ZOOM: f64 = 8.0;

/// `drawPixelGrid` (EditorCanvas.swift:1087-1110) — one hairline per document
/// pixel boundary, over the image only. The lines' extent is the canvas rect;
/// painting clips them to the window.
fn pixel_grid_lines(
    viewport: CanvasViewport,
    doc_size: CGSize,
    image_rect: Bounds<Pixels>,
) -> Vec<(f32, bool)> {
    if image_rect.size.width <= px(0.0) || image_rect.size.height <= px(0.0) {
        return Vec::new();
    }
    let first = viewport.document_point(
        CGPoint::new(
            f64::from(image_rect.origin.x),
            f64::from(image_rect.origin.y),
        ),
        doc_size,
    );
    let last = viewport.document_point(
        CGPoint::new(
            f64::from(image_rect.origin.x) + f64::from(image_rect.size.width),
            f64::from(image_rect.origin.y) + f64::from(image_rect.size.height),
        ),
        doc_size,
    );
    let mut lines = Vec::new();
    for column in (first.x.ceil() as i64)..=(last.x.floor() as i64) {
        let x = viewport
            .view_point(CGPoint::new(column as f64, 0.0), doc_size)
            .x;
        lines.push((x as f32, true));
    }
    for row in (first.y.ceil() as i64)..=(last.y.floor() as i64) {
        let y = viewport
            .view_point(CGPoint::new(0.0, row as f64), doc_size)
            .y;
        lines.push((y as f32, false));
    }
    lines
}

/// Paints the pixel grid: hairline quads, one screen pixel wide
/// (`hairline = 1 / backingScale`), white 0.55 at 0.45 alpha.
fn paint_pixel_grid(window: &mut Window, image_bounds: Bounds<Pixels>, lines: &[(f32, bool)]) {
    if image_bounds.size.width <= px(0.0) || image_bounds.size.height <= px(0.0) {
        return;
    }
    let hairline = px(1.0 / window.scale_factor());
    let color = rgba(0x8c8c8c73);
    for (position, vertical) in lines {
        let quad = if *vertical {
            Bounds::new(
                point(px(*position) - hairline / 2.0, image_bounds.origin.y),
                size(hairline, image_bounds.size.height),
            )
        } else {
            Bounds::new(
                point(image_bounds.origin.x, px(*position) - hairline / 2.0),
                size(image_bounds.size.width, hairline),
            )
        };
        window.paint_quad(gpui_kit::fill(quad, color));
    }
}

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

    // The document pixel grid sits over the image, under everything else
    // (EditorCanvas.draw's drawPixelGrid call).
    paint_pixel_grid(window, image_bounds, &scene.pixel_grid);

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

    // Selection outline: a white solid line under a static black dash.
    // Swift's marching ants animate the dash phase
    // (TransformOverlay.drawSelection); the port paints them still — the
    // animation gap is recorded in the diff report.
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
        out.pixel_grid = out
            .pixel_grid
            .iter()
            .map(|(position, vertical)| {
                (
                    if *vertical {
                        position + ox
                    } else {
                        position + oy
                    },
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

    // Ticks every `tick` document pixels, chosen for roughly 70 px on screen.
    let viewport = scene.viewport;
    let doc_size = CGSize::new(scene.doc.0, scene.doc.1);
    let per_pixel = viewport.points_per_pixel();
    let tick = nice_tick(70.0 / per_pixel.max(0.001));
    let color = rgba(0xb4b4b4aa);
    let mut value = 0.0f64;
    while value <= scene.doc.0 {
        let screen = viewport.view_point(CGPoint::new(value, 0.0), doc_size).x;
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
    while value <= scene.doc.1 {
        let screen = viewport.view_point(CGPoint::new(0.0, value), doc_size).y;
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

// MARK: Drag events

/// Registers the window-level listeners that continue an in-progress drag
/// when the pointer leaves the canvas. Called during paint; listeners are
/// re-registered every frame. The gesture runs on the canvas entity; when a
/// Move-layer tick (or its release commit) needs the shell's transform-field
/// sync, the canvas reports it and this listener runs the shell side — the
/// sync needs the window, which only the listener owns during dispatch
/// (ticket 39's seam; the call used to sit inside `canvas_mouse_move`).
fn install_drag_events(
    canvas_move: &gpui_kit::WeakEntity<EditorCanvasView>,
    canvas_up: &gpui_kit::WeakEntity<EditorCanvasView>,
    app: &gpui_kit::WeakEntity<ProjectWorkspaceView>,
    window: &mut Window,
) {
    let canvas_move = canvas_move.clone();
    let canvas_up = canvas_up.clone();
    let app_move_sync = app.clone();
    let app_up_sync = app.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, _phase, window, cx| {
        let sync_fields = canvas_move
            .update(cx, |canvas, cx| {
                if canvas.drag.is_some() {
                    canvas.canvas_mouse_move(event, window, cx)
                } else {
                    false
                }
            })
            .unwrap_or(false);
        if sync_fields {
            app_move_sync
                .update(cx, |app, cx| app.sync_transform_inputs(window, cx))
                .ok();
        }
    });
    window.on_mouse_event(move |event: &MouseUpEvent, _phase, window, cx| {
        let sync_fields = canvas_up
            .update(cx, |canvas, cx| canvas.canvas_mouse_up(event, window, cx))
            .unwrap_or(false);
        if sync_fields {
            app_up_sync
                .update(cx, |app, cx| app.sync_transform_inputs(window, cx))
                .ok();
        }
    });
}

/// Registers the window-level key listeners for the canvas keys the action
/// system can't express: Space's hold-to-pan (needs keyUp), the opacity
/// digits, Option-arrow text spacing and ⌘Return finishing text
/// (`EditorCanvas.keyDown`/`keyUp`). Listeners run each frame, so they must
/// be re-registered during paint like the drag events above. The canvas
/// state machine routes everything it owns; the opacity digit is reported
/// back so the listener runs the shell-side `opacity_digit` (the tool
/// sliders are shell state and the write needs the window — ticket 39's
/// seam; the call used to sit inside `canvas_key_down`).
fn install_key_events(
    canvas: &gpui_kit::WeakEntity<EditorCanvasView>,
    app: &gpui_kit::WeakEntity<ProjectWorkspaceView>,
    window: &mut Window,
) {
    let canvas_down = canvas.clone();
    let app_digit = app.clone();
    window.on_key_event(move |event: &KeyDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        let digit = canvas_down
            .update(cx, |canvas, cx| canvas.canvas_key_down(event, window, cx))
            .unwrap_or(None);
        if let Some(digit) = digit {
            app_digit
                .update(cx, |app, cx| app.opacity_digit(digit, window, cx))
                .ok();
        }
    });
    let canvas_up = canvas.clone();
    window.on_key_event(move |event: &KeyUpEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        canvas_up
            .update(cx, |canvas, cx| canvas.canvas_key_up(event, window, cx))
            .ok();
    });
}

#[cfg(test)]
mod tests {
    use super::{
        blur_mask_sample, blur_sample, blur_sample_softened, blur_sigma, can_paint,
        clone_layer_sample, guide_axis_position, guide_release_deletes, handle_at, nice_tick,
        paint_target, pixel_grid_lines, ruler_axis_at, shape_drag_rect, transform_press_layer,
    };
    use crate::state::RULER_SIZE;
    use compositor_core::canvas_viewport::CanvasViewport;
    use compositor_core::document::{
        CanvasDocument, GuideAxis, ImageLayer, LayerMask, NavigationTool, ShapeKind,
    };
    use compositor_core::geometry::{CGPoint, CGRect, CGSize};
    use compositor_core::raster::{Bitmap, ImportedImage};
    use compositor_core::selection::{DocumentSelection, SelectionPath};
    use compositor_core::session::EditorSession;
    use compositor_core::transform::LayerTransform;
    use gpui_kit::{Bounds, point, px, size};
    use uuid::Uuid;

    /// Whole-pixel tick label (`CanvasRulers.label`). Production rendering
    /// has no ruler ticks yet; the helper keeps its ported shape here for
    /// the label tests until it gains a caller.
    fn ruler_label(value: f64) -> String {
        let rounded = value.round();
        if rounded == 0.0 {
            "0".to_string()
        } else {
            (rounded as i64).to_string()
        }
    }

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

    /// `drawPixelGrid` (EditorCanvas.swift:1087-1110): one hairline per
    /// document pixel boundary, vertical and horizontal, over the image only.
    #[test]
    fn pixel_grid_lines_follow_the_document_pixel_boundaries() {
        let mut viewport = CanvasViewport::default();
        viewport.resize(
            CGSize::new(160.0, 160.0),
            1.0,
            Some(CGSize::new(16.0, 16.0)),
        );
        viewport.set_zoom(10.0, CGPoint::new(80.0, 80.0), CGSize::new(16.0, 16.0));
        let rect = Bounds::new(point(px(0.0), px(0.0)), size(px(160.0), px(160.0)));
        let lines = pixel_grid_lines(viewport, CGSize::new(16.0, 16.0), rect);
        let verticals = lines.iter().filter(|(_, vertical)| *vertical).count();
        let horizontals = lines.iter().filter(|(_, vertical)| !*vertical).count();
        // The canvas fills the viewport exactly, so every boundary 0..=16
        // lands inside (Swift's ceil/floor stride keeps the edges).
        assert_eq!(verticals, 17);
        assert_eq!(horizontals, 17);
    }

    #[test]
    fn pixel_grid_lines_need_an_image() {
        let viewport = CanvasViewport::default();
        let empty = Bounds::new(point(px(0.0), px(0.0)), size(px(0.0), px(0.0)));
        assert!(pixel_grid_lines(viewport, CGSize::new(16.0, 16.0), empty).is_empty());
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
        let document = CanvasDocument::new(40, 40);
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

    /// `insertPaintedLayer` (TransformTests.swift:131-143): a document-sized
    /// fully red layer, inserted as the new active layer.
    fn insert_painted_layer(session: &mut EditorSession) -> Uuid {
        let (width, height) = {
            let document = session.document.as_ref().unwrap();
            (document.width, document.height)
        };
        let mut image = Bitmap::new(width, height);
        for px in image.data.chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }
        session.insert_image(ImportedImage::new(image, "Painted layer"), None);
        session.active_layer_id.unwrap()
    }

    /// A Move-tool click: the pick half of `begin_move` (the press), then
    /// the release commit (the Swift cases' `mouseDown`/`mouseUp` +
    /// `commitTransform()`).
    fn move_click(session: &mut EditorSession, at: CGPoint, command: bool) {
        if session.transform_edit.is_none() {
            let active_id = session.active_layer_id;
            let document = session.document.clone().unwrap();
            if let Some((id, picked)) = transform_press_layer(
                &document,
                active_id,
                session.transform_auto_select,
                command,
                true,
                at,
            ) && picked
            {
                session.select_layer(Some(id));
            }
        }
        session.commit_transform();
    }

    /// `autoSelectCanBeDisabledAndCommandClickOverridesIt`
    /// (TransformTests.swift:56-71): Auto Select starts off and a press
    /// drags the active layer; on, a press picks the layer under the
    /// pointer; Cmd picks either way without flipping the toggle.
    #[test]
    fn auto_select_can_be_disabled_and_command_click_overrides_it() {
        let mut session = EditorSession::new();
        session.create_document(400, 200, false);
        let first = insert_painted_layer(&mut session);
        session.document.as_mut().unwrap().layers[0].transform =
            LayerTransform::new(CGPoint::ZERO, CGSize::new(100.0, 100.0));
        let second = insert_painted_layer(&mut session);
        session.document.as_mut().unwrap().layers[1].transform =
            LayerTransform::new(CGPoint::new(250.0, 0.0), CGSize::new(100.0, 100.0));
        session.select_layer(Some(first));

        assert!(
            !session.transform_auto_select,
            "Auto Select starts off; a press drags the active layer"
        );
        session.transform_auto_select = true;
        move_click(&mut session, CGPoint::new(300.0, 50.0), false);
        assert_eq!(session.active_layer_id, Some(second));

        session.select_layer(Some(first));
        session.transform_auto_select = false;
        move_click(&mut session, CGPoint::new(300.0, 50.0), false);
        assert_eq!(session.active_layer_id, Some(first));

        move_click(&mut session, CGPoint::new(300.0, 50.0), true);
        assert_eq!(session.active_layer_id, Some(second));
        assert!(!session.transform_auto_select);
    }

    /// `autoSelectPicksForegroundLayerStackedOnSelectedBackground`
    /// (TransformTests.swift:96-111): with Auto Select on, a press inside
    /// both the selected full-canvas background and a foreground layer
    /// above it prefers the foreground; off, the background keeps the
    /// press.
    #[test]
    fn auto_select_picks_foreground_layer_stacked_on_selected_background() {
        let mut session = EditorSession::new();
        session.create_document(400, 200, false);
        let background = insert_painted_layer(&mut session);
        session.document.as_mut().unwrap().layers[0].transform =
            LayerTransform::new(CGPoint::ZERO, CGSize::new(400.0, 200.0));
        let foreground = insert_painted_layer(&mut session);
        session.document.as_mut().unwrap().layers[1].transform =
            LayerTransform::new(CGPoint::new(150.0, 50.0), CGSize::new(80.0, 60.0));
        session.select_layer(Some(background));

        session.transform_auto_select = true;
        move_click(&mut session, CGPoint::new(180.0, 70.0), false);
        assert_eq!(session.active_layer_id, Some(foreground));

        move_click(&mut session, CGPoint::new(20.0, 20.0), false);
        assert_eq!(session.active_layer_id, Some(background));

        move_click(&mut session, CGPoint::new(180.0, 70.0), false);
        assert_eq!(session.active_layer_id, Some(foreground));

        session.select_layer(Some(background));
        session.transform_auto_select = false;
        move_click(&mut session, CGPoint::new(180.0, 70.0), false);
        assert_eq!(session.active_layer_id, Some(background));
    }

    /// The active-layer guards of `transformPressLayer`
    /// (EditorCanvas.swift:1320-1322): the active layer is only the drag
    /// target when it has pixels, isn't a group, and is effectively
    /// visible. Hidden layers aren't in `renderLayers` either, so they are
    /// never under the pointer and auto-select can't pick them.
    #[test]
    fn transform_press_active_layer_needs_pixels_group_and_visibility() {
        let mut doc = CanvasDocument::new(100, 100);

        // A group active: no pixels of its own, never the drag target —
        // and with auto-select on the press still finds nothing to pick.
        let mut group = opaque_layer(80, 80, [255, 0, 0, 255], CGPoint::ZERO);
        group.asset = None;
        group.is_group = true;
        let group_id = group.id;
        doc.layers = vec![group];
        assert_eq!(
            transform_press_layer(
                &doc,
                Some(group_id),
                false,
                false,
                true,
                CGPoint::new(10.0, 10.0)
            ),
            None
        );
        assert_eq!(
            transform_press_layer(
                &doc,
                Some(group_id),
                true,
                false,
                true,
                CGPoint::new(10.0, 10.0)
            ),
            None
        );

        // A hidden active layer is no target either; invisible layers
        // aren't under the pointer, so auto-select on changes nothing.
        let mut hidden = opaque_layer(80, 80, [255, 0, 0, 255], CGPoint::ZERO);
        let hidden_id = hidden.id;
        hidden.is_visible = false;
        doc.layers = vec![hidden];
        assert_eq!(
            transform_press_layer(
                &doc,
                Some(hidden_id),
                false,
                false,
                true,
                CGPoint::new(10.0, 10.0)
            ),
            None
        );
        assert_eq!(
            transform_press_layer(
                &doc,
                Some(hidden_id),
                true,
                false,
                true,
                CGPoint::new(10.0, 10.0)
            ),
            None
        );

        // Effective visibility runs through groups: a visible layer under
        // a hidden folder is hidden with it.
        let mut parent = opaque_layer(80, 80, [255, 0, 0, 255], CGPoint::ZERO);
        parent.asset = None;
        parent.is_group = true;
        let parent_id = parent.id;
        let mut child = opaque_layer(50, 50, [0, 255, 0, 255], CGPoint::ZERO);
        let child_id = child.id;
        child.parent_id = Some(parent_id);
        doc.layers = vec![parent, child];
        assert_eq!(
            transform_press_layer(
                &doc,
                Some(child_id),
                false,
                false,
                true,
                CGPoint::new(10.0, 10.0)
            ),
            Some((child_id, false)),
            "visible under a visible folder"
        );
        doc.layers
            .iter_mut()
            .find(|l| l.id == parent_id)
            .unwrap()
            .is_visible = false;
        assert_eq!(
            transform_press_layer(
                &doc,
                Some(child_id),
                false,
                false,
                true,
                CGPoint::new(10.0, 10.0)
            ),
            None
        );
    }

    fn near(a: CGPoint, b: CGPoint) -> bool {
        (a.x - b.x).abs() < 0.5 && (a.y - b.y).abs() < 0.5
    }

    /// The Move tool's press-drag-release as the canvas runs it:
    /// `begin_move`'s pick half and transient transform, the
    /// `CanvasDrag::MoveLayer` tick — where Option duplicates on the first
    /// tick (`duplicatesTransformOnDrag`, EditorCanvas.swift:1589) and
    /// Control skips the snap ("Control drags freely", :1602) — and the
    /// release commit (`canvas_mouse_up`).
    fn move_drag(
        session: &mut EditorSession,
        from: CGPoint,
        to: CGPoint,
        command: bool,
        option: bool,
        control: bool,
    ) {
        if session.transform_edit.is_none() {
            let active_id = session.active_layer_id;
            let auto_select = session.transform_auto_select;
            let document = session.document.clone().unwrap();
            if let Some((id, picked)) =
                transform_press_layer(&document, active_id, auto_select, command, true, from)
                && picked
            {
                session.select_layer(Some(id));
            }
        }
        if session.transform_edit.is_none() && session.can_transform() {
            session.begin_transform_persistent(false);
        }
        let Some(draft) = session.transform_edit.as_ref().map(|edit| edit.draft) else {
            return;
        };
        // The first drag tick: Option duplicates, and the copy becomes the
        // transform target.
        if option {
            session.begin_duplicate_transform();
        }
        // The draft follows the whole pointer delta (TransformDrag.updated
        // in move mode).
        let mut value = draft;
        value.origin.x += to.x - from.x;
        value.origin.y += to.y - from.y;
        let moving: Vec<Uuid> = session
            .transform_edit
            .as_ref()
            .map(|edit| match &edit.group {
                Some(group) => group.originals.iter().map(|(id, _)| *id).collect(),
                None => vec![edit.layer_id],
            })
            .unwrap_or_default();
        let value = if control {
            value
        } else {
            session.snapped_move(value, &moving, 10.0)
        };
        session.preview_transform(value);
        session.commit_transform();
    }

    /// A 100 × 100 red layer centered in the document
    /// (`TransformPressTests.makeCanvas`: "centered: 150–250 × 100–200").
    fn insert_centered_red_layer(session: &mut EditorSession) {
        let mut image = Bitmap::new(100, 100);
        for px in image.data.chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }
        session.insert_image(ImportedImage::new(image, "Red"), None);
    }

    /// `draggingOutsideTheLayerMovesIt` (TransformPressTests.swift:48-54):
    /// with the Move tool a press outside the layer's bounds still drags
    /// it — the layer follows the whole delta. Control is held so the move
    /// doesn't snap (the Swift case's doc comment: without it a 20 × 10
    /// drag lands inside `TransformSnap.distance` and the layer springs
    /// back — the snapping working, not the press failing).
    #[test]
    fn dragging_outside_the_layer_moves_it() {
        let mut session = EditorSession::new();
        session.create_document(400, 300, false);
        insert_centered_red_layer(&mut session);
        session.select_tool(NavigationTool::Move);

        move_drag(
            &mut session,
            CGPoint::new(20.0, 20.0),
            CGPoint::new(40.0, 30.0),
            false,
            false,
            true,
        );
        assert!(
            session.transform_edit.is_none(),
            "the transient transform closes on release"
        );
        let origin = session.active_layer().unwrap().transform.origin;
        assert!(
            near(origin, CGPoint::new(170.0, 110.0)),
            "layer origin {origin:?}"
        );
    }

    /// `optionDraggingOutsideTheLayerDuplicatesIt`
    /// (TransformPressTests.swift:56-63): an Option drag outside the layer
    /// duplicates it — the original stays and the copy follows the delta.
    #[test]
    fn option_dragging_outside_the_layer_duplicates_it() {
        let mut session = EditorSession::new();
        session.create_document(400, 300, false);
        insert_centered_red_layer(&mut session);
        session.select_tool(NavigationTool::Move);

        move_drag(
            &mut session,
            CGPoint::new(20.0, 20.0),
            CGPoint::new(40.0, 30.0),
            false,
            true,
            true,
        );
        let origins: Vec<CGPoint> = session
            .document
            .as_ref()
            .unwrap()
            .layers
            .iter()
            .map(|l| l.transform.origin)
            .collect();
        assert_eq!(origins.len(), 2, "{origins:?}");
        assert!(
            origins.iter().any(|o| near(*o, CGPoint::new(150.0, 100.0)))
                && origins.iter().any(|o| near(*o, CGPoint::new(170.0, 110.0))),
            "the original stays and the copy moves: {origins:?}"
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
        let mut viewport = compositor_core::canvas_viewport::CanvasViewport::default();
        viewport.set_zoom(2.0, CGPoint::ZERO, CGSize::ZERO);
        viewport.pan = CGSize::new(20.0, 10.0);
        let doc = viewport.document_point(CGPoint::new(120.0, 60.0), CGSize::ZERO);
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
