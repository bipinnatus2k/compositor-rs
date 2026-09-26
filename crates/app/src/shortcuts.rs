//! The shortcut customization system (KeyboardShortcuts.swift:129-319).
//!
//! Port of Swift's `ShortcutSettings` singleton plus its `ShortcutDefinition`
//! catalog: every customizable action has a stable id, a display title, a
//! default chord and an optional user override. Overrides persist as JSON in
//! the platform config directory and are applied when `keys::bindings()`
//! regenerates the app keymap at startup (the menu bar's key hints read that
//! keymap, so they follow custom chords automatically).
//!
//! Modeling notes (KeyboardShortcuts.swift):
//! - Swift keys every definition by `"\(group):\(title)"`; here each spec has
//!   a dot-separated `id` that survives renames and stays ASCII-stable.
//! - Swift allows exactly one chord per action. A few actions here ship two
//!   default chords that are plain aliases (`delete`/`backspace`); assigning
//!   an override replaces the whole alias set, matching the Swift model.
//! - Swift's `problem(in:)` validation (single key, text-editing modifier
//!   rule, reserved macOS chords, duplicate assignment) is ported to
//!   [`ShortcutProblem`]. The reserved chords follow this port's ⌘→`ctrl`
//!   mapping convention used by every default binding.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use gpui_kit::Keystroke;
use serde::{Deserialize, Serialize};

/// Shortcut groups, mirroring `ShortcutDefinition`'s groups in
/// KeyboardShortcuts.swift ("Menus", "Canvas & Layers", "Text Editing").
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Group {
    Menus,
    CanvasLayers,
    TextEditing,
}

impl Group {
    /// The Swift group label (KeyboardShortcutsSheet lists these verbatim).
    pub fn label(self) -> &'static str {
        match self {
            Group::Menus => "Menus",
            Group::CanvasLayers => "Canvas & Layers",
            Group::TextEditing => "Text Editing",
        }
    }
}

/// One customizable shortcut slot (`ShortcutDefinition` in Swift).
#[derive(Clone, Copy, Debug)]
pub struct ShortcutSpec {
    /// Stable identifier used for persistence (never renamed).
    pub id: &'static str,
    /// Display title, aligned with Swift's action names where they exist.
    pub title: &'static str,
    pub group: Group,
    /// gpui key-context predicate, e.g. `Some("Compositor")` for keys that
    /// only fire while the workspace canvas owns the keyboard.
    pub context: Option<&'static str>,
    /// Default keystroke(s) in gpui format. Extra entries are aliases of the
    /// same action (e.g. `delete` and `backspace` both delete); a user
    /// override replaces all of them.
    pub defaults: &'static [&'static str],
}

/// Every customizable shortcut, in the same order as `keys::bindings_with`
/// (a test enforces that the two tables stay in lockstep).
pub const CATALOG: &[ShortcutSpec] = &[
    // Tools (Swift group "Canvas & Layers", titles from ShortcutDefinition.all).
    ShortcutSpec { id: "tool.move", title: "Move / Transform tool", group: Group::CanvasLayers, context: None, defaults: &["v"] },
    ShortcutSpec { id: "tool.marquee", title: "Marquee / cycle shape", group: Group::CanvasLayers, context: None, defaults: &["m"] },
    ShortcutSpec { id: "tool.lasso", title: "Lasso / cycle mode", group: Group::CanvasLayers, context: None, defaults: &["l"] },
    ShortcutSpec { id: "tool.wand", title: "Magic Wand", group: Group::CanvasLayers, context: None, defaults: &["w"] },
    ShortcutSpec { id: "tool.crop", title: "Crop tool", group: Group::CanvasLayers, context: None, defaults: &["c"] },
    ShortcutSpec { id: "tool.brush", title: "Brush tool", group: Group::CanvasLayers, context: None, defaults: &["b"] },
    ShortcutSpec { id: "tool.eraser", title: "Eraser", group: Group::CanvasLayers, context: None, defaults: &["e"] },
    ShortcutSpec { id: "tool.healing", title: "Spot Healing", group: Group::CanvasLayers, context: None, defaults: &["j"] },
    ShortcutSpec { id: "tool.clone", title: "Clone Stamp", group: Group::CanvasLayers, context: None, defaults: &["s"] },
    ShortcutSpec { id: "tool.smear", title: "Blur / Smudge / Liquify", group: Group::CanvasLayers, context: None, defaults: &["r"] },
    ShortcutSpec { id: "tool.gradient", title: "Gradient tool", group: Group::CanvasLayers, context: None, defaults: &["g"] },
    ShortcutSpec { id: "tool.shape", title: "Shape tool", group: Group::CanvasLayers, context: None, defaults: &["u"] },
    ShortcutSpec { id: "tool.type", title: "Type tool", group: Group::CanvasLayers, context: None, defaults: &["t"] },
    ShortcutSpec { id: "tool.eyedropper", title: "Eyedropper tool", group: Group::CanvasLayers, context: None, defaults: &["i"] },
    ShortcutSpec { id: "tool.hand", title: "Hand tool", group: Group::CanvasLayers, context: None, defaults: &["h"] },
    ShortcutSpec { id: "tool.zoom", title: "Zoom tool", group: Group::CanvasLayers, context: None, defaults: &["z"] },
    // Palette
    ShortcutSpec { id: "palette.swap", title: "Swap foreground/background", group: Group::CanvasLayers, context: None, defaults: &["x"] },
    ShortcutSpec { id: "palette.reset", title: "Reset colors", group: Group::CanvasLayers, context: None, defaults: &["d"] },
    ShortcutSpec { id: "tool.cycle-mode", title: "Cycle tool mode", group: Group::CanvasLayers, context: Some("Compositor"), defaults: &["tab"] },
    // File
    ShortcutSpec { id: "file.new", title: "New Canvas", group: Group::Menus, context: None, defaults: &["ctrl-n"] },
    ShortcutSpec { id: "file.open", title: "Open Project", group: Group::Menus, context: None, defaults: &["ctrl-o"] },
    ShortcutSpec { id: "file.save", title: "Save", group: Group::Menus, context: None, defaults: &["ctrl-s"] },
    ShortcutSpec { id: "file.save-as", title: "Save As", group: Group::Menus, context: None, defaults: &["ctrl-shift-s"] },
    ShortcutSpec { id: "file.export-png", title: "Export PNG", group: Group::Menus, context: None, defaults: &["ctrl-shift-e"] },
    ShortcutSpec { id: "file.export-jpeg", title: "Export JPEG", group: Group::Menus, context: None, defaults: &["ctrl-alt-shift-s"] },
    ShortcutSpec { id: "file.close", title: "Close Project", group: Group::Menus, context: None, defaults: &["ctrl-w"] },
    // Edit
    ShortcutSpec { id: "edit.undo", title: "Undo", group: Group::Menus, context: None, defaults: &["ctrl-z"] },
    ShortcutSpec { id: "edit.redo", title: "Redo", group: Group::Menus, context: None, defaults: &["ctrl-shift-z"] },
    ShortcutSpec { id: "edit.free-transform", title: "Transform Layer / Selection", group: Group::Menus, context: None, defaults: &["ctrl-t"] },
    ShortcutSpec { id: "edit.cut", title: "Cut", group: Group::Menus, context: None, defaults: &["ctrl-x"] },
    ShortcutSpec { id: "edit.copy", title: "Copy", group: Group::Menus, context: None, defaults: &["ctrl-c"] },
    ShortcutSpec { id: "edit.copy-merged", title: "Copy Merged", group: Group::Menus, context: None, defaults: &["ctrl-shift-c"] },
    ShortcutSpec { id: "edit.paste", title: "Paste", group: Group::Menus, context: None, defaults: &["ctrl-v"] },
    // Photoshop's fill shortcuts: Option-Delete, Cmd-Delete, Shift-Delete
    // (the backspace variants are aliases of the same action).
    ShortcutSpec { id: "edit.fill-foreground", title: "Fill with Foreground", group: Group::Menus, context: None, defaults: &["alt-delete", "alt-backspace"] },
    ShortcutSpec { id: "edit.fill-background", title: "Fill with Background", group: Group::Menus, context: None, defaults: &["ctrl-delete", "ctrl-backspace"] },
    ShortcutSpec { id: "edit.content-aware-fill", title: "Content-Aware Fill", group: Group::Menus, context: None, defaults: &["shift-delete", "shift-backspace"] },
    ShortcutSpec { id: "view.transform-controls", title: "Show Transform Controls", group: Group::Menus, context: None, defaults: &["ctrl-h"] },
    ShortcutSpec { id: "image.invert", title: "Invert Pixels / Mask", group: Group::Menus, context: None, defaults: &["ctrl-i"] },
    ShortcutSpec { id: "image.curves", title: "Curves", group: Group::Menus, context: None, defaults: &["ctrl-m"] },
    ShortcutSpec { id: "layer.clipping", title: "Toggle Clipping Mask", group: Group::Menus, context: None, defaults: &["ctrl-alt-g"] },
    ShortcutSpec { id: "layer.move-down", title: "Move Layer Down", group: Group::Menus, context: None, defaults: &["ctrl-["] },
    ShortcutSpec { id: "layer.move-up", title: "Move Layer Up", group: Group::Menus, context: None, defaults: &["ctrl-]"] },
    // Guides / grid / snapping. `ctrl-shift-r` is a Rust extra alias of
    // "Show Guides" (Swift only binds ⌘;).
    ShortcutSpec { id: "view.guides", title: "Show Guides", group: Group::Menus, context: None, defaults: &["ctrl-;", "ctrl-shift-r"] },
    ShortcutSpec { id: "view.snap", title: "Snap", group: Group::Menus, context: None, defaults: &["ctrl-shift-;"] },
    ShortcutSpec { id: "view.lock-guides", title: "Lock Guides", group: Group::Menus, context: None, defaults: &["ctrl-alt-;"] },
    ShortcutSpec { id: "image.levels", title: "Levels", group: Group::Menus, context: None, defaults: &["ctrl-l"] },
    ShortcutSpec { id: "image.hue-saturation", title: "Hue/Saturation", group: Group::Menus, context: None, defaults: &["ctrl-u"] },
    // Image
    ShortcutSpec { id: "image.image-size", title: "Image Size", group: Group::Menus, context: None, defaults: &["ctrl-alt-i"] },
    ShortcutSpec { id: "image.canvas-size", title: "Canvas Size", group: Group::Menus, context: None, defaults: &["ctrl-alt-c"] },
    // Layer
    ShortcutSpec { id: "layer.new", title: "New Blank Layer", group: Group::Menus, context: None, defaults: &["ctrl-shift-n"] },
    ShortcutSpec { id: "layer.duplicate", title: "Duplicate / Layer via Copy", group: Group::Menus, context: None, defaults: &["ctrl-j"] },
    ShortcutSpec { id: "layer.merge", title: "Merge Layers", group: Group::Menus, context: None, defaults: &["ctrl-e"] },
    ShortcutSpec { id: "layer.group", title: "Group Layers", group: Group::Menus, context: None, defaults: &["ctrl-g"] },
    ShortcutSpec {
        id: "layer.delete",
        title: "Delete selection / layer / effect / lasso point",
        group: Group::CanvasLayers,
        context: None,
        defaults: &["delete", "backspace"],
    },
    // Select
    ShortcutSpec { id: "select.all", title: "Select All", group: Group::Menus, context: None, defaults: &["ctrl-a"] },
    ShortcutSpec { id: "select.deselect", title: "Deselect", group: Group::Menus, context: None, defaults: &["ctrl-d"] },
    ShortcutSpec { id: "select.invert", title: "Inverse Selection", group: Group::Menus, context: None, defaults: &["ctrl-shift-i"] },
    ShortcutSpec { id: "select.feather", title: "Feather", group: Group::Menus, context: None, defaults: &["ctrl-alt-d"] },
    ShortcutSpec { id: "select.expand", title: "Expand", group: Group::Menus, context: None, defaults: &["ctrl-alt-e"] },
    // Contract used to be bound as `alt-ctrl-c`, which parses to the same
    // chord as Canvas Size's `ctrl-alt-c` (gpui modifiers are
    // order-insensitive) — one of the two was dead in the keymap and the
    // Swift conflict check rejected the default table. Shift disambiguates.
    ShortcutSpec { id: "select.contract", title: "Contract", group: Group::Menus, context: None, defaults: &["ctrl-alt-shift-c"] },
    // View
    ShortcutSpec { id: "view.rulers", title: "Show Rulers", group: Group::Menus, context: None, defaults: &["ctrl-r"] },
    ShortcutSpec { id: "view.grid", title: "Show Grid", group: Group::Menus, context: None, defaults: &["ctrl-'"] },
    ShortcutSpec { id: "view.zoom-in", title: "Zoom In", group: Group::Menus, context: None, defaults: &["ctrl-=", "ctrl-+"] },
    ShortcutSpec { id: "view.zoom-out", title: "Zoom Out", group: Group::Menus, context: None, defaults: &["ctrl--"] },
    ShortcutSpec { id: "view.zoom-fit", title: "Fit Canvas", group: Group::Menus, context: None, defaults: &["ctrl-0"] },
    ShortcutSpec { id: "view.zoom-100", title: "Actual Pixels", group: Group::Menus, context: None, defaults: &["ctrl-1"] },
    // Canvas keys
    ShortcutSpec { id: "nudge.left", title: "Nudge Left 1 px", group: Group::CanvasLayers, context: None, defaults: &["left"] },
    ShortcutSpec { id: "nudge.right", title: "Nudge Right 1 px", group: Group::CanvasLayers, context: None, defaults: &["right"] },
    ShortcutSpec { id: "nudge.up", title: "Nudge Up 1 px", group: Group::CanvasLayers, context: None, defaults: &["up"] },
    ShortcutSpec { id: "nudge.down", title: "Nudge Down 1 px", group: Group::CanvasLayers, context: None, defaults: &["down"] },
    ShortcutSpec { id: "nudge.left-10", title: "Nudge Left 10 px", group: Group::CanvasLayers, context: None, defaults: &["shift-left"] },
    ShortcutSpec { id: "nudge.right-10", title: "Nudge Right 10 px", group: Group::CanvasLayers, context: None, defaults: &["shift-right"] },
    ShortcutSpec { id: "nudge.up-10", title: "Nudge Up 10 px", group: Group::CanvasLayers, context: None, defaults: &["shift-up"] },
    ShortcutSpec { id: "nudge.down-10", title: "Nudge Down 10 px", group: Group::CanvasLayers, context: None, defaults: &["shift-down"] },
    ShortcutSpec { id: "brush.smaller", title: "Decrease brush size", group: Group::CanvasLayers, context: None, defaults: &["["] },
    ShortcutSpec { id: "brush.larger", title: "Increase brush size", group: Group::CanvasLayers, context: None, defaults: &["]"] },
    ShortcutSpec { id: "brush.softer", title: "Decrease brush hardness", group: Group::CanvasLayers, context: None, defaults: &["shift-["] },
    ShortcutSpec { id: "brush.harder", title: "Increase brush hardness", group: Group::CanvasLayers, context: None, defaults: &["shift-]"] },
    ShortcutSpec { id: "blend.previous", title: "Previous blend mode", group: Group::CanvasLayers, context: None, defaults: &["shift--"] },
    ShortcutSpec { id: "blend.next", title: "Next blend mode", group: Group::CanvasLayers, context: None, defaults: &["shift-="] },
    ShortcutSpec { id: "shape.cycle", title: "Cycle shape kind", group: Group::CanvasLayers, context: None, defaults: &["shift-u"] },
    ShortcutSpec { id: "view.levels-preview", title: "Toggle Levels preview", group: Group::CanvasLayers, context: None, defaults: &["alt-p"] },
    ShortcutSpec { id: "canvas.apply", title: "Apply current canvas operation", group: Group::CanvasLayers, context: Some("Compositor"), defaults: &["enter"] },
    ShortcutSpec { id: "canvas.cancel", title: "Cancel current canvas operation", group: Group::CanvasLayers, context: Some("Compositor"), defaults: &["escape"] },
];

/// Looks up a catalog entry by id.
pub fn spec(id: &str) -> Option<&'static ShortcutSpec> {
    CATALOG.iter().find(|entry| entry.id == id)
}

/// All catalog entries of one group, in catalog order (the settings sheet
/// lists them per group like KeyboardShortcutsSheet does).
pub fn entries_in_group(group: Group) -> impl Iterator<Item = &'static ShortcutSpec> {
    CATALOG.iter().filter(move |entry| entry.group == group)
}

/// Chords reserved by the system and therefore rejected as overrides
/// (KeyboardShortcuts.swift:173: ⌘Q quit, ⌘, settings, ⌃⌘M). The strings use
/// this port's ⌘→`ctrl` binding convention.
const RESERVED: &[&str] = &["ctrl-q", "ctrl-,", "ctrl-alt-m"];

/// The JSON document persisted on disk (`keyboardShortcuts.v1` in Swift).
#[derive(Serialize, Deserialize)]
struct StoredShortcuts {
    version: u32,
    overrides: BTreeMap<String, String>,
}

const STORAGE_VERSION: u32 = 1;

/// A validation failure, porting `ShortcutSettings.problem(in:)`. The
/// `Display` strings mirror the Swift messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShortcutProblem {
    /// The override names an action that is not in the catalog.
    UnknownAction(String),
    /// Not a single parseable key chord ("Choose a single key with optional
    /// modifiers.").
    InvalidChord { action: String, keystroke: String },
    /// Text-editing chords need Ctrl/Alt so they do not replace normal
    /// typing ("Text-editing shortcuts need Command, Option, or Control...").
    TextEditingNeedsModifier { action: String, keystroke: String },
    /// The chord is reserved by the platform ("... is reserved by macOS.").
    Reserved(String),
    /// The same chord is assigned to two actions ("... is assigned to both
    /// ... and ...").
    Conflict { chord: String, first: String, second: String },
}

impl fmt::Display for ShortcutProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShortcutProblem::UnknownAction(id) => {
                write!(f, "\"{id}\" is not a known shortcut.")
            }
            ShortcutProblem::InvalidChord { keystroke, .. } => write!(
                f,
                "\"{keystroke}\" is not a single key with optional modifiers."
            ),
            ShortcutProblem::TextEditingNeedsModifier { keystroke, .. } => write!(
                f,
                "Text-editing shortcuts need Ctrl or Alt so they do not replace normal typing (got \"{keystroke}\")."
            ),
            ShortcutProblem::Reserved(chord) => write!(f, "{chord} is reserved by the system."),
            ShortcutProblem::Conflict { chord, first, second } => {
                write!(f, "{chord} is assigned to both {first} and {second}.")
            }
        }
    }
}

/// Why a save failed (Swift's `save(_:)` silently refuses invalid drafts;
/// here the refusal is explicit).
#[derive(Debug)]
pub enum ShortcutError {
    Invalid(ShortcutProblem),
    Io(io::Error),
}

impl fmt::Display for ShortcutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShortcutError::Invalid(problem) => write!(f, "{problem}"),
            ShortcutError::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ShortcutError {}

impl From<io::Error> for ShortcutError {
    fn from(error: io::Error) -> Self {
        ShortcutError::Io(error)
    }
}

/// A duplicate chord assignment found by [`ShortcutRegistry::conflicts`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutConflict {
    /// Canonical keystroke string, e.g. `"ctrl-shift-s"`.
    pub chord: String,
    /// Human-readable label, e.g. `"Ctrl+Shift+S"`.
    pub label: String,
    /// Catalog ids holding this chord, in catalog order.
    pub actions: Vec<&'static str>,
}

/// The user's shortcut table: catalog defaults plus per-action overrides,
/// persisted as JSON (`ShortcutSettings` in Swift, which stores the same
/// shape in UserDefaults under `keyboardShortcuts.v1`).
///
/// Overrides are kept normalized (parsed and re-unparsed through gpui's
/// `Keystroke`) so persistence and conflict detection compare canonically.
#[derive(Debug, Default)]
pub struct ShortcutRegistry {
    /// Action id → normalized override chord. Absent ids use their defaults.
    overrides: BTreeMap<String, String>,
    /// Persistence target; `None` keeps the registry in memory (tests).
    path: Option<PathBuf>,
}

impl ShortcutRegistry {
    /// An empty registry with no persistence target (all catalog defaults).
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry bound to a JSON file. Does not read it; call [`load`]
    /// (or [`load_or_default`]) first. `path` is a constructor parameter so
    /// tests can inject a temporary directory.
    ///
    /// [`load`]: Self::load
    /// [`load_or_default`]: Self::load_or_default
    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        ShortcutRegistry { path: Some(path.into()), ..Self::default() }
    }

    /// The registry's persistence target, if any.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    // MARK: Reading

    /// The effective keystrokes for one action: the override chord if set,
    /// otherwise every default alias (`ShortcutSettings.chord(_:)`).
    ///
    /// Panics on an unknown id — a programming error caught by tests.
    pub fn keystrokes_for(&self, id: &str) -> Vec<String> {
        match self.overrides.get(id) {
            Some(chord) => vec![chord.clone()],
            None => {
                let entry = spec(id).unwrap_or_else(|| panic!("unknown shortcut id: {id}"));
                entry.defaults.iter().map(|key| key.to_string()).collect()
            }
        }
    }

    /// The primary chord for one action, for display.
    pub fn chord(&self, id: &str) -> String {
        self.keystrokes_for(id).first().cloned().unwrap_or_default()
    }

    /// The key-context predicate for one action (from the catalog).
    pub fn context_for(&self, id: &str) -> Option<&'static str> {
        spec(id).unwrap_or_else(|| panic!("unknown shortcut id: {id}")).context
    }

    /// The user override for one action, if any.
    pub fn override_for(&self, id: &str) -> Option<&str> {
        self.overrides.get(id).map(String::as_str)
    }

    /// Whether the action differs from its default chord.
    pub fn is_customized(&self, id: &str) -> bool {
        self.overrides.contains_key(id)
    }

    /// All overrides (id → chord). The settings sheet copies this as its
    /// draft, like `KeyboardShortcutsSheet(settings:)` does in Swift.
    pub fn overrides(&self) -> &BTreeMap<String, String> {
        &self.overrides
    }

    /// Effective chord table for every catalog entry.
    pub fn effective(&self) -> BTreeMap<&'static str, Vec<String>> {
        CATALOG
            .iter()
            .map(|entry| (entry.id, self.keystrokes_for(entry.id)))
            .collect()
    }

    // MARK: Editing

    /// Assigns a new chord to an action (`draft[id] = chord` in Swift).
    /// The chord is normalized before storage; setting an action's own
    /// default stores an explicit override on purpose so the sheet can show
    /// "customized" state until it is reset.
    pub fn set_override(&mut self, id: &str, keystroke: &str) -> Result<(), ShortcutProblem> {
        let entry = spec(id).ok_or_else(|| ShortcutProblem::UnknownAction(id.to_string()))?;
        let chord = normalize(keystroke).map_err(|_| ShortcutProblem::InvalidChord {
            action: entry.title.to_string(),
            keystroke: keystroke.to_string(),
        })?;
        self.overrides.insert(id.to_string(), chord.unparse());
        Ok(())
    }

    /// Resets one action to its default chord (`Save` with an empty draft in
    /// Swift; the sheet's per-row reset). Returns whether an override was
    /// removed.
    pub fn reset(&mut self, id: &str) -> bool {
        self.overrides.remove(id).is_some()
    }

    /// Restores every default chord ("Restore Defaults" button).
    pub fn reset_all(&mut self) {
        self.overrides.clear();
    }

    // MARK: Validation (Swift's `ShortcutSettings.problem(in:)`)

    /// Validates the full effective table: overrides must name known actions
    /// with a single parseable chord, text-editing chords must keep a
    /// non-typing modifier, reserved chords are rejected, and no two actions
    /// may share a chord.
    pub fn validate(&self) -> Result<(), ShortcutProblem> {
        for (id, keystroke) in &self.overrides {
            let entry = spec(id)
                .ok_or_else(|| ShortcutProblem::UnknownAction(id.clone()))?;
            let chord = normalize(keystroke).map_err(|_| ShortcutProblem::InvalidChord {
                action: entry.title.to_string(),
                keystroke: keystroke.clone(),
            })?;
            if entry.group == Group::TextEditing
                && !(chord.modifiers.control || chord.modifiers.alt || chord.modifiers.platform)
            {
                return Err(ShortcutProblem::TextEditingNeedsModifier {
                    action: entry.title.to_string(),
                    keystroke: chord.unparse(),
                });
            }
            if RESERVED.contains(&chord.unparse().as_str()) {
                return Err(ShortcutProblem::Reserved(chord_label(&chord)));
            }
        }
        // Duplicate assignment across the whole effective table (defaults
        // included), exactly like Swift's `assigned` map.
        let mut assigned: HashMap<Keystroke, &'static str> = HashMap::new();
        for entry in CATALOG {
            for keystroke in self.keystrokes_for(entry.id) {
                let chord = normalize(&keystroke).map_err(|_| ShortcutProblem::InvalidChord {
                    action: entry.title.to_string(),
                    keystroke,
                })?;
                if let Some(&first_id) = assigned.get(&chord) {
                    let first = spec(first_id).unwrap();
                    return Err(ShortcutProblem::Conflict {
                        chord: chord.unparse(),
                        first: first.title.to_string(),
                        second: entry.title.to_string(),
                    });
                }
                assigned.insert(chord, entry.id);
            }
        }
        Ok(())
    }

    /// Every duplicate chord assignment in the effective table. Unlike
    /// [`validate`] (which stops at the first problem, like Swift), this
    /// lists all conflicts so a settings sheet can flag each row.
    pub fn conflicts(&self) -> Vec<ShortcutConflict> {
        let mut by_chord: HashMap<Keystroke, Vec<&'static str>> = HashMap::new();
        for entry in CATALOG {
            for keystroke in self.keystrokes_for(entry.id) {
                if let Ok(chord) = normalize(&keystroke) {
                    by_chord.entry(chord).or_default().push(entry.id);
                }
            }
        }
        let mut conflicts: Vec<ShortcutConflict> = by_chord
            .into_iter()
            .filter(|(_, ids)| ids.len() > 1)
            .map(|(chord, mut actions)| ShortcutConflict {
                label: chord_label(&chord),
                chord: chord.unparse(),
                actions: {
                    actions.sort_by_key(|id| CATALOG.iter().position(|e| e.id == *id));
                    actions
                },
            })
            .collect();
        conflicts.sort_by(|a, b| a.chord.cmp(&b.chord));
        conflicts
    }

    // MARK: Persistence

    /// Reads overrides from the registry's file. Missing, corrupt or invalid
    /// files fall back to the current state (Swift: a decode or validation
    /// failure keeps the empty override map). Unknown ids and unparseable
    /// chords are dropped individually, mirroring Swift's tolerant decode of
    /// `[String: ShortcutChord]`; a table that would still fail validation
    /// (e.g. a stored conflict) is rejected wholesale.
    pub fn load(&mut self) {
        let path = self.path.clone();
        self.load_from(path.as_deref());
    }

    fn load_from(&mut self, path: Option<&Path>) {
        let Some(path) = path.or(self.path.as_deref()) else { return };
        let Ok(text) = fs::read_to_string(path) else { return };
        let Ok(stored) = serde_json::from_str::<StoredShortcuts>(&text) else { return };
        if stored.version != STORAGE_VERSION {
            return;
        }
        let mut candidate = ShortcutRegistry::new();
        for (id, keystroke) in &stored.overrides {
            let _ = candidate.set_override(id, keystroke);
        }
        if candidate.validate().is_ok() {
            self.overrides = candidate.overrides;
        }
    }

    /// Loads from `path` and returns the registry. Convenience for startup
    /// and tests.
    pub fn load_or_default(path: impl Into<PathBuf>) -> Self {
        let mut registry = ShortcutRegistry::with_path(path);
        registry.load_from(None);
        registry
    }

    /// Persists the overrides atomically (write a temp file, then rename).
    /// Refuses to save an invalid table, like Swift's `guard problem == nil`.
    pub fn save(&self) -> Result<(), ShortcutError> {
        self.validate().map_err(ShortcutError::Invalid)?;
        self.save_unchecked()
    }

    /// Persists without validation — for callers that already validated a
    /// draft and want the exact bytes they reviewed on disk.
    pub fn save_unchecked(&self) -> Result<(), ShortcutError> {
        let Some(path) = &self.path else { return Ok(()) };
        let stored = StoredShortcuts {
            version: STORAGE_VERSION,
            overrides: self.overrides.clone(),
        };
        let text = serde_json::to_string_pretty(&stored).map_err(|error| {
            ShortcutError::Io(io::Error::new(io::ErrorKind::InvalidData, error))
        })?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Atomic write: temp file in the same directory, then rename over the
        // target (MoveFileEx with MOVEFILE_REPLACE_EXISTING on Windows).
        let temp = path.with_extension("json.tmp");
        fs::write(&temp, text)?;
        fs::rename(&temp, path)?;
        Ok(())
    }
}

// MARK: Process-wide registry

static GLOBAL: OnceLock<Mutex<ShortcutRegistry>> = OnceLock::new();

/// The process-wide registry, loaded from the platform config directory on
/// first use (Windows: `%APPDATA%\compositor\shortcuts.json`, i.e.
/// `{FOLDERID_RoamingAppData}`; macOS: `~/Library/Application Support`;
/// Linux/XDG: `$XDG_CONFIG_HOME` or `~/.config`).
///
/// `keys::bindings()` reads this at startup so custom chords take effect
/// without any extra wiring; the settings sheet edits the same instance.
pub fn global() -> &'static Mutex<ShortcutRegistry> {
    GLOBAL.get_or_init(|| {
        let registry = match default_path() {
            Some(path) => ShortcutRegistry::load_or_default(path),
            None => ShortcutRegistry::new(),
        };
        Mutex::new(registry)
    })
}

/// Locks [`global`], recovering from a poisoned mutex (a panic while a guard
/// is held must not brick shortcuts for the rest of the process).
pub fn lock_global() -> MutexGuard<'static, ShortcutRegistry> {
    global().lock().unwrap_or_else(PoisonError::into_inner)
}

/// The platform's per-user config directory (the `dirs` crate's
/// `config_dir()` hand-rolled to avoid the dependency).
fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        // %APPDATA% is FOLDERID_RoamingAppData.
        std::env::var_os("APPDATA").map(PathBuf::from)
    }
    #[cfg(target_os = "macos")]
    {
        home_dir().map(|home| home.join("Library").join("Application Support"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(home_dir)
            .map(|home| home.join(".config"))
    }
}

#[cfg(any(target_os = "macos", all(unix, not(target_os = "macos"))))]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The shortcuts.json path inside the config directory.
fn default_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("compositor").join("shortcuts.json"))
}

// MARK: Chord helpers

/// Parses one chord in gpui keystroke format (`ctrl-shift-s`). Only single
/// chords are legal here — whitespace-separated sequences and the test-only
/// `key->char` syntax are rejected, matching Swift's "single key with
/// optional modifiers" rule.
fn normalize(keystroke: &str) -> Result<Keystroke, ()> {
    if keystroke.split_whitespace().count() > 1 {
        return Err(());
    }
    let chord = Keystroke::parse(keystroke).map_err(|_| ())?;
    if chord.key_char.is_some() {
        return Err(());
    }
    Ok(chord)
}

/// Human-readable chord label, e.g. `"Ctrl+Shift+S"` (Swift's
/// `ShortcutChord.label`, in Windows notation).
pub fn chord_label(chord: &Keystroke) -> String {
    let mut label = String::new();
    if chord.modifiers.control {
        label.push_str("Ctrl+");
    }
    if chord.modifiers.alt {
        label.push_str("Alt+");
    }
    if chord.modifiers.shift {
        label.push_str("Shift+");
    }
    if chord.modifiers.platform {
        #[cfg(target_os = "macos")]
        label.push_str("Cmd+");
        #[cfg(target_os = "windows")]
        label.push_str("Win+");
        #[cfg(all(unix, not(target_os = "macos")))]
        label.push_str("Super+");
    }
    label.push_str(&key_label(&chord.key));
    label
}

fn key_label(key: &str) -> String {
    match key {
        "escape" => "Esc".to_string(),
        "enter" => "Return".to_string(),
        "space" => "Space".to_string(),
        "left" => "\u{2190}".to_string(),
        "right" => "\u{2192}".to_string(),
        "up" => "\u{2191}".to_string(),
        "down" => "\u{2193}".to_string(),
        // Letters print uppercase, like Swift's `key.uppercased()`.
        key if key.chars().count() == 1 => key.to_uppercase(),
        key => key.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::KeyBinding;

    /// Monotonic temp directory factory (tests may run in parallel).
    fn temp_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = format!(
            "compositor-shortcuts-{}-{}-{}",
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        );
        let dir = std::env::temp_dir().join(unique);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    #[test]
    fn catalog_ids_are_unique_and_defaults_parse() {
        let mut ids = CATALOG.iter().map(|entry| entry.id).collect::<Vec<_>>();
        assert_eq!(ids.len(), 84, "expected the full customizable catalog");
        ids.sort_unstable();
        let unique = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), unique, "catalog ids must be unique");
        for entry in CATALOG {
            assert!(!entry.defaults.is_empty(), "{} has no default", entry.id);
            assert!(!entry.title.is_empty(), "{} has no title", entry.id);
            for key in entry.defaults {
                assert!(
                    normalize(key).is_ok(),
                    "{} default {key:?} does not parse",
                    entry.id
                );
            }
        }
    }

    #[test]
    fn catalog_defaults_match_swift_table() {
        // Spot-checks straight out of ShortcutDefinition.all
        // (KeyboardShortcuts.swift:70-126), with ⌘→ctrl / ⌥→alt / ⇧→shift.
        let cases: &[(&str, &[&str])] = &[
            ("tool.brush", &["b"]),
            ("tool.zoom", &["z"]),
            ("file.new", &["ctrl-n"]),
            ("file.save", &["ctrl-s"]),
            ("file.save-as", &["ctrl-shift-s"]),
            ("file.export-png", &["ctrl-shift-e"]),
            ("file.export-jpeg", &["ctrl-alt-shift-s"]),
            ("edit.undo", &["ctrl-z"]),
            ("edit.redo", &["ctrl-shift-z"]),
            ("edit.fill-foreground", &["alt-delete", "alt-backspace"]),
            ("edit.fill-background", &["ctrl-delete", "ctrl-backspace"]),
            ("edit.content-aware-fill", &["shift-delete", "shift-backspace"]),
            ("edit.free-transform", &["ctrl-t"]),
            ("image.curves", &["ctrl-m"]),
            ("image.levels", &["ctrl-l"]),
            ("image.hue-saturation", &["ctrl-u"]),
            ("image.invert", &["ctrl-i"]),
            ("image.image-size", &["ctrl-alt-i"]),
            ("image.canvas-size", &["ctrl-alt-c"]),
            ("layer.new", &["ctrl-shift-n"]),
            ("layer.duplicate", &["ctrl-j"]),
            ("layer.merge", &["ctrl-e"]),
            ("layer.group", &["ctrl-g"]),
            ("layer.move-up", &["ctrl-]"]),
            ("layer.move-down", &["ctrl-["]),
            ("layer.clipping", &["ctrl-alt-g"]),
            ("view.guides", &["ctrl-;", "ctrl-shift-r"]),
            ("view.snap", &["ctrl-shift-;"]),
            ("view.lock-guides", &["ctrl-alt-;"]),
            ("view.grid", &["ctrl-'"]),
            ("view.rulers", &["ctrl-r"]),
            ("view.zoom-in", &["ctrl-=", "ctrl-+"]),
            ("view.zoom-out", &["ctrl--"]),
            ("view.zoom-fit", &["ctrl-0"]),
            ("view.zoom-100", &["ctrl-1"]),
            ("select.all", &["ctrl-a"]),
            ("select.deselect", &["ctrl-d"]),
            ("select.invert", &["ctrl-shift-i"]),
            ("brush.softer", &["shift-["]),
            ("brush.harder", &["shift-]"]),
            ("blend.previous", &["shift--"]),
            ("blend.next", &["shift-="]),
            ("shape.cycle", &["shift-u"]),
            ("view.levels-preview", &["alt-p"]),
            ("tool.cycle-mode", &["tab"]),
            ("layer.delete", &["delete", "backspace"]),
            ("canvas.apply", &["enter"]),
            ("canvas.cancel", &["escape"]),
            ("nudge.left-10", &["shift-left"]),
        ];
        for (id, expected) in cases {
            let entry = spec(id).unwrap();
            assert_eq!(entry.defaults, *expected, "defaults of {id}");
        }
    }

    #[test]
    fn fresh_registry_is_valid_and_conflict_free() {
        let registry = ShortcutRegistry::new();
        assert!(registry.validate().is_ok());
        assert!(registry.conflicts().is_empty());
    }

    #[test]
    fn default_bindings_align_with_legacy_table() {
        // The acceptance table: every binding the app shipped before the
        // registry existed (keys.rs `bindings()`), as
        // (keystrokes, action name, context). Generated bindings from a
        // fresh registry must reproduce it exactly, in order.
        let legacy: &[(&str, &str, &str)] = &[
            // Tools
            ("v", "compositor::ToolMove", ""),
            ("m", "compositor::ToolMarquee", ""),
            ("l", "compositor::ToolLasso", ""),
            ("w", "compositor::ToolWand", ""),
            ("c", "compositor::ToolCrop", ""),
            ("b", "compositor::ToolBrush", ""),
            ("e", "compositor::ToolEraser", ""),
            ("j", "compositor::ToolHealing", ""),
            ("s", "compositor::ToolClone", ""),
            ("r", "compositor::ToolSmear", ""),
            ("g", "compositor::ToolGradient", ""),
            ("u", "compositor::ToolShape", ""),
            ("t", "compositor::ToolType", ""),
            ("i", "compositor::ToolEyedropper", ""),
            ("h", "compositor::ToolHand", ""),
            ("z", "compositor::ToolZoom", ""),
            // Palette
            ("x", "compositor::SwapColors", ""),
            ("d", "compositor::DefaultColors", ""),
            ("tab", "compositor::CycleToolMode", "Compositor"),
            // File
            ("ctrl-n", "compositor::NewCanvas", ""),
            ("ctrl-o", "compositor::OpenDocument", ""),
            ("ctrl-s", "compositor::SaveDocument", ""),
            ("ctrl-shift-s", "compositor::SaveAs", ""),
            ("ctrl-shift-e", "compositor::ExportPng", ""),
            ("ctrl-alt-shift-s", "compositor::ExportJpeg", ""),
            ("ctrl-w", "compositor::CloseDocument", ""),
            // Edit
            ("ctrl-z", "compositor::Undo", ""),
            ("ctrl-shift-z", "compositor::Redo", ""),
            ("ctrl-t", "compositor::FreeTransform", ""),
            ("ctrl-x", "compositor::Cut", ""),
            ("ctrl-c", "compositor::CopyPixels", ""),
            ("ctrl-shift-c", "compositor::CopyMerged", ""),
            ("ctrl-v", "compositor::PastePixels", ""),
            ("alt-delete", "compositor::FillForeground", ""),
            ("alt-backspace", "compositor::FillForeground", ""),
            ("ctrl-delete", "compositor::FillBackground", ""),
            ("ctrl-backspace", "compositor::FillBackground", ""),
            ("shift-delete", "compositor::ContentAwareFill", ""),
            ("shift-backspace", "compositor::ContentAwareFill", ""),
            ("ctrl-h", "compositor::ShowTransformControls", ""),
            ("ctrl-i", "compositor::InvertPixels", ""),
            ("ctrl-m", "compositor::CurvesSheet", ""),
            ("ctrl-alt-g", "compositor::ToggleClippingMask", ""),
            ("ctrl-[", "compositor::MoveLayerDown", ""),
            ("ctrl-]", "compositor::MoveLayerUp", ""),
            ("ctrl-;", "compositor::ToggleGuides", ""),
            ("ctrl-shift-r", "compositor::ToggleGuides", ""),
            ("ctrl-shift-;", "compositor::SnapToggle", ""),
            ("ctrl-alt-;", "compositor::LockGuides", ""),
            ("ctrl-l", "compositor::LevelsSheet", ""),
            ("ctrl-u", "compositor::HueSaturationSheet", ""),
            ("ctrl-alt-i", "compositor::ImageSize", ""),
            ("ctrl-alt-c", "compositor::CanvasSize", ""),
            ("ctrl-shift-n", "compositor::NewLayer", ""),
            ("ctrl-j", "compositor::DuplicateLayer", ""),
            ("ctrl-e", "compositor::MergeDown", ""),
            ("ctrl-g", "compositor::GroupSelectedLayers", ""),
            ("delete", "compositor::DeleteLayer", ""),
            ("backspace", "compositor::DeleteLayer", ""),
            ("ctrl-a", "compositor::SelectAllAction", ""),
            ("ctrl-d", "compositor::DeselectAction", ""),
            ("ctrl-shift-i", "compositor::InvertSelection", ""),
            ("ctrl-alt-d", "compositor::FeatherSheet", ""),
            ("ctrl-alt-e", "compositor::ExpandSheet", ""),
            ("ctrl-alt-shift-c", "compositor::ContractSheet", ""),
            ("ctrl-r", "compositor::ToggleRulers", ""),
            ("ctrl-'", "compositor::ToggleGrid", ""),
            ("ctrl-=", "compositor::ZoomIn", ""),
            ("ctrl-+", "compositor::ZoomIn", ""),
            ("ctrl--", "compositor::ZoomOut", ""),
            ("ctrl-0", "compositor::ZoomFit", ""),
            ("ctrl-1", "compositor::Zoom100", ""),
            // Nudge
            ("left", "compositor::NudgeLeft", ""),
            ("right", "compositor::NudgeRight", ""),
            ("up", "compositor::NudgeUp", ""),
            ("down", "compositor::NudgeDown", ""),
            ("shift-left", "compositor::NudgeLeftMore", ""),
            ("shift-right", "compositor::NudgeRightMore", ""),
            ("shift-up", "compositor::NudgeUpMore", ""),
            ("shift-down", "compositor::NudgeDownMore", ""),
            // Brush size / hardness
            ("[", "compositor::BrushSmaller", ""),
            ("]", "compositor::BrushLarger", ""),
            ("shift-[", "compositor::BrushSofter", ""),
            ("shift-]", "compositor::BrushHarder", ""),
            // Blend modes, shape kind, levels preview
            ("shift--", "compositor::BlendModePrevious", ""),
            ("shift-=", "compositor::BlendModeNext", ""),
            ("shift-u", "compositor::CycleShapeKind", ""),
            ("alt-p", "compositor::ToggleLevelsPreview", ""),
            // Canvas keys
            ("enter", "compositor::FinishLasso", "Compositor"),
            ("escape", "compositor::CancelAction", "Compositor"),
        ];

        let registry = ShortcutRegistry::new();
        let bindings = crate::keys::bindings_with(&registry);
        assert_eq!(bindings.len(), legacy.len(), "binding count");
        for (binding, expected) in bindings.iter().zip(legacy.iter()) {
            let (keystrokes, action, context) = *expected;
            let actual_keystrokes = binding
                .keystrokes()
                .iter()
                .map(|key| key.inner().unparse())
                .collect::<Vec<_>>()
                .join(" ");
            assert_eq!(actual_keystrokes, keystrokes, "keystrokes of {action}");
            assert_eq!(binding.action().name(), action, "action of {keystrokes}");
            assert_eq!(
                binding.predicate().map(|predicate| predicate.to_string()),
                if context.is_empty() { None } else { Some(context.to_string()) },
                "context of {keystrokes} -> {action}"
            );
        }

        // The ids keys.rs binds must be exactly the catalog, in order, so
        // the two tables cannot drift apart.
        let mut ids = Vec::new();
        let _ = crate::keys::bindings_tracked(&registry, Some(&mut ids));
        let catalog_ids: Vec<&str> = CATALOG.iter().map(|entry| entry.id).collect();
        assert_eq!(ids, catalog_ids);
    }

    #[test]
    fn override_takes_effect_in_generated_bindings() {
        let mut registry = ShortcutRegistry::new();
        registry.set_override("edit.undo", "ctrl-alt-z").unwrap();
        registry.set_override("tool.brush", "f").unwrap();
        let bindings = crate::keys::bindings_with(&registry);

        let find = |bindings: &[KeyBinding], name: &str| -> Vec<String> {
            bindings
                .iter()
                .filter(|binding| binding.action().name() == name)
                .map(|binding| {
                    binding
                        .keystrokes()
                        .iter()
                        .map(|key| key.inner().unparse())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect()
        };
        assert_eq!(find(&bindings, "compositor::Undo"), vec!["ctrl-alt-z"]);
        assert_eq!(find(&bindings, "compositor::ToolBrush"), vec!["f"]);
        // The stale defaults are gone from the keymap.
        assert!(!find(&bindings, "compositor::Undo").iter().any(|keys| keys == "ctrl-z"));
        assert!(!find(&bindings, "compositor::ToolBrush").iter().any(|keys| keys == "b"));
    }

    #[test]
    fn overrides_are_normalized() {
        let mut registry = ShortcutRegistry::new();
        registry.set_override("file.save", "CTRL-SHIFT-S").unwrap();
        assert_eq!(registry.chord("file.save"), "ctrl-shift-s");
    }

    #[test]
    fn multi_keystroke_overrides_are_rejected() {
        let mut registry = ShortcutRegistry::new();
        let error = registry.set_override("edit.cut", "ctrl-x ctrl-c").unwrap_err();
        assert!(matches!(error, ShortcutProblem::InvalidChord { .. }));
        assert_eq!(registry.override_for("edit.cut"), None);
    }

    #[test]
    fn unknown_actions_are_rejected() {
        let mut registry = ShortcutRegistry::new();
        let error = registry.set_override("no.such.action", "ctrl-9").unwrap_err();
        assert_eq!(error, ShortcutProblem::UnknownAction("no.such.action".into()));
    }

    #[test]
    fn reset_restores_single_and_all_defaults() {
        let mut registry = ShortcutRegistry::new();
        registry.set_override("edit.undo", "ctrl-alt-z").unwrap();
        registry.set_override("tool.brush", "f").unwrap();
        assert!(registry.is_customized("edit.undo"));
        assert!(registry.reset("edit.undo"));
        assert!(!registry.is_customized("edit.undo"));
        assert_eq!(registry.keystrokes_for("edit.undo"), vec!["ctrl-z"]);
        assert_eq!(registry.chord("edit.undo"), "ctrl-z");
        registry.reset_all();
        assert!(registry.overrides().is_empty());
        assert_eq!(registry.keystrokes_for("tool.brush"), vec!["b"]);
    }

    #[test]
    fn conflicts_detect_duplicate_assignment() {
        let mut registry = ShortcutRegistry::new();
        registry.set_override("palette.swap", "ctrl-shift-z").unwrap();
        let problems = registry.conflicts();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].chord, "ctrl-shift-z");
        assert_eq!(problems[0].label, "Ctrl+Shift+Z");
        assert_eq!(problems[0].actions, vec!["palette.swap", "edit.redo"]);
        // validate() refuses the table with Swift's conflict message shape.
        let error = registry.validate().unwrap_err();
        assert!(matches!(error, ShortcutProblem::Conflict { .. }));
        assert_eq!(
            error.to_string(),
            "ctrl-shift-z is assigned to both Swap foreground/background and Redo."
        );
    }

    #[test]
    fn conflicts_catch_alias_collisions() {
        // layer.delete keeps the `delete`/`backspace` aliases by default.
        let mut registry = ShortcutRegistry::new();
        registry.set_override("palette.swap", "delete").unwrap();
        let problems = registry.conflicts();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].actions, vec!["palette.swap", "layer.delete"]);
    }

    #[test]
    fn reserved_chords_are_rejected() {
        let mut registry = ShortcutRegistry::new();
        for reserved in ["ctrl-q", "ctrl-,", "ctrl-alt-m"] {
            registry.set_override("edit.undo", reserved).unwrap();
            match registry.validate() {
                Err(ShortcutProblem::Reserved(label)) => assert!(!label.is_empty()),
                other => panic!("{reserved} should be reserved, got {other:?}"),
            }
            registry.reset("edit.undo");
        }
    }

    #[test]
    fn text_editing_chords_need_a_non_typing_modifier() {
        // The port ships no text-editing bindings (gpui inputs handle them
        // natively), but the Swift rule is kept for catalog entries in the
        // "Text Editing" group.
        let plain = normalize("g").unwrap();
        let shifted = normalize("shift-g").unwrap();
        let controlled = normalize("ctrl-g").unwrap();
        for chord in [plain, shifted] {
            assert!(
                !(chord.modifiers.control || chord.modifiers.alt || chord.modifiers.platform),
                "typing chord must fail the rule"
            );
        }
        assert!(controlled.modifiers.control);
    }

    #[test]
    fn persistence_round_trip() {
        let dir = temp_dir("round-trip");
        let path = dir.join("compositor").join("shortcuts.json");
        let mut registry = ShortcutRegistry::with_path(&path);
        registry.set_override("edit.undo", "ctrl-alt-z").unwrap();
        registry.set_override("layer.delete", "ctrl-shift-backspace").unwrap();
        registry.save().unwrap();

        let loaded = ShortcutRegistry::load_or_default(&path);
        assert_eq!(loaded.keystrokes_for("edit.undo"), vec!["ctrl-alt-z"]);
        assert_eq!(loaded.keystrokes_for("layer.delete"), vec!["ctrl-shift-backspace"]);
        assert_eq!(loaded.keystrokes_for("tool.brush"), vec!["b"]);
        assert!(loaded.validate().is_ok());

        // The file is versioned JSON, pretty-printed and id-keyed.
        let text = fs::read_to_string(&path).unwrap();
        let stored: StoredShortcuts = serde_json::from_str(&text).unwrap();
        assert_eq!(stored.version, 1);
        assert_eq!(stored.overrides.get("edit.undo").map(String::as_str), Some("ctrl-alt-z"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = temp_dir("corrupt");
        let path = dir.join("shortcuts.json");
        write_file(&path, "{{{ not json");
        let loaded = ShortcutRegistry::load_or_default(&path);
        assert!(loaded.overrides().is_empty());
        assert_eq!(loaded.keystrokes_for("edit.undo"), vec!["ctrl-z"]);

        // A valid document with the wrong version is ignored too.
        write_file(
            &path,
            r#"{"version": 99, "overrides": {"edit.undo": "ctrl-alt-z"}}"#,
        );
        let loaded = ShortcutRegistry::load_or_default(&path);
        assert!(loaded.overrides().is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn invalid_or_conflicting_files_fall_back() {
        let dir = temp_dir("invalid");
        let path = dir.join("shortcuts.json");
        // A stored conflict: Swift rejects the whole map when `problem(in:)`
        // is non-nil; so does the port.
        write_file(
            &path,
            r#"{"version": 1, "overrides": {
                "edit.undo": "ctrl-alt-z",
                "edit.redo": "ctrl-alt-z"
            }}"#,
        );
        let loaded = ShortcutRegistry::load_or_default(&path);
        assert!(loaded.overrides().is_empty());

        // Unknown ids and unparseable chords are dropped individually; the
        // valid remainder survives.
        write_file(
            &path,
            r#"{"version": 1, "overrides": {
                "edit.undo": "ctrl-alt-z",
                "no.such.action": "ctrl-9",
                "tool.brush": "not a chord at all -"
            }}"#,
        );
        let loaded = ShortcutRegistry::load_or_default(&path);
        assert_eq!(loaded.overrides().len(), 1);
        assert_eq!(loaded.keystrokes_for("edit.undo"), vec!["ctrl-alt-z"]);
        assert_eq!(loaded.keystrokes_for("tool.brush"), vec!["b"]);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn save_refuses_conflicts_and_writes_atomically() {
        let dir = temp_dir("atomic");
        let path = dir.join("nested").join("shortcuts.json");
        let mut registry = ShortcutRegistry::with_path(&path);
        registry.set_override("palette.swap", "ctrl-z").unwrap();
        match registry.save() {
            Err(ShortcutError::Invalid(ShortcutProblem::Conflict { .. })) => {}
            other => panic!("expected a refused save, got {other:?}"),
        }
        assert!(!path.exists(), "invalid table must not reach disk");

        registry.reset_all();
        registry.set_override("edit.undo", "ctrl-alt-z").unwrap();
        registry.save().unwrap();
        assert!(path.exists());
        assert!(
            !path.with_extension("json.tmp").exists(),
            "temp file must be renamed away"
        );
        let loaded = ShortcutRegistry::load_or_default(&path);
        assert_eq!(loaded.keystrokes_for("edit.undo"), vec!["ctrl-alt-z"]);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn chord_labels_use_platform_notation() {
        assert_eq!(chord_label(&normalize("ctrl-shift-s").unwrap()), "Ctrl+Shift+S");
        assert_eq!(chord_label(&normalize("alt-ctrl-;").unwrap()), "Ctrl+Alt+;");
        assert_eq!(chord_label(&normalize("shift-u").unwrap()), "Shift+U");
        assert_eq!(chord_label(&normalize("escape").unwrap()), "Esc");
        assert_eq!(chord_label(&normalize("left").unwrap()), "\u{2190}");
        assert_eq!(chord_label(&normalize("b").unwrap()), "B");
    }

    #[test]
    fn global_registry_loads_from_disk() {
        let dir = temp_dir("global");
        let path = dir.join("compositor").join("shortcuts.json");
        write_file(
            &path,
            r#"{"version": 1, "overrides": {"edit.undo": "ctrl-alt-z"}}"#,
        );
        let registry = ShortcutRegistry::load_or_default(&path);
        assert_eq!(registry.chord("edit.undo"), "ctrl-alt-z");
        // path() reports the injected target for the settings sheet.
        assert_eq!(registry.path(), Some(path.as_path()));
        fs::remove_dir_all(dir).ok();
    }
}
