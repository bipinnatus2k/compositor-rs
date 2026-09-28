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
    CloseDocument, Quit,
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
    ClearGuides,
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
    NudgePixelsLeft, NudgePixelsRight, NudgePixelsUp, NudgePixelsDown,
    NudgePixelsLeftMore, NudgePixelsRightMore, NudgePixelsUpMore, NudgePixelsDownMore,
    BrushSmaller, BrushLarger, FinishLasso, CancelAction,
    // Inline text editor spacing (CanvasTextView.keyDown's Option-arrows).
    // Their bindings carry no context, so they outrank the input's own
    // alt-arrow bindings (word move / extra cursors) whenever the inline
    // editor's overlay div sits on the dispatch path; elsewhere nothing
    // handles them and the next binding (the input's) still applies.
    InlineTextTrackingTighten, InlineTextTrackingLoosen,
    InlineTextTrackingTightenMore, InlineTextTrackingLoosenMore,
    InlineTextLeadingTighten, InlineTextLeadingLoosen,
    InlineTextLeadingTightenMore, InlineTextLeadingLoosenMore,
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

/// The action-name prefix of every binding this app generates
/// (`actions!(compositor, ...)`). The keymap rebuild keeps every binding
/// whose action does not carry this prefix — gpui-component's input, menu
/// and list navigation bindings, which [`gpui_kit::init`] installed once at
/// startup and a plain [`gpui_kit::App::clear_key_bindings`] would lose.
const APP_ACTION_PREFIX: &str = "compositor::";

/// Whether a binding's action is one of this app's own (as opposed to the
/// component baseline shipped by gpui-kit).
pub(crate) fn is_app_action(action_name: &str) -> bool {
    action_name.starts_with(APP_ACTION_PREFIX)
}

/// Splits a keymap snapshot into `(component baseline, app bindings)`. The
/// rebuild replays the baseline verbatim and regenerates the app half from
/// the shortcut registry, so custom chords replace — never duplicate — the
/// previous app bindings.
pub(crate) fn split_bindings<'a>(
    bindings: impl IntoIterator<Item = &'a KeyBinding>,
) -> (Vec<KeyBinding>, Vec<KeyBinding>) {
    let (baseline, app): (Vec<&KeyBinding>, Vec<&KeyBinding>) = bindings
        .into_iter()
        .partition(|binding| !is_app_action(binding.action().name()));
    (
        baseline.into_iter().cloned().collect(),
        app.into_iter().cloned().collect(),
    )
}

/// Rebuilds the app keymap in place from the live shortcut registry: the
/// component baseline is snapshotted, everything is cleared, the baseline is
/// re-added and [`bindings_with`] regenerates the app's bindings. Called
/// after the shortcuts panel saves (Done) or restores defaults, so a changed
/// chord takes effect on the next keystroke without a restart.
pub fn rebuild_bindings(cx: &mut gpui_kit::App) {
    let registry = crate::shortcuts::lock_global();
    rebuild_with(cx, &registry);
}

/// [`rebuild_bindings`] against an explicit registry (tests and callers that
/// already hold the lock).
pub(crate) fn rebuild_with(
    cx: &mut gpui_kit::App,
    registry: &crate::shortcuts::ShortcutRegistry,
) {
    // Snapshot before clearing: the keymap also holds gpui-kit's component
    // bindings, which nothing re-installs automatically.
    let (baseline, _) = split_bindings(
        cx.key_bindings()
            .borrow()
            .bindings()
            .cloned()
            .collect::<Vec<_>>()
            .iter(),
    );
    cx.clear_key_bindings();
    cx.bind_keys(baseline);
    cx.bind_keys(bindings_with(registry));
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
    bind!(out, ids, registry, "file.quit", Quit);
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
    // Cmd-arrow moves the selected pixels (SelectionEdits.swift:141).
    bind!(out, ids, registry, "nudge.pixels-left", NudgePixelsLeft);
    bind!(out, ids, registry, "nudge.pixels-right", NudgePixelsRight);
    bind!(out, ids, registry, "nudge.pixels-up", NudgePixelsUp);
    bind!(out, ids, registry, "nudge.pixels-down", NudgePixelsDown);
    bind!(out, ids, registry, "nudge.pixels-left-10", NudgePixelsLeftMore);
    bind!(out, ids, registry, "nudge.pixels-right-10", NudgePixelsRightMore);
    bind!(out, ids, registry, "nudge.pixels-up-10", NudgePixelsUpMore);
    bind!(out, ids, registry, "nudge.pixels-down-10", NudgePixelsDownMore);
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
    out.extend(inline_text_bindings());
    out
}

/// The inline editor's Option-arrow spacing chords, bound without a context
/// on purpose. gpui ranks bindings by the depth of the context they match
/// and then by insertion order; a context-less binding matches the full
/// stack, so these outrank gpui-base's input bindings (`alt-left` moves by
/// word, `shift-alt-up` adds a cursor) which would otherwise consume the
/// keystroke before the canvas's bubble-phase key listener ever ran —
/// starving tracking/leading while the inline editor is focused
/// (`CanvasTextView.keyDown`, InlineTextEditor.swift:12-22). When the focus
/// is elsewhere no handler is on the dispatch path, the binding falls
/// through, and the input keeps its own word movement.
pub(crate) fn inline_text_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("alt-left", InlineTextTrackingTighten, None),
        KeyBinding::new("alt-right", InlineTextTrackingLoosen, None),
        KeyBinding::new("alt-up", InlineTextLeadingTighten, None),
        KeyBinding::new("alt-down", InlineTextLeadingLoosen, None),
        // Shift makes each step ten, as in the Swift editor.
        KeyBinding::new("alt-shift-left", InlineTextTrackingTightenMore, None),
        KeyBinding::new("alt-shift-right", InlineTextTrackingLoosenMore, None),
        KeyBinding::new("alt-shift-up", InlineTextLeadingTightenMore, None),
        KeyBinding::new("alt-shift-down", InlineTextLeadingLoosenMore, None),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shortcuts::ShortcutRegistry;

    /// The (keystrokes, action, context) identity of one binding, for
    /// order-sensitive comparisons.
    fn identity(binding: &KeyBinding) -> (String, &'static str, Option<String>) {
        let keystrokes = binding
            .keystrokes()
            .iter()
            .map(|key| key.inner().unparse())
            .collect::<Vec<_>>()
            .join(" ");
        (
            keystrokes,
            binding.action().name(),
            binding.predicate().map(|p| p.to_string()),
        )
    }

    /// The inline spacing chords are bound without a context (the full
    /// context stack matches, so they outrank the input's `Input`-scoped
    /// alt-arrow bindings), every step chord is present, and none of them
    /// collides with a registry chord.
    #[test]
    fn inline_text_spacing_bindings_shadow_the_input() {
        let registry = ShortcutRegistry::new();
        let registry_chords: Vec<String> = bindings_with(&registry)
            .iter()
            .filter(|b| {
                !b.action()
                    .name()
                    .starts_with("compositor::InlineTextTracking")
                    && !b.action().name().starts_with("compositor::InlineTextLeading")
            })
            .map(|b| {
                b.keystrokes()
                    .iter()
                    .map(|k| k.inner().unparse())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        let bindings = inline_text_bindings();
        assert_eq!(bindings.len(), 8);
        let expected = [
            ("alt-left", "InlineTextTrackingTighten"),
            ("alt-right", "InlineTextTrackingLoosen"),
            ("alt-up", "InlineTextLeadingTighten"),
            ("alt-down", "InlineTextLeadingLoosen"),
            ("alt-shift-left", "InlineTextTrackingTightenMore"),
            ("alt-shift-right", "InlineTextTrackingLoosenMore"),
            ("alt-shift-up", "InlineTextLeadingTightenMore"),
            ("alt-shift-down", "InlineTextLeadingLoosenMore"),
        ];
        for (keystroke, action) in expected {
            let binding = bindings
                .iter()
                .find(|b| b.action().name() == &format!("compositor::{action}"))
                .unwrap_or_else(|| panic!("missing binding for {action}"));
            assert_eq!(
                binding.keystrokes()[0].inner().unparse(),
                keystroke,
                "{action}"
            );
            // No context: the binding matches the full context stack, which
            // is what ranks it above the input's own alt-arrow bindings.
            assert!(binding.predicate().is_none(), "{action}");
            assert!(
                !registry_chords.contains(&keystroke.to_string()),
                "{keystroke} is also a registry chord"
            );
        }
    }

    /// `rebuild_with`'s precondition: regenerating the app bindings from the
    /// same registry always produces the same table, so a clear + replay
    /// cannot drift.
    #[test]
    fn bindings_with_is_idempotent() {
        let mut registry = ShortcutRegistry::new();
        registry.set_override("edit.undo", "ctrl-alt-z").unwrap();
        registry.set_override("tool.brush", "f").unwrap();

        let first = bindings_with(&registry);
        let second = bindings_with(&registry);
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(identity(a), identity(b));
        }
    }

    /// The rebuild replays the app half wholesale, so a duplicate binding
    /// (same action bound twice to the same chord) would shadow itself.
    #[test]
    fn bindings_have_no_duplicate_action_key_pairs() {
        let registry = ShortcutRegistry::new();
        let bindings = bindings_with(&registry);
        let mut seen: Vec<(String, &'static str)> = Vec::new();
        for binding in &bindings {
            let (keystrokes, action, _) = identity(binding);
            assert!(
                !seen.contains(&(keystrokes.clone(), action)),
                "duplicate binding: {action} <- {keystrokes}"
            );
            seen.push((keystrokes, action));
        }
    }

    /// The rebuild keeps only the component baseline and regenerates every
    /// `compositor::` binding: the split must separate the two sets without
    /// losing either.
    #[test]
    fn split_bindings_separates_app_from_baseline() {
        use gpui_kit::NoAction;
        let registry = ShortcutRegistry::new();
        let app_bindings = bindings_with(&registry);
        // A stand-in for gpui-kit's component baseline (any non-`compositor::`
        // action), as the keymap would hold it after startup.
        let baseline = vec![
            KeyBinding::new("up", NoAction, None),
            KeyBinding::new("down", NoAction, Some("Input")),
        ];
        let mut mixed: Vec<KeyBinding> = baseline.clone();
        mixed.extend(app_bindings.iter().cloned());

        let (kept_baseline, regenerated) = split_bindings(mixed.iter());
        assert_eq!(kept_baseline.len(), baseline.len());
        assert_eq!(regenerated.len(), app_bindings.len());
        for (a, b) in kept_baseline.iter().zip(baseline.iter()) {
            assert_eq!(identity(a), identity(b));
        }
        for (a, b) in regenerated.iter().zip(app_bindings.iter()) {
            assert_eq!(identity(a), identity(b));
        }
        // Every regenerated binding is app-owned; every kept one is not.
        assert!(regenerated.iter().all(|b| is_app_action(b.action().name())));
        assert!(kept_baseline
            .iter()
            .all(|b| !is_app_action(b.action().name())));
    }
}
