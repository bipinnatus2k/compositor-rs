//! Actions and keyboard shortcuts (KeyboardShortcuts.swift).

use gpui_kit::KeyBinding;
use gpui_kit::actions;

actions!(compositor, [
    // Tools
    ToolMove, ToolMarquee, ToolLasso, ToolWand, ToolCrop, ToolBrush, ToolEraser,
    ToolHealing, ToolClone, ToolSmear, ToolGradient, ToolShape, ToolType,
    ToolEyedropper, ToolHand, ToolZoom,
    // Palette and tool modes
    SwapColors, DefaultColors, CycleToolMode,
    // File
    NewCanvas, OpenDocument, SaveDocument, ExportPng, CloseDocument,
    // Edit
    Undo, Redo, FillForeground, FillBackground, FreeTransform, ShowShortcuts,
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
    SnapGuides, SnapGrid, SnapLayers, SnapBounds,
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
        KeyBinding::new("ctrl-shift-s", ExportPng, None),
        // Edit
        KeyBinding::new("ctrl-z", Undo, None),
        KeyBinding::new("ctrl-shift-z", Redo, None),
        KeyBinding::new("ctrl-t", FreeTransform, None),
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
        // Finish a polygonal lasso
        KeyBinding::new("enter", FinishLasso, Some("Compositor")),
        // Escape cancels the active canvas edit or sheet.
        KeyBinding::new("escape", CancelAction, Some("Compositor")),
    ]
}
