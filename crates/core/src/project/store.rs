//! `ProjectStore`: validation and package IO for `.comp` projects, ported
//! 1:1 from `IO/ProjectStore.swift` (with `LayerGroups.swift`'s
//! `LayerHierarchy.validate` and `LiveLayerMask.swift`'s `LiveMaskGraph.validate`).
//!
//! A `.comp` package is a directory holding `manifest.json` and an `images/`
//! directory of `<UUID>.png` / `<UUID>.mask.png` assets. Saving stages a
//! complete sibling package and only then swaps it in, so a failed save never
//! damages the previously saved package (Swift's `NSFileCoordinator`
//! coordinated replacement; here a best-effort remove-and-rename).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufReader, Cursor};
use std::path::Path;

use image::codecs::png::PngDecoder;
use image::{ColorType, DynamicImage, ImageDecoder, ImageFormat};
use uuid::Uuid;

use super::manifest::{
    MAX_VERSION, MIN_VERSION, ProjectLayerRecord, ProjectManifest, image_file_name, mask_file_name,
};
use super::{ProjectData, ProjectError};
use crate::adjustment::AdjustmentKind;
use crate::blend::LayerBlendMode;
use crate::document::LayerTextStyle;
use crate::raster::{Bitmap, GrayMask};

const MANIFEST_MAX_BYTES: u64 = 4 * 1024 * 1024;
const ASSET_MAX_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SIDE: i64 = 30_000;
const MAX_SOURCE_PIXELS: u64 = 100_000_000;
const MAX_LAYERS: usize = 10_000;
const MAX_GUIDES: usize = 1_000;

/// `ProjectStore.validate`: the manifest must be self-consistent and inside
/// every limit before a package is written or opened.
pub(crate) fn validate(manifest: &ProjectManifest) -> Result<(), ProjectError> {
    if manifest.format != super::manifest::FORMAT {
        return Err(ProjectError::Invalid);
    }
    if !(MIN_VERSION..=MAX_VERSION).contains(&manifest.version) {
        return Err(ProjectError::Version(manifest.version));
    }
    if manifest.color_space != "sRGB" {
        return Err(ProjectError::Invalid);
    }
    if let Some(resolution) = manifest.resolution
        && (!resolution.is_finite() || !(1.0..=9600.0).contains(&resolution))
    {
        return Err(ProjectError::Invalid);
    }
    if !(1..=MAX_SIDE).contains(&manifest.width)
        || !(1..=MAX_SIDE).contains(&manifest.height)
        || manifest.layers.len() > MAX_LAYERS
    {
        return Err(ProjectError::TooLarge);
    }
    for layer in &manifest.layers {
        if let Some(text) = &layer.text
            && (!LayerTextStyle::from(text.clone()).is_valid()
                || layer.image_file.is_none()
                || layer.is_group == Some(true)
                || layer.adjustment.is_some())
        {
            return Err(ProjectError::Invalid);
        }
        if let Some(adjustment) = &layer.adjustment {
            // Adjustment layers arrived in version 7; the blur and noise
            // filters in version 9.
            if manifest.version < 7
                || layer.is_group == Some(true)
                || layer.image_file.is_some()
                || !adjustment.is_valid()
            {
                return Err(ProjectError::Invalid);
            }
            if matches!(
                adjustment.kind,
                AdjustmentKind::GaussianBlur
                    | AdjustmentKind::MotionBlur
                    | AdjustmentKind::AddNoise
            ) && manifest.version < 9
            {
                return Err(ProjectError::Invalid);
            }
        }
        // Layer masks arrived in version 4, folder masks in version 6.
        if let Some(mask_file) = &layer.mask_file {
            let minimum = if layer.is_group == Some(true) { 6 } else { 4 };
            if manifest.version < minimum || *mask_file != mask_file_name(layer.id) {
                return Err(ProjectError::Invalid);
            }
        }
        if layer.mask_enabled.is_some() && layer.mask_file.is_none() {
            return Err(ProjectError::Invalid);
        }
        if let Some(placement) = &layer.mask_placement
            && (!placement.is_valid() || layer.mask_file.is_none())
        {
            return Err(ProjectError::Invalid);
        }
        let opacity = layer.opacity.unwrap_or(1.0);
        let blend = layer.blend_mode.unwrap_or(LayerBlendMode::Normal);
        // Folders took an opacity of their own in version 8, which multiplies
        // into what is inside them; their blend mode is still pass-through, so
        // it stays Normal.
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(ProjectError::Invalid);
        }
        if manifest.version < 3 && !(opacity == 1.0 && blend == LayerBlendMode::Normal) {
            return Err(ProjectError::Invalid);
        }
        if layer.is_group == Some(true)
            && !(blend == LayerBlendMode::Normal && (manifest.version >= 8 || opacity == 1.0))
        {
            return Err(ProjectError::Invalid);
        }
    }
    validate_hierarchy(&manifest.layers)?;
    validate_live_masks(&manifest.layers)?;
    if manifest.version < 5 && manifest.layers.iter().any(|l| l.mask_source_id.is_some()) {
        return Err(ProjectError::Invalid);
    }
    if manifest.version == 1
        && manifest
            .layers
            .iter()
            .any(|l| l.parent_id.is_some() || l.is_group == Some(true))
    {
        return Err(ProjectError::Invalid);
    }
    let mut ids = HashSet::new();
    for layer in &manifest.layers {
        if !ids.insert(layer.id) {
            return Err(ProjectError::Invalid);
        }
        if !layer.transform.is_valid() {
            return Err(ProjectError::Invalid);
        }
        if layer.name.trim().is_empty() || layer.name.len() > 16_384 {
            return Err(ProjectError::Invalid);
        }
        if let Some(image_file) = &layer.image_file
            && *image_file != image_file_name(layer.id)
        {
            return Err(ProjectError::Invalid);
        }
    }
    if let Some(active) = manifest.active_layer_id
        && !ids.contains(&active)
    {
        return Err(ProjectError::Invalid);
    }
    validate_guides(manifest)
}

/// `LayerHierarchy.validate`: unique ids, groups carry no image, parents exist
/// and are groups, and no nesting beyond 64 ancestor levels (groups one less,
/// so leaf nodes still fit at the deepest level).
fn validate_hierarchy(layers: &[ProjectLayerRecord]) -> Result<(), ProjectError> {
    let mut by_id: HashMap<Uuid, &ProjectLayerRecord> = HashMap::new();
    for layer in layers {
        if by_id.insert(layer.id, layer).is_some() {
            return Err(ProjectError::Invalid);
        }
        if layer.is_group == Some(true) && layer.image_file.is_some() {
            return Err(ProjectError::Invalid);
        }
    }
    for layer in layers {
        let mut seen: HashSet<Uuid> = HashSet::from([layer.id]);
        let mut parent = layer.parent_id;
        while let Some(id) = parent {
            if seen.len() > 64 || !seen.insert(id) {
                return Err(ProjectError::Invalid);
            }
            let Some(node) = by_id.get(&id) else {
                return Err(ProjectError::Invalid);
            };
            if node.is_group != Some(true) {
                return Err(ProjectError::Invalid);
            }
            parent = node.parent_id;
        }
        if layer.is_group == Some(true) && seen.len() > 64 {
            return Err(ProjectError::Invalid);
        }
    }
    Ok(())
}

/// `LiveMaskGraph.validate`: clipping sources exist, are non-group layers
/// without adjustments, and form no self-links, cycles or chains over 256 nodes.
fn validate_live_masks(layers: &[ProjectLayerRecord]) -> Result<(), ProjectError> {
    let mut records: HashMap<Uuid, &ProjectLayerRecord> = HashMap::new();
    for layer in layers {
        if records.insert(layer.id, layer).is_some() {
            return Err(ProjectError::Invalid);
        }
    }
    for layer in layers {
        let mut path: HashSet<Uuid> = HashSet::new();
        let mut current = Some(layer.id);
        while let Some(id) = current {
            if path.len() >= 256 || !path.insert(id) {
                return Err(ProjectError::Invalid);
            }
            let Some(record) = records.get(&id) else {
                return Err(ProjectError::Invalid);
            };
            if let Some(source) = record.mask_source_id
                && (record.is_group == Some(true)
                    || !records.contains_key(&source)
                    || records
                        .get(&source)
                        .map(|r| r.is_group == Some(true))
                        .unwrap_or(true)
                    || records
                        .get(&source)
                        .map(|r| r.adjustment.is_some())
                        .unwrap_or(true))
            {
                return Err(ProjectError::Invalid);
            }
            current = record.mask_source_id;
        }
    }
    Ok(())
}

/// `ProjectStore.validateGuides`: guides arrived in version 8.
fn validate_guides(manifest: &ProjectManifest) -> Result<(), ProjectError> {
    let guides = manifest.guides.as_deref().unwrap_or(&[]);
    if manifest.version < 8 {
        return if guides.is_empty() {
            Ok(())
        } else {
            Err(ProjectError::Invalid)
        };
    }
    if guides.len() > MAX_GUIDES {
        return Err(ProjectError::TooLarge);
    }
    let mut ids = HashSet::new();
    for guide in guides {
        if !ids.insert(guide.id)
            || !guide.position.is_finite()
            || guide.position.abs() > 1_000_000.0
        {
            return Err(ProjectError::Invalid);
        }
    }
    Ok(())
}

/// `ProjectStore.checkSize`: per-side and cumulative source-pixel budgets
/// (masks have a budget of their own).
fn check_size(width: u32, height: u32, used: &mut u64) -> Result<(), ProjectError> {
    let wide = width as i64;
    let high = height as i64;
    if !(1..=MAX_SIDE).contains(&wide)
        || !(1..=MAX_SIDE).contains(&high)
        || width as u64 * height as u64 > MAX_SOURCE_PIXELS.saturating_sub(*used)
    {
        return Err(ProjectError::TooLarge);
    }
    *used += width as u64 * height as u64;
    Ok(())
}

/// `ProjectStore.checkFile`: regular files only, inside the package, under the
/// byte limit. Symlinks, directories and oversize files map to `.TooLarge`,
/// exactly as the Swift resource-value checks do; unreadable paths report the
/// caller's "missing" error.
fn check_file(
    file: &Path,
    package: &Path,
    maximum_bytes: u64,
    missing: ProjectError,
) -> Result<(), ProjectError> {
    let root = package.canonicalize().map_err(|_| ProjectError::Invalid)?;
    let resolved = file.canonicalize().map_err(|_| missing)?;
    if !resolved.starts_with(&root) {
        return Err(ProjectError::Invalid);
    }
    let values = fs::symlink_metadata(file).map_err(|_| missing)?;
    if values.is_symlink() || !values.is_file() || values.len() > maximum_bytes {
        return Err(ProjectError::TooLarge);
    }
    Ok(())
}

/// An asset decoded from `images/`: straight-alpha PNG pixels premultiplied
/// into a `Bitmap`, or 8-bit grayscale mask pixels.
enum Asset {
    Image(Bitmap),
    Mask(GrayMask),
}

/// Reads one PNG asset. ImageIO's checks, in order: the file must be a PNG
/// (`MissingImage` otherwise), no deeper than 8 bits per component
/// (`MissingImage`), dimensions inside the pixel budget (`TooLarge`), and a
/// mask must be plain 8-bit grayscale without alpha (`Invalid`).
fn read_png_asset(path: &Path, is_mask: bool, used: &mut u64) -> Result<Asset, ProjectError> {
    let file = fs::File::open(path).map_err(|_| ProjectError::MissingImage)?;
    let decoder = PngDecoder::new(BufReader::new(file)).map_err(|_| ProjectError::MissingImage)?;
    // Multi-frame (APNG) files are rejected: the package format stores single
    // images (`CGImageSourceGetCount(source) == 1`).
    if decoder.is_apng().unwrap_or(true) {
        return Err(ProjectError::MissingImage);
    }
    let (width, height) = decoder.dimensions();
    let color_type = decoder.color_type();
    if matches!(
        color_type,
        ColorType::L16 | ColorType::La16 | ColorType::Rgb16 | ColorType::Rgba16
    ) {
        return Err(ProjectError::MissingImage);
    }
    check_size(width, height, used)?;
    if is_mask {
        if color_type != ColorType::L8 {
            return Err(ProjectError::Invalid);
        }
        let mut data = vec![0u8; width as usize * height as usize];
        decoder
            .read_image(&mut data)
            .map_err(|_| ProjectError::MissingImage)?;
        Ok(Asset::Mask(GrayMask::from_raw(
            width as usize,
            height as usize,
            data,
        )))
    } else {
        let dynamic =
            DynamicImage::from_decoder(decoder).map_err(|_| ProjectError::MissingImage)?;
        let rgba = dynamic.into_rgba8();
        let (width, height) = (rgba.width() as usize, rgba.height() as usize);
        let mut data = rgba.into_raw();
        for pixel in data.chunks_exact_mut(4) {
            premultiply_pixel(pixel);
        }
        Ok(Asset::Image(Bitmap::from_raw(width, height, data)))
    }
}

/// Straight alpha to premultiplied alpha (the same rounding as
/// `Bitmap::new_clearing`).
fn premultiply_pixel(pixel: &mut [u8]) {
    let a = pixel[3] as u32;
    if a == 255 || a == 0 {
        return;
    }
    for channel in &mut pixel[..3] {
        *channel = ((*channel as u32 * a + 127) / 255) as u8;
    }
}

/// Premultiplied alpha back to straight alpha for PNG encoding.
fn unpremultiply_pixel(pixel: &mut [u8]) {
    let a = pixel[3] as u32;
    if a == 255 || a == 0 {
        return;
    }
    for channel in &mut pixel[..3] {
        *channel = ((*channel as u32 * 255 + a / 2) / a) as u8;
    }
}

fn encode_image_png(bitmap: &Bitmap) -> Result<Vec<u8>, ProjectError> {
    let mut data = bitmap.data.clone();
    for pixel in data.chunks_exact_mut(4) {
        unpremultiply_pixel(pixel);
    }
    let image = image::RgbaImage::from_raw(bitmap.width as u32, bitmap.height as u32, data)
        .ok_or(ProjectError::Encode)?;
    let mut out = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .map_err(|_| ProjectError::Encode)?;
    Ok(out)
}

fn encode_mask_png(mask: &GrayMask) -> Result<Vec<u8>, ProjectError> {
    let image =
        image::GrayImage::from_raw(mask.width as u32, mask.height as u32, mask.data.clone())
            .ok_or(ProjectError::Encode)?;
    let mut out = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .map_err(|_| ProjectError::Encode)?;
    Ok(out)
}

/// `ProjectStore.load`: reads and validates the package, then decodes every
/// referenced asset.
pub fn load(dir: &Path) -> Result<ProjectData, ProjectError> {
    if !dir.is_dir() {
        return Err(ProjectError::Invalid);
    }
    let metadata_path = dir.join("manifest.json");
    check_file(
        &metadata_path,
        dir,
        MANIFEST_MAX_BYTES,
        ProjectError::Invalid,
    )?;
    let metadata = fs::read(&metadata_path).map_err(|_| ProjectError::Invalid)?;
    // The header decides between "damaged" and "unsupported version".
    #[derive(serde::Deserialize)]
    struct Header {
        format: String,
        version: i64,
    }
    let header: Header = serde_json::from_slice(&metadata).map_err(|_| ProjectError::Invalid)?;
    if header.format != super::manifest::FORMAT {
        return Err(ProjectError::Invalid);
    }
    if !(MIN_VERSION..=MAX_VERSION).contains(&header.version) {
        return Err(ProjectError::Version(header.version));
    }
    let manifest: ProjectManifest =
        serde_json::from_slice(&metadata).map_err(|_| ProjectError::Invalid)?;
    validate(&manifest)?;
    let images_dir = dir.join("images");
    let mut images: HashMap<Uuid, Bitmap> = HashMap::new();
    let mut masks: HashMap<Uuid, GrayMask> = HashMap::new();
    let mut pixels = 0u64;
    let mut mask_pixels = 0u64;
    for layer in &manifest.layers {
        for is_mask in [false, true] {
            let filename = if is_mask {
                layer.mask_file.clone()
            } else {
                layer.image_file.clone()
            };
            let Some(filename) = filename else { continue };
            let file = images_dir.join(&filename);
            check_file(&file, dir, ASSET_MAX_BYTES, ProjectError::MissingImage)?;
            let asset = read_png_asset(
                &file,
                is_mask,
                if is_mask {
                    &mut mask_pixels
                } else {
                    &mut pixels
                },
            )?;
            match asset {
                Asset::Image(bitmap) => {
                    images.insert(layer.id, bitmap);
                }
                Asset::Mask(mask) => {
                    masks.insert(layer.id, mask);
                }
            }
        }
    }
    Ok(ProjectData {
        manifest,
        images,
        masks,
    })
}

/// `ProjectStore.save`: validates, encodes the manifest and every asset, and
/// then replaces `dir` with the new package in one coordinated step.
pub fn save(dir: &Path, data: &ProjectData) -> Result<(), ProjectError> {
    let manifest = &data.manifest;
    validate(manifest)?;
    let mut files: std::collections::BTreeMap<String, Vec<u8>> = std::collections::BTreeMap::new();
    let mut pixels = 0u64;
    let mut mask_pixels = 0u64;
    for layer in &manifest.layers {
        for is_mask in [false, true] {
            let filename = if is_mask {
                layer.mask_file.clone()
            } else {
                layer.image_file.clone()
            };
            let Some(filename) = filename else { continue };
            if is_mask {
                // GrayMask is 8-bit grayscale by construction, so the Swift
                // `LayerMask.isValid` check cannot fail here.
                let asset = data
                    .masks
                    .get(&layer.id)
                    .ok_or(ProjectError::MissingImage)?;
                check_size(asset.width as u32, asset.height as u32, &mut mask_pixels)?;
                files.insert(filename, encode_mask_png(asset)?);
            } else {
                let asset = data
                    .images
                    .get(&layer.id)
                    .ok_or(ProjectError::MissingImage)?;
                check_size(asset.width as u32, asset.height as u32, &mut pixels)?;
                files.insert(filename, encode_image_png(asset)?);
            }
        }
    }
    let metadata = serde_json::to_string_pretty(manifest).map_err(|_| ProjectError::Encode)?;
    if metadata.len() as u64 > MANIFEST_MAX_BYTES {
        return Err(ProjectError::TooLarge);
    }
    write_package(dir, metadata.as_bytes(), &files)
}

/// Stages the complete package as a sibling of `dir`, then swaps it in. On
/// Windows (and wherever an atomic directory exchange is unavailable) this is
/// best-effort: the old package is removed and the staged one renamed into
/// place; any failure leaves the previous package untouched.
fn write_package(
    dir: &Path,
    metadata: &[u8],
    files: &std::collections::BTreeMap<String, Vec<u8>>,
) -> Result<(), ProjectError> {
    let name = dir
        .file_name()
        .ok_or(ProjectError::Encode)?
        .to_string_lossy()
        .to_string();
    let parent = dir.parent().ok_or(ProjectError::Encode)?;
    let stage_error = |_| ProjectError::Encode;
    let staging = parent.join(format!("{name}.staging-{}", Uuid::new_v4()));
    fs::create_dir_all(staging.join("images")).map_err(stage_error)?;
    let write_synced = |path: &Path, bytes: &[u8]| -> Result<(), ProjectError> {
        // Flush to disk: a power cut between the renames must not leave a
        // new package whose contents never reached the drive.
        use std::io::Write;
        let mut file = fs::File::create(path).map_err(stage_error)?;
        file.write_all(bytes).map_err(stage_error)?;
        file.sync_all().map_err(stage_error)
    };
    write_synced(&staging.join("manifest.json"), metadata)?;
    for (filename, bytes) in files {
        write_synced(&staging.join("images").join(filename), bytes)?;
    }
    if let Ok(dir_handle) = fs::File::open(&staging) {
        let _ = dir_handle.sync_all();
    }
    // The old package moves aside first and is only deleted once the new
    // package is in place. This is still remove-and-rename, not an atomic
    // directory exchange: a process killed between the two renames finds the
    // destination missing, with the complete old package in `.backup-*` for
    // manual recovery (see the module note above).
    let backup = parent.join(format!("{name}.backup-{}", Uuid::new_v4()));
    let replaced = match fs::symlink_metadata(dir) {
        Ok(existing) => {
            let committed = fs::rename(dir, &backup).and_then(|()| fs::rename(&staging, dir));
            match committed {
                Ok(()) => {
                    if existing.is_dir() {
                        let _ = fs::remove_dir_all(&backup);
                    } else {
                        let _ = fs::remove_file(&backup);
                    }
                    Ok(())
                }
                Err(rename_error) => {
                    // Roll the old package back into place.
                    let _ = fs::rename(&backup, dir);
                    let _ = fs::remove_dir_all(&staging);
                    Err(rename_error)
                }
            }
        }
        Err(_) => match fs::rename(&staging, dir) {
            Ok(()) => Ok(()),
            Err(rename_error) => {
                let _ = fs::remove_dir_all(&staging);
                Err(rename_error)
            }
        },
    };
    match replaced {
        Ok(()) => {
            // Best-effort: persist the directory entry so the rename
            // survives a power cut too.
            if let Ok(parent_handle) = fs::File::open(parent) {
                let _ = parent_handle.sync_all();
            }
            Ok(())
        }
        Err(_) => Err(ProjectError::Encode),
    }
}

#[cfg(test)]
mod tests {
    use super::super::convert::{document_from_project, project_from_document};
    use super::super::manifest::{
        CURRENT_VERSION, ManifestGuide, ManifestTextStyle, ManifestTransform, ProjectLayerRecord,
        ProjectManifest, image_file_name, mask_file_name,
    };
    use super::*;
    use crate::document::{CanvasDocument, CanvasGuide, GuideAxis, ImageLayer};
    use crate::geometry::{CGPoint, CGSize};
    use crate::raster::ImportedImage;
    use crate::transform::{LayerSampling, LayerTransform};

    /// A unique scratch folder removed on drop (the Swift tests'
    /// `temporaryFolder`).
    struct TempRoot(std::path::PathBuf);

    impl TempRoot {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "CompositorProjectTests-{tag}-{}-{}",
                std::process::id(),
                Uuid::new_v4().simple()
            ));
            fs::create_dir_all(&dir).unwrap();
            TempRoot(dir)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The 64x32 import fixture: opaque red on the left half, transparent on
    /// the right.
    fn fixture() -> Bitmap {
        let mut bitmap = Bitmap::new(64, 32);
        for y in 0..32 {
            for x in 0..64 {
                if x < 32 {
                    bitmap.set_pixel(x, y, [255, 0, 0, 255]);
                }
            }
        }
        bitmap
    }

    fn transform(origin: (f64, f64), size: (f64, f64)) -> ManifestTransform {
        ManifestTransform::from(&LayerTransform::new(
            CGPoint::new(origin.0, origin.1),
            CGSize::new(size.0, size.1),
        ))
    }

    fn blank_record() -> ProjectLayerRecord {
        ProjectLayerRecord {
            id: Uuid::new_v4(),
            name: "Layer".to_string(),
            is_visible: true,
            transform: transform((0.0, 0.0), (10.0, 10.0)),
            image_file: None,
            parent_id: None,
            is_group: Some(false),
            opacity: None,
            blend_mode: None,
            mask_file: None,
            mask_enabled: None,
            mask_source_id: None,
            adjustment: None,
            mask_placement: None,
            mask_linked: None,
            shape: None,
            effects: None,
            text: None,
        }
    }

    fn base_manifest() -> ProjectManifest {
        let mut manifest = ProjectManifest::new(Uuid::new_v4(), 10, 10);
        manifest.layers.push(blank_record());
        manifest
    }

    // MARK: validation rules

    #[test]
    fn supported_versions_and_rejection() {
        let manifest = base_manifest();
        for version in MIN_VERSION..=MAX_VERSION {
            let mut m = manifest.clone();
            m.version = version;
            assert_eq!(validate(&m), Ok(()), "version {version}");
        }
        for version in [0i64, -1, 10, 42, 99] {
            let mut m = manifest.clone();
            m.version = version;
            assert_eq!(
                validate(&m),
                Err(ProjectError::Version(version)),
                "version {version}"
            );
        }
        let mut m = manifest.clone();
        m.format = "com.other.project".into();
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = manifest.clone();
        m.color_space = "Display P3".into();
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
    }

    #[test]
    fn dimension_resolution_and_layer_limits() {
        let mut m = base_manifest();
        m.width = 30_000;
        m.height = 30_000;
        assert_eq!(validate(&m), Ok(()));
        let mut m = m.clone();
        m.width = 30_001;
        assert_eq!(validate(&m), Err(ProjectError::TooLarge));
        let mut m = base_manifest();
        m.height = 0;
        assert_eq!(validate(&m), Err(ProjectError::TooLarge));
        let mut m = base_manifest();
        m.resolution = Some(9600.0);
        assert_eq!(validate(&m), Ok(()));
        let mut m = m.clone();
        m.resolution = Some(9601.0);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = base_manifest();
        m.resolution = Some(0.5);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = base_manifest();
        m.resolution = Some(f64::NAN);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = base_manifest();
        m.layers = (0..MAX_LAYERS).map(|_| blank_record()).collect();
        assert_eq!(validate(&m), Ok(()));
        let mut m = m.clone();
        m.layers.push(blank_record());
        assert_eq!(validate(&m), Err(ProjectError::TooLarge));
    }

    #[test]
    fn source_pixel_budget_accumulates() {
        let mut used = 0u64;
        // 30,000 x 3,333 = 99,990,000: 10,000 pixels still fit; 10,001 do not.
        assert_eq!(check_size(30_000, 3_333, &mut used), Ok(()));
        assert_eq!(used, 99_990_000);
        assert_eq!(check_size(10_000, 1, &mut used), Ok(()));
        assert_eq!(
            check_size(10_001, 1, &mut used),
            Err(ProjectError::TooLarge)
        );
        // A fresh budget rejects anything over 100 million at once.
        let mut fresh = 0u64;
        assert_eq!(
            check_size(30_000, 3_334, &mut fresh),
            Err(ProjectError::TooLarge)
        );
        // Per-side limits.
        let mut side = 0u64;
        assert_eq!(
            check_size(30_001, 1, &mut side),
            Err(ProjectError::TooLarge)
        );
        assert_eq!(check_size(0, 10, &mut side), Err(ProjectError::TooLarge));
        // Masks draw on a budget of their own.
        let mut masks = 0u64;
        assert_eq!(check_size(30_000, 3_333, &mut masks), Ok(()));
        assert_eq!(check_size(30_000, 3_333, &mut 0u64), Ok(()));
    }

    #[test]
    fn appearance_values_need_version_3_and_folders_version_8() {
        // Layers: opacity and blend mode from version 3.
        let mut m = base_manifest();
        m.version = 2;
        m.layers[0].opacity = Some(0.5);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = m.clone();
        m.layers[0].opacity = Some(1.0);
        m.layers[0].blend_mode = Some(LayerBlendMode::Multiply);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = m.clone();
        m.version = 3;
        assert_eq!(validate(&m), Ok(()));
        let mut m = m.clone();
        m.layers[0].opacity = Some(1.5);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        // Folders: Normal always; their own opacity from version 8.
        let mut m = base_manifest();
        m.version = 7;
        m.layers[0].is_group = Some(true);
        m.layers[0].opacity = Some(0.5);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = m.clone();
        m.version = 8;
        assert_eq!(validate(&m), Ok(()));
        let mut m = m.clone();
        m.layers[0].blend_mode = Some(LayerBlendMode::Multiply);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
    }

    #[test]
    fn mask_fields_per_version() {
        let id = Uuid::new_v4();
        let mut m = base_manifest();
        m.layers[0].id = id;
        m.layers[0].image_file = Some(image_file_name(id));
        // Version 3: no masks anywhere.
        m.version = 3;
        m.layers[0].mask_file = Some(mask_file_name(id));
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        // Version 4: layer masks.
        m.version = 4;
        assert_eq!(validate(&m), Ok(()));
        // A mask file must be exactly `<UUID>.mask.png`.
        let mut m = m.clone();
        m.layers[0].mask_file = Some(format!("{}.png", image_file_name(id)));
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = base_manifest();
        m.layers[0].mask_enabled = Some(true);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = base_manifest();
        m.layers[0].mask_placement = Some(transform((1.0, 2.0), (10.0, 10.0)));
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        // Groups carry masks only from version 6.
        let mut m = base_manifest();
        m.version = 5;
        m.layers[0].is_group = Some(true);
        m.layers[0].id = id;
        m.layers[0].mask_file = Some(mask_file_name(id));
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = m.clone();
        m.version = 6;
        assert_eq!(validate(&m), Ok(()));
        // An invalid placement is rejected even with a mask file.
        let mut m = m.clone();
        m.layers[0].mask_placement = Some(transform((0.0, 0.0), (0.0, 0.0)));
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        // maskLinked alone (older projects) is accepted.
        let mut m = base_manifest();
        m.layers[0].mask_linked = Some(true);
        assert_eq!(validate(&m), Ok(()));
    }

    #[test]
    fn live_mask_rules() {
        let build = |count: usize| -> ProjectManifest {
            let mut m = base_manifest();
            m.version = 5;
            m.layers = (0..count)
                .map(|_| ProjectLayerRecord {
                    is_group: Some(false),
                    ..blank_record()
                })
                .collect();
            m
        };
        let chain = |count: usize| -> ProjectManifest {
            let mut m = build(count);
            for index in 1..m.layers.len() {
                let source = m.layers[index - 1].id;
                m.layers[index].mask_source_id = Some(source);
            }
            m
        };
        // Version 4 cannot carry live masks.
        let mut m = build(1);
        m.version = 4;
        m.layers[0].mask_source_id = Some(Uuid::new_v4());
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        // A chain of 256 is fine; 257 is over the limit.
        assert_eq!(validate(&chain(256)), Ok(()));
        assert_eq!(validate(&chain(257)), Err(ProjectError::Invalid));
        // Self-links and cycles are rejected.
        let mut m = build(2);
        m.layers[0].mask_source_id = Some(m.layers[0].id);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = build(2);
        let first = m.layers[0].id;
        m.layers[0].mask_source_id = Some(m.layers[1].id);
        m.layers[1].mask_source_id = Some(first);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        // Sources must exist, be non-groups, and carry no adjustment.
        let mut m = build(2);
        let source = m.layers[0].id;
        m.layers[1].mask_source_id = Some(source);
        m.layers[0].is_group = Some(true);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = m.clone();
        m.layers[0].is_group = Some(false);
        m.layers[0].adjustment = Some(crate::adjustment::LayerAdjustment::new(
            AdjustmentKind::Invert,
        ));
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = m.clone();
        m.layers[0].adjustment = None;
        assert_eq!(validate(&m), Ok(()));
        let mut m = build(1);
        m.layers[0].mask_source_id = Some(Uuid::new_v4());
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
    }

    #[test]
    fn hierarchy_rules() {
        // A missing or non-group parent is rejected.
        let mut m = base_manifest();
        m.layers[0].parent_id = Some(Uuid::new_v4());
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = base_manifest();
        let parent = Uuid::new_v4();
        let mut group = blank_record();
        group.id = parent;
        group.is_group = Some(true);
        m.layers.insert(0, group);
        m.layers[1].parent_id = Some(parent);
        assert_eq!(validate(&m), Ok(()));
        let mut m = m.clone();
        m.layers[0].is_group = Some(false);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        // Groups carry no image.
        let mut m = base_manifest();
        m.layers[0].is_group = Some(true);
        m.layers[0].image_file = Some(image_file_name(m.layers[0].id));
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        // Duplicate ids are rejected.
        let mut m = base_manifest();
        let id = m.layers[0].id;
        m.layers.push(blank_record());
        let last = m.layers.len() - 1;
        m.layers[last].id = id;
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        // Parent cycles are rejected.
        let mut m = base_manifest();
        m.layers = vec![blank_record(), blank_record()];
        let a = m.layers[0].id;
        let b = m.layers[1].id;
        for layer in &mut m.layers {
            layer.is_group = Some(true);
        }
        m.layers[0].parent_id = Some(b);
        m.layers[1].parent_id = Some(a);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        // Nesting: 64 ancestor groups fit a leaf; 65 do not.
        let nest = |groups: usize| -> ProjectManifest {
            let mut m = base_manifest();
            m.layers.clear();
            let mut parent: Option<Uuid> = None;
            for _ in 0..groups {
                let mut group = blank_record();
                group.is_group = Some(true);
                group.parent_id = parent;
                parent = Some(group.id);
                m.layers.push(group);
            }
            let mut leaf = blank_record();
            leaf.parent_id = parent;
            m.layers.push(leaf);
            m
        };
        assert_eq!(validate(&nest(64)), Ok(()));
        assert_eq!(validate(&nest(65)), Err(ProjectError::Invalid));
    }

    #[test]
    fn version_one_is_flat() {
        let mut m = base_manifest();
        m.version = 1;
        m.layers[0].parent_id = Some(Uuid::new_v4());
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = base_manifest();
        m.version = 1;
        m.layers[0].is_group = Some(true);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
    }

    #[test]
    fn metadata_checks() {
        // Unknown active layer.
        let mut m = base_manifest();
        m.active_layer_id = Some(Uuid::new_v4());
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        // Wrong image filename (including path traversal).
        let mut m = base_manifest();
        m.layers[0].image_file = Some("../../outside.png".into());
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = base_manifest();
        m.layers[0].image_file = Some(format!("{}.PNG", m.layers[0].id));
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        // Names must not be blank or over 16 KiB of UTF-8.
        let mut m = base_manifest();
        m.layers[0].name = "   \n ".into();
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = base_manifest();
        m.layers[0].name = "x".repeat(16_385);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = base_manifest();
        m.layers[0].name = "x".repeat(16_384);
        assert_eq!(validate(&m), Ok(()));
        // Transforms must be valid.
        let mut m = base_manifest();
        m.layers[0].transform = transform((0.0, 0.0), (0.0, 0.0));
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = base_manifest();
        m.layers[0].transform = ManifestTransform {
            rotation: f64::NAN,
            ..transform((0.0, 0.0), (10.0, 10.0))
        };
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
    }

    #[test]
    fn text_and_adjustment_checks() {
        // Text needs pixels, no folder, no adjustment.
        let mut m = base_manifest();
        let id = m.layers[0].id;
        m.layers[0].image_file = Some(image_file_name(id));
        m.layers[0].text = Some(ManifestTextStyle {
            content: "Hello".into(),
            ..ManifestTextStyle::default()
        });
        assert_eq!(validate(&m), Ok(()));
        let mut m = m.clone();
        m.layers[0].image_file = None;
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = m.clone();
        m.layers[0].image_file = Some(image_file_name(id));
        m.layers[0].is_group = Some(true);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = m.clone();
        m.layers[0].is_group = Some(false);
        m.layers[0].adjustment = Some(crate::adjustment::LayerAdjustment::new(
            AdjustmentKind::Invert,
        ));
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        // Adjustments: version 7+, non-group layers without image pixels.
        let mut m = base_manifest();
        m.version = 6;
        m.layers[0].adjustment = Some(crate::adjustment::LayerAdjustment::new(
            AdjustmentKind::Invert,
        ));
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = base_manifest();
        m.layers[0].adjustment = Some(crate::adjustment::LayerAdjustment::new(
            AdjustmentKind::Invert,
        ));
        assert_eq!(validate(&m), Ok(()));
        let mut m = m.clone();
        let id = m.layers[0].id;
        m.layers[0].image_file = Some(image_file_name(id));
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = base_manifest();
        m.layers[0].is_group = Some(true);
        m.layers[0].adjustment = Some(crate::adjustment::LayerAdjustment::new(
            AdjustmentKind::Invert,
        ));
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        // Blur and noise adjustments need version 9.
        for kind in [
            AdjustmentKind::GaussianBlur,
            AdjustmentKind::MotionBlur,
            AdjustmentKind::AddNoise,
        ] {
            let mut m = base_manifest();
            m.version = 8;
            m.layers[0].adjustment = Some(crate::adjustment::LayerAdjustment::new(kind));
            assert_eq!(validate(&m), Err(ProjectError::Invalid), "{kind:?}");
            let mut m = m.clone();
            m.version = 9;
            assert_eq!(validate(&m), Ok(()), "{kind:?}");
        }
    }

    #[test]
    fn guide_checks() {
        let guide = ManifestGuide {
            id: Uuid::new_v4(),
            axis: GuideAxis::Horizontal,
            position: 12.5,
        };
        // Version 7 cannot carry guides.
        let mut m = base_manifest();
        m.version = 7;
        m.guides = Some(vec![guide]);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = base_manifest();
        m.guides = Some(vec![guide]);
        assert_eq!(validate(&m), Ok(()));
        // At most 1000 guides, unique ids, finite positions within a million.
        let mut m = base_manifest();
        m.guides = Some((0..1_001).map(|_| guide).collect());
        assert_eq!(validate(&m), Err(ProjectError::TooLarge));
        let mut m = base_manifest();
        m.guides = Some(vec![
            guide,
            ManifestGuide {
                id: guide.id,
                axis: GuideAxis::Vertical,
                position: 0.0,
            },
        ]);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = base_manifest();
        m.guides = Some(vec![ManifestGuide {
            position: f64::NAN,
            ..guide
        }]);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
        let mut m = base_manifest();
        m.guides = Some(vec![ManifestGuide {
            position: 1_000_001.0,
            ..guide
        }]);
        assert_eq!(validate(&m), Err(ProjectError::Invalid));
    }

    // MARK: package IO

    fn sample_document() -> (CanvasDocument, Uuid) {
        let mut doc = CanvasDocument::new(100, 80);
        doc.resolution = 144.0;
        let mut layer =
            ImageLayer::with_asset(ImportedImage::new(fixture(), "Fixture"), CGPoint::ZERO);
        layer.transform = LayerTransform {
            origin: CGPoint::new(-27.5, 88.25),
            size: CGSize::new(123.0, 47.0),
            rotation: 38.0,
            flip_x: true,
            flip_y: true,
            sampling: LayerSampling::Nearest,
        };
        layer.name = "Himmel \u{fc}ber Szene".to_string();
        layer.is_visible = false;
        let image_id = layer.id;
        doc.layers = vec![layer, ImageLayer::blank("Layer 2", doc.size())];
        (doc, image_id)
    }

    #[test]
    fn project_round_trip_survives_package_move() {
        let root = TempRoot::new("round-trip");
        let (doc, image_id) = sample_document();
        let mut data = project_from_document(&doc);
        data.manifest.active_layer_id = Some(image_id);
        let original = root.0.join("Original.comp");
        save(&original, &data).unwrap();
        // Moving the package (even deleting the source image) changes nothing.
        let moved = root.0.join("Moved.comp");
        fs::rename(&original, &moved).unwrap();
        let loaded = load(&moved).unwrap();
        let (reopened, active) = document_from_project(&loaded).unwrap();
        assert_eq!(reopened.id, doc.id);
        assert_eq!((reopened.width, reopened.height), (doc.width, doc.height));
        assert_eq!(reopened.resolution, doc.resolution);
        assert_eq!(
            reopened.layers.iter().map(|l| l.id).collect::<Vec<_>>(),
            doc.layers.iter().map(|l| l.id).collect::<Vec<_>>()
        );
        assert_eq!(
            reopened
                .layers
                .iter()
                .map(|l| l.name.clone())
                .collect::<Vec<_>>(),
            doc.layers
                .iter()
                .map(|l| l.name.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            reopened
                .layers
                .iter()
                .map(|l| l.is_visible)
                .collect::<Vec<_>>(),
            doc.layers.iter().map(|l| l.is_visible).collect::<Vec<_>>()
        );
        assert_eq!(
            reopened
                .layers
                .iter()
                .map(|l| l.transform)
                .collect::<Vec<_>>(),
            doc.layers.iter().map(|l| l.transform).collect::<Vec<_>>()
        );
        assert_eq!(active, Some(image_id));
        // The blank layer has no asset.
        assert!(reopened.layers[1].asset.is_none());
        let image = loaded.images.get(&image_id).unwrap();
        assert_eq!((image.width, image.height), (64, 32));
        assert!(image.pixel(0, 0)[0] as f64 / 255.0 > 0.95);
        assert_eq!(image.pixel(63, 0)[3], 0);
        // The package layout is manifest.json + images/.
        assert!(moved.join("manifest.json").is_file());
        assert!(
            moved
                .join("images")
                .join(image_file_name(image_id))
                .is_file()
        );
    }

    #[test]
    fn a_dimmed_folder_saves_and_reopens() {
        let root = TempRoot::new("dimmed-folder");
        let mut doc = CanvasDocument::new(100, 80);
        let mut child = ImageLayer::blank("Child", doc.size());
        let mut folder = ImageLayer::blank("Folder 1", doc.size());
        folder.is_group = true;
        folder.opacity = 0.5;
        child.parent_id = Some(folder.id);
        let folder_id = folder.id;
        doc.layers = vec![folder, child];
        let data = project_from_document(&doc);
        let url = root.0.join("Dimmed.comp");
        save(&url, &data).unwrap();
        let loaded = load(&url).unwrap();
        let saved = loaded
            .manifest
            .layers
            .iter()
            .find(|l| l.is_group == Some(true))
            .unwrap();
        assert_eq!(saved.id, folder_id);
        assert_eq!(saved.opacity, Some(0.5));
    }

    #[test]
    fn overwrite_replaces_package_and_drops_removed_assets() {
        let root = TempRoot::new("overwrite");
        let mut doc = CanvasDocument::new(100, 80);
        doc.layers = vec![ImageLayer::with_asset(
            ImportedImage::new(fixture(), "Fixture"),
            CGPoint::ZERO,
        )];
        let destination = root.0.join("Overwrite.comp");
        save(&destination, &project_from_document(&doc)).unwrap();
        // Replace every layer with a blank one and save again.
        let mut doc = CanvasDocument::new(100, 80);
        doc.layers = vec![ImageLayer::blank("Layer", doc.size())];
        save(&destination, &project_from_document(&doc)).unwrap();
        let loaded = load(&destination).unwrap();
        assert!(loaded.images.is_empty());
        assert_eq!(loaded.manifest.layers.len(), 1);
        let images: Vec<_> = fs::read_dir(destination.join("images")).unwrap().collect();
        assert!(images.is_empty(), "{images:?}");
    }

    #[test]
    fn failed_save_preserves_previously_saved_package() {
        let root = TempRoot::new("failed-save");
        let mut doc = CanvasDocument::new(100, 80);
        doc.layers = vec![ImageLayer::blank("Layer", doc.size())];
        let data = project_from_document(&doc);
        let url = root.0.join("Safe.comp");
        save(&url, &data).unwrap();
        let original = fs::read(url.join("manifest.json")).unwrap();
        let mut invalid = data.clone();
        invalid.manifest.version = 99;
        assert_eq!(save(&url, &invalid), Err(ProjectError::Version(99)));
        assert_eq!(fs::read(url.join("manifest.json")).unwrap(), original);
        assert_eq!(load(&url).unwrap().manifest.layers.len(), 1);
        // A regular file in the way of the destination fails the save.
        let blocker = root.0.join("not-a-directory");
        fs::write(&blocker, [1u8]).unwrap();
        assert_eq!(
            save(&blocker.join("CannotSave.comp"), &data),
            Err(ProjectError::Encode)
        );
        assert_eq!(fs::read(url.join("manifest.json")).unwrap(), original);
    }

    #[test]
    fn unsupported_corrupt_and_unsafe_metadata_are_rejected() {
        let root = TempRoot::new("corrupt");
        let (doc, _) = sample_document();
        let data = project_from_document(&doc);
        let url = root.0.join("Invalid.comp");
        save(&url, &data).unwrap();
        let metadata_path = url.join("manifest.json");
        let mut metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
        metadata["version"] = serde_json::Value::from(42);
        fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        assert_eq!(load(&url), Err(ProjectError::Version(42)));
        metadata["layers"][0]["imageFile"] = serde_json::Value::from("../../outside.png");
        metadata["version"] = serde_json::Value::from(CURRENT_VERSION);
        fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        assert_eq!(load(&url), Err(ProjectError::Invalid));
        fs::write(&metadata_path, b"not json").unwrap();
        assert_eq!(load(&url), Err(ProjectError::Invalid));
        // A regular file is not a package.
        let file_package = root.0.join("flat.comp");
        fs::write(&file_package, [1u8]).unwrap();
        assert_eq!(load(&file_package), Err(ProjectError::Invalid));
    }

    #[test]
    fn missing_embedded_image_is_rejected() {
        let root = TempRoot::new("missing-image");
        let (doc, _) = sample_document();
        let data = project_from_document(&doc);
        let url = root.0.join("Missing.comp");
        save(&url, &data).unwrap();
        let filename = data.manifest.layers[0]
            .image_file
            .clone()
            .expect("layer has an image");
        fs::remove_file(url.join("images").join(&filename)).unwrap();
        assert_eq!(load(&url), Err(ProjectError::MissingImage));
        // A mask going missing is rejected too.
        let mut masked = data.clone();
        let layer_id = masked.manifest.layers[0].id;
        masked.manifest.layers[0].mask_file = Some(mask_file_name(layer_id));
        masked.manifest.layers[0].mask_enabled = Some(true);
        masked.masks.insert(layer_id, GrayMask::solid(true));
        let url = root.0.join("MissingMask.comp");
        save(&url, &masked).unwrap();
        fs::remove_file(url.join("images").join(mask_file_name(layer_id))).unwrap();
        assert_eq!(load(&url), Err(ProjectError::MissingImage));
        // And a manifest record whose mask asset is missing is an error on save.
        let mut inconsistent = masked.clone();
        inconsistent.masks.remove(&layer_id);
        assert_eq!(
            save(&root.0.join("Bad.comp"), &inconsistent),
            Err(ProjectError::MissingImage)
        );
    }

    /// A PNG whose IHDR declares `width` x `height` (RGBA, 8-bit) with an
    /// empty IDAT. Enough for the decoder to report dimensions, which is all
    /// the size checks need before rejecting.
    fn png_header_only(width: u32, height: u32) -> Vec<u8> {
        fn crc32(data: &[u8]) -> u32 {
            let mut table = [0u32; 256];
            for (index, entry) in table.iter_mut().enumerate() {
                let mut c = index as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 {
                        0xEDB8_8320 ^ (c >> 1)
                    } else {
                        c >> 1
                    };
                }
                *entry = c;
            }
            let mut crc = 0xFFFF_FFFFu32;
            for &byte in data {
                crc = table[((crc ^ byte as u32) & 0xFF) as usize] ^ (crc >> 8);
            }
            crc ^ 0xFFFF_FFFF
        }
        let mut out = vec![137, 80, 78, 71, 13, 10, 26, 10];
        for (kind, body) in [
            (&b"IHDR"[..], {
                let mut body = Vec::new();
                body.extend_from_slice(&width.to_be_bytes());
                body.extend_from_slice(&height.to_be_bytes());
                body.extend_from_slice(&[8, 6, 0, 0, 0]);
                body
            }),
            (
                &b"IDAT"[..],
                vec![0x78, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
            ),
            (&b"IEND"[..], Vec::new()),
        ] {
            let mut chunk = kind.to_vec();
            chunk.extend_from_slice(&body);
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            out.extend_from_slice(&chunk);
            out.extend_from_slice(&crc32(&chunk).to_be_bytes());
        }
        out
    }

    #[test]
    fn oversize_assets_and_manifests_are_rejected() {
        let root = TempRoot::new("oversize");
        // One side beyond 30,000 pixels.
        let mut bitmap = Bitmap::new(30_001, 1);
        bitmap.set_pixel(0, 0, [255, 0, 0, 255]);
        let mut doc = CanvasDocument::new(100, 100);
        doc.layers = vec![ImageLayer::with_asset(
            ImportedImage::new(bitmap, "Wide"),
            CGPoint::ZERO,
        )];
        assert_eq!(
            save(&root.0.join("Wide.comp"), &project_from_document(&doc)),
            Err(ProjectError::TooLarge)
        );
        // Loading an asset whose declared pixels bust the 100-megapixel budget
        // is rejected before anything is decoded.
        let write_manifest_and_assets =
            |dir: &Path, layers: &[ProjectLayerRecord], assets: &[(&str, Vec<u8>)]| {
                fs::create_dir_all(dir.join("images")).unwrap();
                let mut m = base_manifest();
                m.width = 100;
                m.height = 100;
                m.layers = layers.to_vec();
                fs::write(
                    dir.join("manifest.json"),
                    serde_json::to_vec_pretty(&m).unwrap(),
                )
                .unwrap();
                for (name, bytes) in assets {
                    fs::write(dir.join("images").join(name), bytes).unwrap();
                }
            };
        let huge = root.0.join("Huge.comp");
        let u1 = Uuid::new_v4();
        let mut record = blank_record();
        record.id = u1;
        record.image_file = Some(image_file_name(u1));
        write_manifest_and_assets(
            &huge,
            &[record],
            &[(&image_file_name(u1), png_header_only(30_000, 3_334))],
        );
        assert_eq!(load(&huge), Err(ProjectError::TooLarge));
        // A manifest over 4 MiB is rejected on save (names may be 16 KiB).
        let mut doc = CanvasDocument::new(100, 100);
        let mut layers = Vec::new();
        for index in 0..300 {
            let mut layer = ImageLayer::blank(format!("Layer {index}"), doc.size());
            layer.name = "x".repeat(16_384);
            layers.push(layer);
        }
        doc.layers = layers;
        assert_eq!(
            save(&root.0.join("Giant.comp"), &project_from_document(&doc)),
            Err(ProjectError::TooLarge)
        );
    }

    #[test]
    fn asset_files_must_stay_inside_the_package() {
        let root = TempRoot::new("escape");
        let (doc, image_id) = sample_document();
        let url = root.0.join("Escape.comp");
        save(&url, &project_from_document(&doc)).unwrap();
        // A symlink pointing outside the package is not a regular file.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                root.0.join("outside.png"),
                url.join("images").join(image_file_name(image_id)),
            )
            .unwrap();
            assert_eq!(load(&url), Err(ProjectError::TooLarge));
        }
        let _ = image_id;
    }

    // MARK: conversion-level rules

    #[test]
    fn opacity_and_blend_default_per_version() {
        let mut manifest = base_manifest();
        manifest.version = 2;
        let data = ProjectData {
            manifest: manifest.clone(),
            images: HashMap::new(),
            masks: HashMap::new(),
        };
        let (doc, _) = document_from_project(&data).unwrap();
        assert_eq!(doc.layers[0].opacity, 1.0);
        assert_eq!(doc.layers[0].blend_mode, LayerBlendMode::Normal);
        manifest.version = 3;
        manifest.layers[0].opacity = Some(0.25);
        manifest.layers[0].blend_mode = Some(LayerBlendMode::Multiply);
        let data = ProjectData {
            manifest,
            images: HashMap::new(),
            masks: HashMap::new(),
        };
        let (doc, _) = document_from_project(&data).unwrap();
        assert_eq!(doc.layers[0].opacity, 0.25);
        assert_eq!(doc.layers[0].blend_mode, LayerBlendMode::Multiply);
        // Missing resolution defaults to 72 pixels/inch.
        assert_eq!(doc.resolution, 72.0);
    }

    #[test]
    fn lowercase_uuid_json_from_other_writers_loads() {
        let id = Uuid::new_v4();
        let lower = id.to_string();
        let json = format!(
            r#"{{"format":"com.compositor.project","version":1,"colorSpace":"sRGB",
            "documentID":"{l}","width":4,"height":4,"layers":[{{"id":"{l}",
            "name":"L","isVisible":true,
            "transform":{{"origin":{{"x":0.0,"y":0.0}},"size":{{"width":4.0,"height":4.0}},
            "rotation":0.0,"flipX":false,"flipY":false,"sampling":"High quality"}}}}]}}"#,
            l = lower
        );
        let manifest: ProjectManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest.document_id, Uuid::parse_str(&lower).unwrap());
        let data = ProjectData {
            manifest,
            images: HashMap::new(),
            masks: HashMap::new(),
        };
        let (doc, _) = document_from_project(&data).unwrap();
        assert_eq!(doc.width, 4);
        assert_eq!(doc.layers[0].id, id);
    }

    #[test]
    fn guides_round_trip() {
        let root = TempRoot::new("guides");
        let mut doc = CanvasDocument::new(100, 80);
        doc.guides = vec![
            CanvasGuide::new(GuideAxis::Horizontal, 12.5),
            CanvasGuide::new(GuideAxis::Vertical, 33.0),
        ];
        doc.layers = vec![ImageLayer::blank("Layer", doc.size())];
        let data = project_from_document(&doc);
        assert!(data.manifest.guides.is_some());
        let url = root.0.join("Guides.comp");
        save(&url, &data).unwrap();
        let loaded = load(&url).unwrap();
        let (doc2, _) = document_from_project(&loaded).unwrap();
        assert_eq!(doc2.guides, doc.guides);
    }
}
