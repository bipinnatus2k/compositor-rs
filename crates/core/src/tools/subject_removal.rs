//! Port of `SubjectRemoval.swift` — **blocked: platform capability gap**.
//!
//! # Why this cannot be implemented without faking it
//!
//! Swift's Remove Background has two quality levels (`FilterSettings.backgroundQuality`:
//! Basic / Advanced), but **both start from Apple's Vision framework**, which has no Rust
//! equivalent:
//!
//! - `SubjectRemoval.swift:29-38` — the coarse mask is `VNGenerateForegroundInstanceMaskRequest`
//!   (`vision(_:)`). Every quality level consumes this mask; it is the *only* source of the
//!   subject segmentation.
//! - `SubjectRemoval.swift:45-47` — `refined(_:guide:settings:limit:)` begins with
//!   `guard settings.backgroundQuality == .advanced else { return mask }`. In other words the
//!   "quick subject mask" of the **Basic** tier is *exactly Vision's raw output, untouched* —
//!   it is not a color-statistics / edge / guided-filter algorithm. "Quick" refers to skipping
//!   the post-processing below, not to a non-Vision mask.
//! - The **Advanced** tier (`SubjectRemoval.swift:48-83`) only adds CoreImage post-processing
//!   on top of the Vision mask: `GuidedMatte.refine` (guided filter, `refineEdges`), a
//!   blur-and-threshold edge shift (`shiftEdge`), and a linear contrast stretch
//!   (`matteContrast`). None of that can manufacture a subject mask on its own.
//!
//! The same applies to **Select Subject** (`EditorSession.selectSubject`,
//! `SubjectRemoval.swift:119-141`): it calls `subjectMask` (i.e. Vision) and then traces the
//! result with `MaskTracing.whitePixels`. With no Vision mask, `whitePixels` has nothing to
//! trace.
//!
//! Conclusion: no fake Vision implementation here; this stays a no-op stub. Swift's own tests
//! accept `SubjectRemoval.Failure.noSubject` for synthetic fixtures (`SmartEditTests.swift:75-78`),
//! so there is no pure-algorithm reference behavior to port either. The Remove Background menu
//! entry should be hidden or removed while this gap exists (app layer's call).
//!
//! The Advanced post-processing *is* portable — `GuidedMatte.swift` is straight arithmetic
//! (box means, He/Sun/Tang guided filter) — so once a mask source exists (e.g. a bundled
//! segmentation model), `refined` can be ported 1:1. The mask source is the gap.

#![allow(unused)]

use crate::filters::FilterSettings;
use crate::raster::Bitmap;

pub fn run(image: &Bitmap, settings: &FilterSettings) -> Bitmap {
    // No-op: Swift = SubjectRemoval.run (SubjectRemoval.swift:101-110), which needs the Vision
    // mask (see module docs). Returning the image unchanged is the honest degradation.
    image.clone()
}
