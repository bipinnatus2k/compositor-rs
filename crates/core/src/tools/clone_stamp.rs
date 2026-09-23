//! Placeholder — port of `CloneStamp.swift` settings lands with the tools pass.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// Clone Stamp settings (`CloneStamp.swift`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct CloneSettings {
    /// Aligned strokes keep the offset from brush to source they started with.
    #[serde(default = "default_true")]
    pub aligned: bool,
    /// Sample one layer or the composite of all of them.
    #[serde(default)]
    pub sample_all_layers: bool,
}

fn default_true() -> bool {
    true
}
