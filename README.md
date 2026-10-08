# compositor-rs

A Rust rewrite of [Compositor](./Compositor/) — the free, open-source
Photoshop-style image editor for macOS — aiming for a 1:1 restoration of the
original program's logic, data model, algorithms and UI, verified by tests
ported from the original XCTest suite.

The original is ~39,000 lines of Swift/SwiftUI/Metal plus C pixel kernels.
This workspace ports that program to pure Rust: `compositor-core` holds the
whole engine (document model, rendering, tools, adjustments, filters, file
formats), `compositor` (crates/app) is the desktop application built on
gpui-kit, the Rust UI framework.

## Status

| Original (Swift/C) | Port (Rust) | Notes |
| --- | --- | --- |
| `EditorSession.swift` model, `NavigationTool` | `core::document` | `ImageLayer`, `CanvasDocument`, guides, `PaletteColor`, `PickerHSB`, text style, shapes |
| `LayerTransform.swift` | `core::transform` | exact formulas: `point/contains/placing/following/rounded`, `TransformDrag`, `TransformSnap` |
| `BrushRaster`, CGContext bitmaps, `DownsampleCache` | `core::raster` | premultiplied RGBA8 bitmaps, gray masks, affine resample, halving, Gaussian blur, `falloff`, `pixel_to_document` |
| `LayerAppearance.swift` + Core Image blend modes | `core::blend` | all 24 modes with the PDF/Photoshop formulas CI implements (including the 10 that needed a CI surface) |
| `Selection.swift` (CGPath + CIBlending) | `core::selection` | polygon boolean engine (geo), analytic area coverage (sub-pixel exact), feather = σ/2 Gaussian, expand/contract via round-join stroke bands |
| `Levels.swift` + `LevelsPixels.c` | `core::adjust::levels` | exact tables, `levels_apply`, `levels_histogram`, display scaling |
| `Curves.swift` | `core::adjust::curves` | shape-preserving Hermite spline, exact |
| `HueSaturation.swift` | `core::adjust::hue_saturation` | bands, colorize, the exact HSL adjust |
| `ImageAdjustments.swift` + `AdjustPixels.c` | `core::adjust::image_adjustments` | exposure (sRGB↔linear), gradient map, B&W, color balance, grain — kernel-exact |
| `PixelAdjust/PixelInvert.swift` | `core::adjust::pixel` | invert (α−c), blend-through-selection, guided matte (He/Sun/Tang) |
| `CameraRaw*.swift` + `AdjustPixels.c` camera kernels | `core::adjust::camera_raw` | full pipeline: geometry warp, calibration, light/color, parametric+point curves, mixer, grading, effects (texture/clarity/dehaze/glow/vignette), detail, optics, scope |
| `Filters.swift` + `NoisePixels.c`, `LensPixels.c` | `core::filters` | Gaussian/motion blur, Add Noise (same hash), Lens Correction (same warp), trimmed() |
| `LayerEffects.swift` | `core::effects` | stroke/shadow/overlay/inner shadow/glow with the sliding-window morphology `extreme()` |
| `DocumentHistory.swift` | `core::history` | snapshot undo with entry + byte limits, nestable transactions |
| `LayerMask.swift` | `core::document` + `core::render` | placement/link semantics, edge-tone background |
| `BrushStroke.swift` | `core::tools::brush` | centripetal Catmull-Rom, dab spacing, coverage caps, clone/erase/heal wash, gradients, fills, pixel moves |
| `HealPixels.c` | `core::tools::heal` | membrane solve + patch search, kernel-exact |
| `ContentFill.c` | `core::tools::content_fill` | coherent-offset patch synthesis, kernel-exact |
| `WandPixels.c`, `MagicWand.swift` | `core::tools::wand` + `session` | same hash/flood-fill/outline trace |
| `CanvasResizer`, `ImageResizer`, `Crop`, `ImageTrim`, `LayerFlip` | `core::canvas` | anchors, units, trim, mirror math |
| `LayerRenderer`/`TiledLayerRenderer` semantics | `core::render` | compositing: blend, opacity, masks, folder masks, clipping masks, effects, adjustment layers, canvas-shaped panel thumbnails |
| `LevelsAutomatic.swift` | `core::adjust::levels_auto` | Contrast/Color/Neutral auto levels, 256-bin histograms, clip 0.001, neutral gamma |
| `AdjustmentEditing.swift` | `core::session` | the adjustment-layer editing session (preview/commit/cancel, bound to its layer) |
| `SelectionEdits.swift` | `core::session` | fill/clear/invert selected pixels, mask from selection, adjustment creation |
| `KeyboardShortcuts.swift` | `app::shortcuts` | action catalog with per-action overrides, conflict validation, persisted customizations |
| `EditorSession` + extensions | `core::session` | the operation state machine with the original undo names and guards, busy/import/sheet flags included (`can_start_project_operation`) |
| `IO/ProjectStore.swift` (`.comp` package) | `core::project` | manifest JSON v1–9 semantics, validation limits, atomic save |
| `IO/PSD/*` | `core::psd` | PSD reader/writer, RLE, layer records, vector paths |
| SwiftUI UI | `crates/app` (gpui-kit) | tool rail, per-tool options header, canvas viewport with rulers/guides/grid/selection ants/crop overlay, layers panel (blend/opacity/thumbnails/rename), sheets (New/Image Size/Canvas Size/Levels with histogram/Hue-Saturation/Filters/Camera Raw with 14 sliders/Trim/Export), menus, Photoshop-style shortcuts |
| `SmudgeLiquify.swift` (`WarpStroke`) | `core::tools::warp` + UI | Smudge pickup/paint, Liquify forward warp; the finished stroke paints back into the layer along its path (`finishWarp`) |
| `ShapeTool.swift` | `core::render::render_shape_image` + UI | Rectangle/Rounded/Ellipse/Line as editable `LayerShape` layers, re-rendered live while uncommitted |

## Building and testing

Requires Rust 1.85+ (edition 2024).

```sh
cargo test          # the ported test suite
cargo run -p compositor   # the desktop app
```

`cargo test -p compositor-core` runs the engine tests; `cargo run` opens the
editor window (Windows/macOS/Linux via gpui).

## Project format and PSD

`.comp` packages (manifest.json + PNG assets) are read and written with the
version 1–9 semantics documented in
[`Compositor/docs/project-format.md`](./Compositor/docs/project-format.md).
Photoshop PSD files (8-bit RGB) are imported with the same feature subset the
original supports.

## License

MIT — see [Compositor/LICENSE](./Compositor/LICENSE).
