//! Port of the `.comp` project package format (`IO/ProjectStore.swift`,
//! `IO/ProjectController.swift`, `Document/EditorSession+Projects.swift`, and
//! `docs/project-format.md`).
//!
//! A `.comp` project is a directory package: `manifest.json` plus an `images/`
//! directory holding `<UUID>.png` image assets and `<UUID>.mask.png` grayscale
//! masks. Loading validates the whole manifest and decodes every asset before
//! anything replaces the live document; saving validates first, encodes into a
//! staged sibling package, and only then swaps it in, so a failed save never
//! damages the previously saved package.
//!
//! Module layout mirrors the Swift sources:
//! - `manifest` — `ProjectManifest` / `ProjectLayerRecord` and the Codable
//!   shapes they embed, with Swift-exact JSON keys
//! - `store` — `ProjectStore`: validation, limits, package load and the
//!   coordinated atomic save
//! - `convert` — `EditorSession+Projects`: snapshots and installation
//!   (`ProjectData` <-> `CanvasDocument`)
//! - `workspace` — `ProjectWorkspace.copyLayer`'s model half
//!   (`copy_layers_between`)

mod convert;
pub mod manifest;
mod store;
mod workspace;

use std::collections::HashMap;

use uuid::Uuid;

use crate::raster::{Bitmap, GrayMask};

pub use convert::{document_from_project, project_from_document};
pub use manifest::{
    CURRENT_VERSION, FORMAT, MAX_VERSION, MIN_VERSION, ManifestEffects, ManifestGuide,
    ManifestShape, ManifestTextStyle, ManifestTransform, ProjectLayerRecord, ProjectManifest,
    image_file_name, mask_file_name,
};
pub use store::{load, save};
pub use workspace::copy_layers_between;

/// `ProjectSnapshot`: a validated manifest plus its decoded assets. Image
/// pixels are premultiplied RGBA8 `Bitmap`s keyed by layer id; masks are
/// 8-bit grayscale `GrayMask`s.
#[derive(Clone, PartialEq)]
pub struct ProjectData {
    pub manifest: ProjectManifest,
    pub images: HashMap<Uuid, Bitmap>,
    pub masks: HashMap<Uuid, GrayMask>,
}

impl std::fmt::Debug for ProjectData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectData")
            .field("manifest", &self.manifest)
            .field(
                "images",
                &self
                    .images
                    .iter()
                    .map(|(id, bitmap)| (*id, bitmap.width, bitmap.height))
                    .collect::<Vec<_>>(),
            )
            .field(
                "masks",
                &self
                    .masks
                    .iter()
                    .map(|(id, mask)| (*id, mask.width, mask.height))
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// `ProjectError`: a package could not be opened or saved. The live document
/// is never replaced on failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectError {
    /// Not a valid Compositor project, or damaged metadata.
    Invalid,
    /// An unsupported format version.
    Version(i64),
    /// An image inside the project is missing or damaged.
    MissingImage,
    /// Over the canvas, layer, file-size or 100-megapixel image limit.
    TooLarge,
    /// An image could not be saved; the previous package is untouched.
    Encode,
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectError::Invalid => write!(
                f,
                "This is not a valid Compositor project, or its metadata is damaged."
            ),
            ProjectError::Version(version) => write!(
                f,
                "This project uses format version {version}. This app supports versions \
                 {MIN_VERSION}\u{2013}{MAX_VERSION}."
            ),
            ProjectError::MissingImage => write!(
                f,
                "An image inside the project is missing or damaged. The current document \
                 has not been replaced."
            ),
            ProjectError::TooLarge => write!(
                f,
                "This project exceeds the supported canvas, layer, file-size, or \
                 100-megapixel image limit."
            ),
            ProjectError::Encode => write!(
                f,
                "An image could not be saved. The previous project has not been replaced."
            ),
        }
    }
}

impl std::error::Error for ProjectError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_mirror_the_swift_localizations() {
        assert_eq!(
            ProjectError::Invalid.to_string(),
            "This is not a valid Compositor project, or its metadata is damaged."
        );
        assert_eq!(
            ProjectError::Version(42).to_string(),
            "This project uses format version 42. This app supports versions 1\u{2013}9."
        );
        assert_eq!(
            ProjectError::MissingImage.to_string(),
            "An image inside the project is missing or damaged. The current document has not been replaced."
        );
        assert_eq!(
            ProjectError::TooLarge.to_string(),
            "This project exceeds the supported canvas, layer, file-size, or 100-megapixel image limit."
        );
        assert_eq!(
            ProjectError::Encode.to_string(),
            "An image could not be saved. The previous project has not been replaced."
        );
    }

    #[test]
    fn new_saves_use_the_highest_swift_version() {
        assert_eq!(CURRENT_VERSION, 9);
        assert_eq!(MIN_VERSION, 1);
        assert_eq!(MAX_VERSION, 9);
        assert_eq!(FORMAT, "com.compositor.project");
    }
}
