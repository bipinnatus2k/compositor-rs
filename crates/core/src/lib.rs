//! Compositor core: a faithful Rust port of the Compositor document model,
//! rendering pipeline, tools and file formats (see `Compositor/` in the repo root).
//!
//! Module layout mirrors the Swift sources:
//! - `geometry` / `transform` ← CoreGraphics types + `LayerTransform.swift`
//! - `raster` ← CGImage bitmap contexts (`BrushRaster`, `DownsampleCache`)
//! - `blend` ← `LayerAppearance.swift` blend modes + Core Image formulas
//! - `document` ← `EditorSession.swift` model types
//! - `selection` ← `Selection.swift` and friends
//! - `adjust` ← Levels / Curves / HueSaturation / CameraRaw / PixelAdjust
//! - `filters` ← `Filters.swift`
//! - `tools` ← Brush / Clone / Heal / Smudge / Gradient / Shape / Type / Wand
//! - `effects` ← `LayerEffects.swift`
//! - `history` ← `DocumentHistory.swift`
//! - `project` ← `ProjectStore.swift` `.comp` manifest format
//! - `psd` ← `IO/PSD/*`
//! - `session` ← `EditorSession` operation logic

pub mod blend;
pub mod document;
pub mod effects;
pub mod geometry;
pub mod history;
pub mod raster;
pub mod selection;
pub mod transform;

/// In progress — ported from `Document/` and `Rendering/`.
pub mod adjust;
pub mod adjustment;
pub mod canvas;
pub mod filters;
pub mod project;
pub mod psd;
pub mod render;
pub mod session;
pub mod tools;
