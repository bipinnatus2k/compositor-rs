//! Actions and keyboard shortcuts (KeyboardShortcuts.swift).

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
pub fn bindings() -> Vec<KeyBinding> {
    vec![
        // Tools
        KeyBinding::new("v", ToolMove, None),
        KeyBinding::new("m", ToolMarquee, None),
        KeyBinding::new("l", ToolLasso, None),
        KeyBinding::new("w", ToolWand, None),
        KeyBinding::new("c", ToolCrop, None),
        KeyBinding::new("b", ToolBrush, None),
        KeyBinding::new("e", ToolEraser, None),
        KeyBinding::new("j", ToolHealing, None),
        KeyBinding::new("s", ToolClone, None),
        KeyBinding::new("r", ToolSmear, None),
        KeyBinding::new("g", ToolGradient, None),
        KeyBinding::new("u", ToolShape, None),
        KeyBinding::new("t", ToolType, None),
        KeyBinding::new("i", ToolEyedropper, None),
        KeyBinding::new("h", ToolHand, None),
        KeyBinding::new("z", ToolZoom, None),
        // Palette
        KeyBinding::new("x", SwapColors, None),
        KeyBinding::new("d", DefaultColors, None),
        KeyBinding::new("tab", CycleToolMode, Some("Compositor")),
        // File
        KeyBinding::new("ctrl-n", NewCanvas, None),
        KeyBinding::new("ctrl-o", OpenDocument, None),
        KeyBinding::new("ctrl-s", SaveDocument, None),
        KeyBinding::new("ctrl-shift-s", SaveAs, None),
        // Export PNG ⇧⌘E, Export JPEG ⌥⇧⌘S
        KeyBinding::new("ctrl-shift-e", ExportPng, None),
        KeyBinding::new("ctrl-alt-shift-s", ExportJpeg, None),
        KeyBinding::new("ctrl-w", CloseDocument, None),
        // Edit
        KeyBinding::new("ctrl-z", Undo, None),
        KeyBinding::new("ctrl-shift-z", Redo, None),
        KeyBinding::new("ctrl-t", FreeTransform, None),
        // Clipboard (Cmd-X/C/V and Copy Merged Cmd-Shift-C)
        KeyBinding::new("ctrl-x", Cut, None),
        KeyBinding::new("ctrl-c", CopyPixels, None),
        KeyBinding::new("ctrl-shift-c", CopyMerged, None),
        KeyBinding::new("ctrl-v", PastePixels, None),
        // Photoshop's fill shortcuts: Option-Delete, Cmd-Delete, Shift-Delete
        KeyBinding::new("alt-delete", FillForeground, None),
        KeyBinding::new("alt-backspace", FillForeground, None),
        KeyBinding::new("ctrl-delete", FillBackground, None),
        KeyBinding::new("ctrl-backspace", FillBackground, None),
        KeyBinding::new("shift-delete", ContentAwareFill, None),
        KeyBinding::new("shift-backspace", ContentAwareFill, None),
        // Transform controls toggle (Cmd-H) and Invert pixels (Cmd-I)
        KeyBinding::new("ctrl-h", ShowTransformControls, None),
        KeyBinding::new("ctrl-i", InvertPixels, None),
        // Curves (Cmd-M) and clipping mask toggle (Cmd-Alt-G)
        KeyBinding::new("ctrl-m", CurvesSheet, None),
        KeyBinding::new("alt-ctrl-g", ToggleClippingMask, None),
        // Layer stacking (Cmd-[ / Cmd-])
        KeyBinding::new("ctrl-[", MoveLayerDown, None),
        KeyBinding::new("ctrl-]", MoveLayerUp, None),
        // Show guides ⌘;, show grid ⌘', snap master ⇧⌘;, lock guides ⌥⌘;
        KeyBinding::new("ctrl-;", ToggleGuides, None),
        KeyBinding::new("ctrl-shift-;", SnapToggle, None),
        KeyBinding::new("alt-ctrl-;", LockGuides, None),
        // Image adjustments
        KeyBinding::new("ctrl-l", LevelsSheet, None),
        KeyBinding::new("ctrl-u", HueSaturationSheet, None),
        // Image
        KeyBinding::new("ctrl-alt-i", ImageSize, None),
        KeyBinding::new("ctrl-alt-c", CanvasSize, None),
        // Layer
        KeyBinding::new("ctrl-shift-n", NewLayer, None),
        KeyBinding::new("ctrl-j", DuplicateLayer, None),
        KeyBinding::new("ctrl-e", MergeDown, None),
        KeyBinding::new("ctrl-g", NewGroup, None),
        KeyBinding::new("delete", DeleteLayer, None),
        KeyBinding::new("backspace", DeleteLayer, None),
        // Select
        KeyBinding::new("ctrl-a", SelectAllAction, None),
        KeyBinding::new("ctrl-d", DeselectAction, None),
        KeyBinding::new("ctrl-shift-i", InvertSelection, None),
        KeyBinding::new("alt-ctrl-d", FeatherSheet, None),
        KeyBinding::new("alt-ctrl-e", ExpandSheet, None),
        KeyBinding::new("alt-ctrl-c", ContractSheet, None),
        // View
        KeyBinding::new("ctrl-r", ToggleRulers, None),
        KeyBinding::new("ctrl-'", ToggleGrid, None),
        KeyBinding::new("ctrl-shift-r", ToggleGuides, None),
        KeyBinding::new("ctrl-=", ZoomIn, None),
        KeyBinding::new("ctrl-+", ZoomIn, None),
        KeyBinding::new("ctrl--", ZoomOut, None),
        KeyBinding::new("ctrl-0", ZoomFit, None),
        KeyBinding::new("ctrl-1", Zoom100, None),
        // Nudge
        KeyBinding::new("left", NudgeLeft, None),
        KeyBinding::new("right", NudgeRight, None),
        KeyBinding::new("up", NudgeUp, None),
        KeyBinding::new("down", NudgeDown, None),
        KeyBinding::new("shift-left", NudgeLeftMore, None),
        KeyBinding::new("shift-right", NudgeRightMore, None),
        KeyBinding::new("shift-up", NudgeUpMore, None),
        KeyBinding::new("shift-down", NudgeDownMore, None),
        // Brush size
        KeyBinding::new("[", BrushSmaller, None),
        KeyBinding::new("]", BrushLarger, None),
        // Brush hardness (Swift Shift-[ / Shift-])
        KeyBinding::new("shift-[", BrushSofter, None),
        KeyBinding::new("shift-]", BrushHarder, None),
        // Blend mode stepping (Swift Shift-- / Shift-=)
        KeyBinding::new("shift--", BlendModePrevious, None),
        KeyBinding::new("shift-=", BlendModeNext, None),
        // Cycle shape kind (Swift Shift-U)
        KeyBinding::new("shift-u", CycleShapeKind, None),
        // Levels preview toggle (Swift Alt-P)
        KeyBinding::new("alt-p", ToggleLevelsPreview, None),
        // Finish a polygonal lasso
        KeyBinding::new("enter", FinishLasso, Some("Compositor")),
        // Escape cancels the active canvas edit or sheet.
        KeyBinding::new("escape", CancelAction, Some("Compositor")),
    ]
}
