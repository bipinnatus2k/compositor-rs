//! Port of `SubjectRemoval.swift` — to be completed by the tools pass.
#![allow(unused)]

use crate::filters::FilterSettings;
use crate::raster::Bitmap;

pub fn run(image: &Bitmap, settings: &FilterSettings) -> Bitmap {
    // TODO(tools-pass): subject mask + guided matte refine + mask-as-layer-mask.
    image.clone()
}
