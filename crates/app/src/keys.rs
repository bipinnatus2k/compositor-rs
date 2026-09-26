//! Actions and keyboard shortcuts (KeyboardShortcuts.swift). The default
//! chords and any user overrides live in `shortcuts.rs`; `bindings()`
//! assembles the gpui keymap from that registry.

use gpui_kit::KeyBinding;
use gpui_kit::actions;

actions!(compositor, [
    // Tools
    ToolMove, ToolMarquee, ToolLasso, ToolWand, ToolCrop, ToolBrush, ToolEraser,
    ToolHealing, ToolClone, ToolSmear, ToolGradient, ToolShape, ToolType,
    ToolEyedropper, ToolHand, ToolZoom,
    // Palette and tool modes
    SwapColors, DefaultColors, CycleToolMode, CycleShapeKind,
    BrushSofter, BrushHarder, BlendModeNext, BlendModePrevious,
    // File
    NewCanvas, OpenDocument, SaveDocument, SaveAs, ExportPng, ExportJpeg, ImportImages,
    CloseDocument,
    // Edit
    Undo, Redo, FillForeground, FillBackground, FreeTransform, ShowShortcuts,
    Cut, CopyPixels, CopyMerged, PastePixels,
    ShowTransformControls, ContentAwareFill,
    CurvesSheet, InvertPixels, ToggleClippingMask,
    MoveLayerUp, MoveLayerDown, ToggleLevelsPreview,
    // Image
    ImageSize, CanvasSize, TrimSheet, CropToSelection,
    FlipCanvasHorizontal, FlipCanvasVertical,
    LevelsSheet, HueSaturationSheet,
    // Layer
    NewLayer, DuplicateLayer, DeleteLayer, MergeDown, NewGroup, AddMask,
    FlipLayerHorizontal, FlipLayerVertical,
    GroupSelectedLayers, MoveOutOfFolder, RenameLayer, ToggleLayerVisibility,
    EditAdjustment,
    // Select / Edit extras
    ClearSelectionPixels,
    LoadLayerSelection, LoadMaskSelection,
    // Image adjustments that run through the filter sheet
    FilterBlackWhite, FilterColorBalance, FilterExposure, FilterGradientMap, FilterGrain,
    RemoveBackground, ClearGuides,
    // New Adjustment Layer submenu (AdjustmentKind)
    AddAdjustmentHsv, AddAdjustmentLevels, AddAdjustmentCurves, AddAdjustmentExposure,
    AddAdjustmentGradientMap, AddAdjustmentGrain, AddAdjustmentAddNoise, AddAdjustmentGaussianBlur,
    AddAdjustmentMotionBlur, AddAdjustmentInvert, AddAdjustmentBlackWhite, AddAdjustmentColorBalance,
    // Select
    SelectAllAction, DeselectAction, InvertSelection,
    FeatherSheet, ExpandSheet, ContractSheet,
    // Filter
    FilterGaussianBlur, FilterMotionBlur, FilterAddNoise, FilterLensCorrection,
    FilterCameraRaw,
    // View
    ToggleRulers, ToggleGrid, ToggleGuides, TogglePixelGrid,
    SnapToggle, SnapGuides, SnapGrid, SnapLayers, SnapBounds, LockGuides,
    ZoomIn, ZoomOut, ZoomFit, Zoom100,
    // Window / Help
    ResetPanels, About,
    // Canvas keys
    NudgeLeft, NudgeRight, NudgeUp, NudgeDown,
    NudgeLeftMore, NudgeRightMore, NudgeUpMore, NudgeDownMore,
    BrushSmaller, BrushLarger, FinishLasso, CancelAction,
]);

/// Every key binding, bound globally so menus show hints and canvas keys work
/// as soon as the workspace has focus. Handlers ignore most actions while a
/// text input owns the keyboard.
///
/// The bindings come from the shortcut registry
/// (KeyboardShortcuts.swift:129-213): the catalog in `shortcuts.rs` holds the
/// default chord of every action and any user override loaded from
/// `shortcuts.json`, so custom chords take effect here (and in the menu bar's
/// key hints, which read the same keymap).
pub fn bindings() -> Vec<KeyBinding> {
    bindings_with(&crate::shortcuts::lock_global())
}

/// Builds the keymap from an explicit registry (the injection point for
/// tests and for a live rebind after the settings sheet saves).
pub fn bindings_with(registry: &crate::shortcuts::ShortcutRegistry) -> Vec<KeyBinding> {
    bindings_tracked(registry, None)
}

/// Same as [`bindings_with`], optionally recording the shortcut id behind
/// every emitted binding so tests can prove the binding table and the
/// shortcut catalog never drift apart.
pub(crate) fn bindings_tracked(
    registry: &crate::shortcuts::ShortcutRegistry,
    mut ids: Option<&mut Vec<&'static str>>,
) -> Vec<KeyBinding> {
    /// Binds every effective chord of one catalog entry (the override chord
    /// if the user customized the action, otherwise all default aliases).
    macro_rules! bind {
        ($out:expr, $ids:expr, $registry:expr, $id:literal, $action:ident) => {{
            if let Some(record) = $ids.as_mut() {
                record.push($id);
            }
            for keystroke in $registry.keystrokes_for($id) {
                $out.push(KeyBinding::new(&keystroke, $action, $registry.context_for($id)));
            }
        }};
    }

    let mut out = Vec::with_capacity(96);
    bind!(out, ids, registry, "tool.move", ToolMove);
    bind!(out, ids, registry, "tool.marquee", ToolMarquee);
    bind!(out, ids, registry, "tool.lasso", ToolLasso);
    bind!(out, ids, registry, "tool.wand", ToolWand);
    bind!(out, ids, registry, "tool.crop", ToolCrop);
    bind!(out, ids, registry, "tool.brush", ToolBrush);
    bind!(out, ids, registry, "tool.eraser", ToolEraser);
    bind!(out, ids, registry, "tool.healing", ToolHealing);
    bind!(out, ids, registry, "tool.clone", ToolClone);
    bind!(out, ids, registry, "tool.smear", ToolSmear);
    bind!(out, ids, registry, "tool.gradient", ToolGradient);
    bind!(out, ids, registry, "tool.shape", ToolShape);
    bind!(out, ids, registry, "tool.type", ToolType);
    bind!(out, ids, registry, "tool.eyedropper", ToolEyedropper);
    bind!(out, ids, registry, "tool.hand", ToolHand);
    bind!(out, ids, registry, "tool.zoom", ToolZoom);
    // Palette
    bind!(out, ids, registry, "palette.swap", SwapColors);
    bind!(out, ids, registry, "palette.reset", DefaultColors);
    bind!(out, ids, registry, "tool.cycle-mode", CycleToolMode);
    // File
    bind!(out, ids, registry, "file.new", NewCanvas);
    bind!(out, ids, registry, "file.open", OpenDocument);
    bind!(out, ids, registry, "file.save", SaveDocument);
    bind!(out, ids, registry, "file.save-as", SaveAs);
    // Export PNG ⇧⌘E, Export JPEG ⌥⇧⌘S
    bind!(out, ids, registry, "file.export-png", ExportPng);
    bind!(out, ids, registry, "file.export-jpeg", ExportJpeg);
    bind!(out, ids, registry, "file.close", CloseDocument);
    // Edit
    bind!(out, ids, registry, "edit.undo", Undo);
    bind!(out, ids, registry, "edit.redo", Redo);
    bind!(out, ids, registry, "edit.free-transform", FreeTransform);
    // Clipboard (Cmd-X/C/V and Copy Merged Cmd-Shift-C)
    bind!(out, ids, registry, "edit.cut", Cut);
    bind!(out, ids, registry, "edit.copy", CopyPixels);
    bind!(out, ids, registry, "edit.copy-merged", CopyMerged);
    bind!(out, ids, registry, "edit.paste", PastePixels);
    // Photoshop's fill shortcuts: Option-Delete, Cmd-Delete, Shift-Delete
    bind!(out, ids, registry, "edit.fill-foreground", FillForeground);
    bind!(out, ids, registry, "edit.fill-background", FillBackground);
    bind!(out, ids, registry, "edit.content-aware-fill", ContentAwareFill);
    // Transform controls toggle (Cmd-H) and Invert pixels (Cmd-I)
    bind!(out, ids, registry, "view.transform-controls", ShowTransformControls);
    bind!(out, ids, registry, "image.invert", InvertPixels);
    // Curves (Cmd-M) and clipping mask toggle (Cmd-Alt-G)
    bind!(out, ids, registry, "image.curves", CurvesSheet);
    bind!(out, ids, registry, "layer.clipping", ToggleClippingMask);
    // Layer stacking (Cmd-[ / Cmd-])
    bind!(out, ids, registry, "layer.move-down", MoveLayerDown);
    bind!(out, ids, registry, "layer.move-up", MoveLayerUp);
    // Show guides ⌘;, snap master ⇧⌘;, lock guides ⌥⌘; (ctrl-shift-r is a
    // Rust alias of "Show Guides")
    bind!(out, ids, registry, "view.guides", ToggleGuides);
    bind!(out, ids, registry, "view.snap", SnapToggle);
    bind!(out, ids, registry, "view.lock-guides", LockGuides);
    // Image adjustments
    bind!(out, ids, registry, "image.levels", LevelsSheet);
    bind!(out, ids, registry, "image.hue-saturation", HueSaturationSheet);
    // Image
    bind!(out, ids, registry, "image.image-size", ImageSize);
    bind!(out, ids, registry, "image.canvas-size", CanvasSize);
    // Layer
    // New Blank Layer (Cmd-Shift-N); New Group stays available via the panel.
    bind!(out, ids, registry, "layer.new", NewLayer);
    bind!(out, ids, registry, "layer.duplicate", DuplicateLayer);
    bind!(out, ids, registry, "layer.merge", MergeDown);
    // Group Selected Layers (Cmd-G)
    bind!(out, ids, registry, "layer.group", GroupSelectedLayers);
    bind!(out, ids, registry, "layer.delete", DeleteLayer);
    // Select
    bind!(out, ids, registry, "select.all", SelectAllAction);
    bind!(out, ids, registry, "select.deselect", DeselectAction);
    bind!(out, ids, registry, "select.invert", InvertSelection);
    bind!(out, ids, registry, "select.feather", FeatherSheet);
    bind!(out, ids, registry, "select.expand", ExpandSheet);
    bind!(out, ids, registry, "select.contract", ContractSheet);
    // View
    bind!(out, ids, registry, "view.rulers", ToggleRulers);
    bind!(out, ids, registry, "view.grid", ToggleGrid);
    bind!(out, ids, registry, "view.zoom-in", ZoomIn);
    bind!(out, ids, registry, "view.zoom-out", ZoomOut);
    bind!(out, ids, registry, "view.zoom-fit", ZoomFit);
    bind!(out, ids, registry, "view.zoom-100", Zoom100);
    // Nudge
    bind!(out, ids, registry, "nudge.left", NudgeLeft);
    bind!(out, ids, registry, "nudge.right", NudgeRight);
    bind!(out, ids, registry, "nudge.up", NudgeUp);
    bind!(out, ids, registry, "nudge.down", NudgeDown);
    bind!(out, ids, registry, "nudge.left-10", NudgeLeftMore);
    bind!(out, ids, registry, "nudge.right-10", NudgeRightMore);
    bind!(out, ids, registry, "nudge.up-10", NudgeUpMore);
    bind!(out, ids, registry, "nudge.down-10", NudgeDownMore);
    // Brush size / hardness
    bind!(out, ids, registry, "brush.smaller", BrushSmaller);
    bind!(out, ids, registry, "brush.larger", BrushLarger);
    bind!(out, ids, registry, "brush.softer", BrushSofter);
    bind!(out, ids, registry, "brush.harder", BrushHarder);
    // Blend mode stepping and shape kind
    bind!(out, ids, registry, "blend.previous", BlendModePrevious);
    bind!(out, ids, registry, "blend.next", BlendModeNext);
    bind!(out, ids, registry, "shape.cycle", CycleShapeKind);
    // Levels preview toggle (Alt-P)
    bind!(out, ids, registry, "view.levels-preview", ToggleLevelsPreview);
    // Finish a polygonal lasso / cancel the active canvas edit or sheet.
    bind!(out, ids, registry, "canvas.apply", FinishLasso);
    bind!(out, ids, registry, "canvas.cancel", CancelAction);
    out
}
