//! Tool defaults persistence (ToolDefaults.swift, ~30 lines, plus its
//! `EditorSession.swift` call sites).
//!
//! Toggles that belong to the person rather than to a document — Auto Select,
//! the transform box, rulers, guides, the grid, the pixel grid and the four
//! snapping switches plus guide locking — keep whatever they were last set to,
//! across tabs and across launches, the way Photoshop's tool options do.
//!
//! Modeling notes:
//! - Swift stores one `Bool` per key in `UserDefaults` under a `"tool."`
//!   prefix (`ToolDefaults.bool`/`ToolDefaults.set`, with each property's
//!   `didSet` writing straight through). The port keeps the same bare keys in
//!   a versioned JSON document at
//!   `{config}/compositor/tool-defaults.json` (`shortcuts.rs` style: the
//!   platform config directory, hand-rolled, no new dependencies).
//! - The `didSet` writes are immediate, so there is no exit save to wire up;
//!   the workspace calls [`persist_session`] at every toggle point
//!   (`AppState::persist_tool_defaults`).
//! - A missing or corrupt file falls back to the compiled defaults, exactly
//!   like Swift's `?? fallback` on an unreadable `UserDefaults` entry and like
//!   `shortcuts.rs`'s tolerant load.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use serde::{Deserialize, Serialize};

use compositor_core::session::EditorSession;

// MARK: Keys (EditorSession.swift:198-279, `ToolDefaults.bool("…", …)`)

/// Move tool: select layers by clicking the canvas (`transformAutoSelect`;
/// carried on `AppState`, not the core session).
pub const AUTO_SELECT: &str = "autoSelect";
/// Transform boxes are drawn over placed layers (`showsTransformControls`).
pub const TRANSFORM_CONTROLS: &str = "transformControls";
/// The per-pixel checkerboard grid over the zoomed canvas (`showsPixelGrid`).
pub const PIXEL_GRID: &str = "pixelGrid";
/// The coarse document grid (`showsGrid`).
pub const GRID: &str = "grid";
/// User-dragged guides are shown (`showsGuides`).
pub const GUIDES: &str = "guides";
/// The document rulers are shown (`showsRulers`).
pub const RULERS: &str = "rulers";
/// Master snapping switch (`snapEnabled` → `snapping_enabled`).
pub const SNAP: &str = "snap";
/// Layers snap to guides (`snapToGuides`).
pub const SNAP_GUIDES: &str = "snapGuides";
/// Layers snap to the grid (`snapToGrid`).
pub const SNAP_GRID: &str = "snapGrid";
/// Layers snap to other layers (`snapToLayers`).
pub const SNAP_LAYERS: &str = "snapLayers";
/// Layers snap to the document bounds (`snapToDocumentBounds`).
pub const SNAP_BOUNDS: &str = "snapBounds";
/// Guides cannot be dragged while locked (`locksGuides`).
pub const LOCK_GUIDES: &str = "lockGuides";

/// Every persisted key, with the compiled default next to it (the fallbacks
/// of the `ToolDefaults.bool` call sites; identical to the core session's
/// initial values, so a fresh install behaves like a fresh session).
pub const KEYS: &[(&str, bool)] = &[
    (AUTO_SELECT, false),
    (TRANSFORM_CONTROLS, true),
    (PIXEL_GRID, true),
    (GRID, false),
    (GUIDES, true),
    (RULERS, false),
    (SNAP, true),
    (SNAP_GUIDES, true),
    (SNAP_GRID, false),
    (SNAP_LAYERS, true),
    (SNAP_BOUNDS, true),
    (LOCK_GUIDES, false),
];

const STORAGE_VERSION: u32 = 1;

/// The JSON document persisted on disk.
#[derive(Serialize, Deserialize)]
struct StoredToolDefaults {
    version: u32,
    values: BTreeMap<String, bool>,
    /// Numeric defaults (Swift's `UserDefaults` doubles behind the
    /// `"tool."` prefix, e.g. the JPEG export quality). Optional so files
    /// written before numbers existed still load.
    #[serde(default)]
    numbers: BTreeMap<String, f64>,
}

/// The user's tool-toggle table (Swift's `UserDefaults` entries behind the
/// `"tool."` prefix). Reads fall back per key, writes update one key, and the
/// whole table saves atomically to [`ToolDefaults::path`].
#[derive(Debug, Default)]
pub struct ToolDefaults {
    /// Persistence target; `None` keeps the table in memory (tests).
    path: Option<PathBuf>,
    values: BTreeMap<String, bool>,
    numbers: BTreeMap<String, f64>,
}

impl ToolDefaults {
    /// An empty table with no persistence target (all compiled defaults).
    pub fn new() -> Self {
        Self::default()
    }

    /// A table bound to a JSON file. Does not read it; call [`load`]
    /// (or [`load_or_default`]) first. `path` is a constructor parameter so
    /// tests can inject a temporary directory.
    ///
    /// [`load`]: Self::load
    /// [`load_or_default`]: Self::load_or_default
    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        ToolDefaults {
            path: Some(path.into()),
            ..Self::default()
        }
    }

    /// The table's persistence target, if any.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    // MARK: Reading and writing

    /// One toggle's stored value, or `fallback` when unset (`ToolDefaults.bool`).
    pub fn bool(&self, key: &str, fallback: bool) -> bool {
        self.values.get(key).copied().unwrap_or(fallback)
    }

    /// Stores one toggle (`ToolDefaults.set`). Saving is a separate step so a
    /// batch of changes (or a session snapshot) lands as one write.
    pub fn set(&mut self, value: bool, key: &str) {
        self.values.insert(key.to_string(), value);
    }

    /// Every stored key → value pair.
    pub fn values(&self) -> &BTreeMap<String, bool> {
        &self.values
    }

    /// One stored number, or `fallback` when unset or non-finite (Swift's
    /// `UserDefaults.double(forKey:)` guarded by `isFinite`).
    pub fn number(&self, key: &str, fallback: f64) -> f64 {
        self.numbers
            .get(key)
            .copied()
            .filter(|value| value.is_finite())
            .unwrap_or(fallback)
    }

    /// Stores one number. Saving is a separate step so a batch of changes
    /// lands as one write.
    pub fn set_number(&mut self, value: f64, key: &str) {
        self.numbers.insert(key.to_string(), value);
    }

    // MARK: Persistence

    /// Reads values from the table's file. Missing, corrupt or wrongly
    /// versioned files fall back to the current state (Swift: an undecodable
    /// `UserDefaults` entry reads as `nil` → `?? fallback`).
    pub fn load(&mut self) {
        let path = self.path.clone();
        self.load_from(path.as_deref());
    }

    fn load_from(&mut self, path: Option<&Path>) {
        let Some(path) = path.or(self.path.as_deref()) else {
            return;
        };
        let Ok(text) = fs::read_to_string(path) else {
            return;
        };
        let Ok(stored) = serde_json::from_str::<StoredToolDefaults>(&text) else {
            return;
        };
        if stored.version != STORAGE_VERSION {
            return;
        }
        self.values = stored.values;
        self.numbers = stored.numbers;
    }

    /// Loads from `path` and returns the table. Convenience for startup and
    /// tests.
    pub fn load_or_default(path: impl Into<PathBuf>) -> Self {
        let mut defaults = ToolDefaults::with_path(path);
        defaults.load_from(None);
        defaults
    }

    /// Persists the table atomically: write a temp file in the target
    /// directory, then rename it over the target (MoveFileEx with
    /// MOVEFILE_REPLACE_EXISTING on Windows), so a crash mid-write can never
    /// leave a half file behind. Creates the parent directories on demand.
    pub fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let stored = StoredToolDefaults {
            version: STORAGE_VERSION,
            values: self.values.clone(),
            numbers: self.numbers.clone(),
        };
        let text = serde_json::to_string_pretty(&stored)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("json.tmp");
        fs::write(&temp, text)?;
        fs::rename(&temp, path)?;
        Ok(())
    }
}

// MARK: Session mapping (the didSet call sites in EditorSession.swift)

/// Snapshots every persisted toggle off a session and the app-level Auto
/// Select flag into a table (the values Swift's `didSet`s write through).
pub fn capture_session(
    session: &EditorSession,
    transform_auto_select: bool,
) -> BTreeMap<String, bool> {
    BTreeMap::from([
        (AUTO_SELECT.to_string(), transform_auto_select),
        (
            TRANSFORM_CONTROLS.to_string(),
            session.shows_transform_controls,
        ),
        (PIXEL_GRID.to_string(), session.shows_pixel_grid),
        (GRID.to_string(), session.shows_grid),
        (GUIDES.to_string(), session.shows_guides),
        (RULERS.to_string(), session.shows_rulers),
        (SNAP.to_string(), session.snapping_enabled),
        (SNAP_GUIDES.to_string(), session.snap_to_guides),
        (SNAP_GRID.to_string(), session.snap_to_grid),
        (SNAP_LAYERS.to_string(), session.snap_to_layers),
        (SNAP_BOUNDS.to_string(), session.snap_to_document_bounds),
        (LOCK_GUIDES.to_string(), session.locks_guides),
    ])
}

/// Applies a table onto a session and the app-level Auto Select flag (the
/// startup load; each field reads `defaults.bool(key, compiled_default)` like
/// Swift's initializers read `ToolDefaults.bool`).
pub fn apply_to_session(
    defaults: &ToolDefaults,
    session: &mut EditorSession,
    transform_auto_select: &mut bool,
) {
    *transform_auto_select = defaults.bool(AUTO_SELECT, false);
    session.shows_transform_controls = defaults.bool(TRANSFORM_CONTROLS, true);
    session.shows_pixel_grid = defaults.bool(PIXEL_GRID, true);
    session.shows_grid = defaults.bool(GRID, false);
    session.shows_guides = defaults.bool(GUIDES, true);
    session.shows_rulers = defaults.bool(RULERS, false);
    session.snapping_enabled = defaults.bool(SNAP, true);
    session.snap_to_guides = defaults.bool(SNAP_GUIDES, true);
    session.snap_to_grid = defaults.bool(SNAP_GRID, false);
    session.snap_to_layers = defaults.bool(SNAP_LAYERS, true);
    session.snap_to_document_bounds = defaults.bool(SNAP_BOUNDS, true);
    session.locks_guides = defaults.bool(LOCK_GUIDES, false);
}

// MARK: Process-wide table

static GLOBAL: OnceLock<Mutex<ToolDefaults>> = OnceLock::new();

static STARTUP_APPLIED: AtomicBool = AtomicBool::new(false);

/// The process-wide table, loaded from the platform config directory on first
/// use (Windows: `%APPDATA%\compositor\tool-defaults.json`; macOS:
/// `~/Library/Application Support`; Linux/XDG: `$XDG_CONFIG_HOME` or
/// `~/.config` — `shortcuts.rs`'s layout).
pub fn global() -> &'static Mutex<ToolDefaults> {
    GLOBAL.get_or_init(|| {
        let defaults = match default_path() {
            Some(path) => ToolDefaults::load_or_default(path),
            None => ToolDefaults::new(),
        };
        Mutex::new(defaults)
    })
}

/// Locks [`global`], recovering from a poisoned mutex (a panic while a guard
/// is held must not brick the toggles for the rest of the process).
pub fn lock_global() -> MutexGuard<'static, ToolDefaults> {
    global().lock().unwrap_or_else(PoisonError::into_inner)
}

/// Applies the stored toggles to a freshly opened window, once per process
/// (the startup load; `AppState::new` lives outside this task's file scope,
/// so the workspace calls this at the top of its first render). Returns
/// whether this call was the one that applied.
pub fn apply_startup_defaults(
    session: &mut EditorSession,
    transform_auto_select: &mut bool,
) -> bool {
    if STARTUP_APPLIED.swap(true, Ordering::Relaxed) {
        return false;
    }
    let defaults = lock_global();
    apply_to_session(&defaults, session, transform_auto_select);
    true
}

/// Snapshots the session's toggles into the process-wide table and saves it
/// to disk (`didSet { ToolDefaults.set(...) }` batched into one atomic
/// write). Write errors are swallowed, as Swift's `UserDefaults.set` is
/// fire-and-forget.
pub fn persist_session(session: &EditorSession, transform_auto_select: bool) {
    let values = capture_session(session, transform_auto_select);
    let mut defaults = lock_global();
    defaults.values = values;
    let _ = defaults.save();
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

/// The tool-defaults.json path inside the config directory.
fn default_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("compositor").join("tool-defaults.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Monotonic temp directory factory (tests may run in parallel).
    fn temp_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = format!(
            "compositor-settings-{}-{}-{}",
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
    fn keys_match_the_swift_call_sites() {
        // Every ToolDefaults.bool call in EditorSession.swift, with its
        // compiled fallback.
        let expected: &[(&str, bool)] = &[
            ("autoSelect", false),
            ("transformControls", true),
            ("pixelGrid", true),
            ("grid", false),
            ("guides", true),
            ("rulers", false),
            ("snap", true),
            ("snapGuides", true),
            ("snapGrid", false),
            ("snapLayers", true),
            ("snapBounds", true),
            ("lockGuides", false),
        ];
        assert_eq!(KEYS, expected);
        // No key is a prefix-collision of another (BTreeMap storage is flat).
        let mut names: Vec<&str> = KEYS.iter().map(|(key, _)| *key).collect();
        names.sort_unstable();
        let unique = names.len();
        names.dedup();
        assert_eq!(names.len(), unique);
    }

    #[test]
    fn bool_reads_stored_values_with_fallbacks() {
        let mut defaults = ToolDefaults::new();
        assert!(defaults.bool(RULERS, false) == false);
        assert!(defaults.bool(SNAP, true));
        defaults.set(true, RULERS);
        defaults.set(false, SNAP);
        assert!(defaults.bool(RULERS, false));
        assert!(!defaults.bool(SNAP, true));
        // Unknown keys still fall back.
        assert!(defaults.bool("no.such.key", true));
    }

    #[test]
    fn persistence_round_trip() {
        let dir = temp_dir("round-trip");
        let path = dir.join("compositor").join("tool-defaults.json");
        let mut defaults = ToolDefaults::with_path(&path);
        defaults.set(true, AUTO_SELECT);
        defaults.set(false, SNAP_GUIDES);
        defaults.set(true, RULERS);
        defaults.save().unwrap();

        let loaded = ToolDefaults::load_or_default(&path);
        assert!(loaded.bool(AUTO_SELECT, false));
        assert!(!loaded.bool(SNAP_GUIDES, true));
        assert!(loaded.bool(RULERS, false));
        // Unset keys fall back, not to zero.
        assert!(loaded.bool(TRANSFORM_CONTROLS, true));

        // The file is versioned, pretty-printed and keyed by the Swift names.
        let text = fs::read_to_string(&path).unwrap();
        let stored: StoredToolDefaults = serde_json::from_str(&text).unwrap();
        assert_eq!(stored.version, 1);
        assert_eq!(stored.values.get(AUTO_SELECT), Some(&true));
        assert_eq!(stored.values.get("snapGuides"), Some(&false));
        fs::remove_dir_all(dir).ok();
    }

    /// Numbers round-trip alongside the toggles (the JPEG export quality,
    /// Swift's `jpegExportQuality` double), and non-finite values fall back.
    #[test]
    fn numbers_round_trip_and_fall_back() {
        let dir = temp_dir("numbers");
        let path = dir.join("tool-defaults.json");
        let mut defaults = ToolDefaults::with_path(&path);
        defaults.set_number(0.85, "jpegExportQuality");
        defaults.save().unwrap();

        let loaded = ToolDefaults::load_or_default(&path);
        assert_eq!(loaded.number("jpegExportQuality", 0.5), 0.85);
        // Unset and non-finite keys fall back, not to zero.
        assert_eq!(loaded.number("no.such.key", 0.7), 0.7);
        let mut bad = ToolDefaults::new();
        bad.set_number(f64::NAN, "jpegExportQuality");
        assert_eq!(bad.number("jpegExportQuality", 0.85), 0.85);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = temp_dir("corrupt");
        let path = dir.join("tool-defaults.json");
        write_file(&path, "{{{ not json");
        let loaded = ToolDefaults::load_or_default(&path);
        assert!(loaded.values().is_empty());
        assert!(!loaded.bool(AUTO_SELECT, false));
        assert!(loaded.bool(SNAP, true));

        // A valid document with the wrong version is ignored too.
        write_file(&path, r#"{"version": 99, "values": {"autoSelect": true}}"#);
        let loaded = ToolDefaults::load_or_default(&path);
        assert!(loaded.values().is_empty());

        // A structurally wrong document (wrong field names) is ignored.
        write_file(&path, r#"{"version": 1, "toggles": {"autoSelect": true}}"#);
        let loaded = ToolDefaults::load_or_default(&path);
        assert!(loaded.values().is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn save_writes_atomically_and_creates_directories() {
        let dir = temp_dir("atomic");
        let path = dir.join("nested").join("tool-defaults.json");
        let mut defaults = ToolDefaults::with_path(&path);
        defaults.set(true, GRID);
        assert!(!path.exists(), "nothing on disk before the first save");
        defaults.save().unwrap();
        assert!(path.exists());
        assert!(
            !path.with_extension("json.tmp").exists(),
            "temp file must be renamed away"
        );

        // A second save replaces the document wholesale.
        let mut defaults = ToolDefaults::load_or_default(&path);
        defaults.values.clear();
        defaults.set(false, GRID);
        defaults.save().unwrap();
        let loaded = ToolDefaults::load_or_default(&path);
        assert_eq!(loaded.values().len(), 1);
        assert!(!loaded.bool(GRID, true));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn session_round_trips_through_the_table() {
        let mut session = EditorSession::new();
        // Flip every toggle off its compiled default; Auto Select travels
        // alongside as the app-level flag.
        session.shows_transform_controls = false;
        session.shows_pixel_grid = false;
        session.shows_grid = true;
        session.shows_guides = false;
        session.shows_rulers = true;
        session.snapping_enabled = false;
        session.snap_to_guides = false;
        session.snap_to_grid = true;
        session.snap_to_layers = false;
        session.snap_to_document_bounds = false;
        session.locks_guides = true;

        let values = capture_session(&session, true);
        assert_eq!(values.len(), KEYS.len(), "every toggle is captured");
        assert_eq!(values.get(AUTO_SELECT), Some(&true));
        assert_eq!(values.get(TRANSFORM_CONTROLS), Some(&false));
        assert_eq!(values.get(LOCK_GUIDES), Some(&true));

        // A fresh session picks the table up wholesale.
        let mut restored = EditorSession::new();
        let mut auto_select = false;
        let defaults = ToolDefaults {
            path: None,
            values: values.clone(),
            numbers: BTreeMap::new(),
        };
        apply_to_session(&defaults, &mut restored, &mut auto_select);
        assert!(auto_select);
        assert!(!restored.shows_transform_controls);
        assert!(!restored.shows_pixel_grid);
        assert!(restored.shows_grid);
        assert!(!restored.shows_guides);
        assert!(restored.shows_rulers);
        assert!(!restored.snapping_enabled);
        assert!(!restored.snap_to_guides);
        assert!(restored.snap_to_grid);
        assert!(!restored.snap_to_layers);
        assert!(!restored.snap_to_document_bounds);
        assert!(restored.locks_guides);
    }

    #[test]
    fn apply_falls_back_per_key() {
        // A table with a single entry moves only that toggle; every other
        // field keeps its compiled default (Swift's `?? fallback`).
        let mut defaults = ToolDefaults::new();
        defaults.set(true, GRID);
        let mut session = EditorSession::new();
        let mut auto_select = true;
        apply_to_session(&defaults, &mut session, &mut auto_select);
        assert!(!auto_select, "unset autoSelect falls back to false");
        assert!(session.shows_grid);
        assert!(session.shows_transform_controls, "unset falls back to true");
        assert!(session.snap_to_document_bounds, "unset falls back to true");
    }

    #[test]
    fn persist_and_startup_round_trip_through_the_global_table() {
        // Persist a snapshot into the process-wide table, then load a fresh
        // table from the same store the startup path reads.
        let mut session = EditorSession::new();
        session.shows_rulers = true;
        session.snap_to_grid = true;
        session.snapping_enabled = false;
        persist_session(&session, true);

        let global = lock_global();
        assert_eq!(global.values().len(), KEYS.len());
        assert!(global.bool(RULERS, false));
        assert!(global.bool(SNAP_GRID, false));
        assert!(!global.bool(SNAP, true));

        let mut restored = EditorSession::new();
        let mut auto_select = false;
        apply_to_session(&global, &mut restored, &mut auto_select);
        drop(global);
        assert!(auto_select);
        assert!(restored.shows_rulers);
        assert!(restored.snap_to_grid);
        assert!(!restored.snapping_enabled);
    }

    #[test]
    fn startup_defaults_apply_exactly_once() {
        // The guard is process-wide, so under parallel tests this asserts the
        // deterministic part: within one caller the second application never
        // reports that it applied.
        let mut session = EditorSession::new();
        let mut auto_select = false;
        apply_startup_defaults(&mut session, &mut auto_select);
        let second = apply_startup_defaults(&mut session, &mut auto_select);
        assert!(!second, "the startup load must not run twice");
    }
}
