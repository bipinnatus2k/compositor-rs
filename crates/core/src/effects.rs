//! Port of `Compositor/Document/LayerEffects.swift` and
//! `Rendering/GuidedMatte.swift`-based effect rendering.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::document::PaletteColor;
use crate::geometry::{CGFloat, CGPoint, CGRect, CGSize};
use crate::raster::{gaussian_blur_gray, Bitmap, GrayMask};
use crate::transform::LayerTransform;

/// A line drawn around what the layer shows, outside its edge or inside it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StrokeEffect {
    /// Missing in older projects means visible.
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default = "default_stroke_size")]
    pub size: CGFloat,
    #[serde(default)]
    pub red: CGFloat,
    #[serde(default)]
    pub green: CGFloat,
    #[serde(default)]
    pub blue: CGFloat,
    #[serde(default = "default_stroke_opacity")]
    pub opacity: f64,
    #[serde(default)]
    pub inside: bool,
}

fn default_stroke_size() -> CGFloat {
    4.0
}

fn default_stroke_opacity() -> f64 {
    1.0
}

impl Default for StrokeEffect {
    fn default() -> Self {
        StrokeEffect {
            enabled: None,
            size: 4.0,
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            opacity: 1.0,
            inside: false,
        }
    }
}

impl StrokeEffect {
    pub const MAX_SIZE: CGFloat = 500.0;

    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn color(&self) -> PaletteColor {
        PaletteColor::new(self.red, self.green, self.blue)
    }

    pub fn is_valid(&self) -> bool {
        self.size.is_finite()
            && (0.0..=Self::MAX_SIZE).contains(&self.size)
            && self.opacity.is_finite()
            && (0.0..=1.0).contains(&self.opacity)
            && [self.red, self.green, self.blue]
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    }
}

macro_rules! shadow_like {
    ($name:ident, $default_distance:expr, $default_blur:expr, $default_opacity:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
        pub struct $name {
            #[serde(default)]
            pub enabled: Option<bool>,
            /// Degrees counterclockwise from the right, as Photoshop's dial.
            #[serde(default = "default_angle")]
            pub angle: CGFloat,
            #[serde(default = $default_distance)]
            pub distance: CGFloat,
            #[serde(default = $default_blur)]
            pub blur: CGFloat,
            #[serde(default)]
            pub red: CGFloat,
            #[serde(default)]
            pub green: CGFloat,
            #[serde(default)]
            pub blue: CGFloat,
            #[serde(default = $default_opacity)]
            pub opacity: f64,
        }

        impl $name {
            pub fn is_enabled(&self) -> bool {
                self.enabled.unwrap_or(true)
            }

            pub fn color(&self) -> PaletteColor {
                PaletteColor::new(self.red, self.green, self.blue)
            }

            /// Where the shadow sits, in layer pixels (y grows downward).
            pub fn offset(&self) -> CGSize {
                let radians = self.angle * std::f64::consts::PI / 180.0;
                // The shadow falls away from the light, and a layer's pixels
                // count y downward.
                CGSize::new(-radians.cos() * self.distance, radians.sin() * self.distance)
            }

            pub fn is_valid(&self) -> bool {
                [self.angle, self.distance, self.blur].iter().all(|v| v.is_finite())
                    && (-360.0..=360.0).contains(&self.angle)
                    && (0.0..=5000.0).contains(&self.distance)
                    && (0.0..=500.0).contains(&self.blur)
                    && self.opacity.is_finite()
                    && (0.0..=1.0).contains(&self.opacity)
                    && [self.red, self.green, self.blue]
                        .iter()
                        .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            }
        }
    };
}

shadow_like!(ShadowEffect, "default_shadow_distance", "default_shadow_blur", "default_shadow_opacity");
shadow_like!(InnerShadowEffect, "default_inner_distance", "default_inner_blur", "default_inner_opacity");

fn default_shadow_distance() -> CGFloat {
    20.0
}
fn default_shadow_blur() -> CGFloat {
    20.0
}
fn default_shadow_opacity() -> f64 {
    0.5
}
fn default_inner_distance() -> CGFloat {
    10.0
}
fn default_inner_blur() -> CGFloat {
    10.0
}
fn default_inner_opacity() -> f64 {
    0.5
}

impl Default for ShadowEffect {
    fn default() -> Self {
        ShadowEffect {
            enabled: None,
            angle: 90.0,
            distance: 20.0,
            blur: 20.0,
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            opacity: 0.5,
        }
    }
}

impl Default for InnerShadowEffect {
    fn default() -> Self {
        InnerShadowEffect {
            enabled: None,
            angle: 90.0,
            distance: 10.0,
            blur: 10.0,
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            opacity: 0.5,
        }
    }
}

/// A flat color over everything the layer shows.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColorOverlayEffect {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub red: CGFloat,
    #[serde(default)]
    pub green: CGFloat,
    #[serde(default)]
    pub blue: CGFloat,
    #[serde(default)]
    pub opacity: f64,
}

impl Default for ColorOverlayEffect {
    fn default() -> Self {
        ColorOverlayEffect {
            enabled: None,
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            opacity: 1.0,
        }
    }
}

impl ColorOverlayEffect {
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn color(&self) -> PaletteColor {
        PaletteColor::new(self.red, self.green, self.blue)
    }

    pub fn is_valid(&self) -> bool {
        self.opacity.is_finite()
            && (0.0..=1.0).contains(&self.opacity)
            && [self.red, self.green, self.blue]
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    }
}

macro_rules! glow_like {
    ($name:ident, $default_size:expr, $default_opacity:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
        pub struct $name {
            #[serde(default)]
            pub enabled: Option<bool>,
            #[serde(default = $default_size)]
            pub size: CGFloat,
            #[serde(default = "default_white_r")]
            pub red: CGFloat,
            #[serde(default = "default_white_r")]
            pub green: CGFloat,
            #[serde(default = "default_white_r")]
            pub blue: CGFloat,
            #[serde(default = $default_opacity)]
            pub opacity: f64,
        }

        impl $name {
            pub fn is_enabled(&self) -> bool {
                self.enabled.unwrap_or(true)
            }

            pub fn color(&self) -> PaletteColor {
                PaletteColor::new(self.red, self.green, self.blue)
            }

            pub fn is_valid(&self) -> bool {
                self.size.is_finite()
                    && (0.0..=500.0).contains(&self.size)
                    && self.opacity.is_finite()
                    && (0.0..=1.0).contains(&self.opacity)
                    && [self.red, self.green, self.blue]
                        .iter()
                        .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            }
        }
    };
}

glow_like!(OuterGlowEffect, "default_outer_size", "default_outer_opacity");
glow_like!(InnerGlowEffect, "default_inner_glow_size", "default_inner_glow_opacity");

fn default_outer_size() -> CGFloat {
    20.0
}
fn default_outer_opacity() -> f64 {
    0.75
}
fn default_inner_glow_size() -> CGFloat {
    10.0
}
fn default_inner_glow_opacity() -> f64 {
    0.75
}

impl Default for OuterGlowEffect {
    fn default() -> Self {
        OuterGlowEffect {
            enabled: None,
            size: 20.0,
            red: 1.0,
            green: 1.0,
            blue: 1.0,
            opacity: 0.75,
        }
    }
}

impl Default for InnerGlowEffect {
    fn default() -> Self {
        InnerGlowEffect {
            enabled: None,
            size: 10.0,
            red: 1.0,
            green: 1.0,
            blue: 1.0,
            opacity: 0.75,
        }
    }
}

/// What a layer draws around itself, kept with the layer so the pixels are
/// never touched.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct LayerEffects {
    #[serde(default)]
    pub stroke: Option<StrokeEffect>,
    #[serde(default)]
    pub shadow: Option<ShadowEffect>,
    #[serde(default)]
    pub color_overlay: Option<ColorOverlayEffect>,
    #[serde(default)]
    pub inner_shadow: Option<InnerShadowEffect>,
    #[serde(default)]
    pub outer_glow: Option<OuterGlowEffect>,
    #[serde(default)]
    pub inner_glow: Option<InnerGlowEffect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerEffectKind {
    Stroke,
    Shadow,
    ColorOverlay,
    InnerShadow,
    OuterGlow,
    InnerGlow,
}

impl LayerEffectKind {
    pub const ALL: [LayerEffectKind; 6] = [
        LayerEffectKind::Stroke,
        LayerEffectKind::Shadow,
        LayerEffectKind::ColorOverlay,
        LayerEffectKind::InnerShadow,
        LayerEffectKind::OuterGlow,
        LayerEffectKind::InnerGlow,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            LayerEffectKind::Stroke => "Stroke",
            LayerEffectKind::Shadow => "Drop Shadow",
            LayerEffectKind::ColorOverlay => "Color Overlay",
            LayerEffectKind::InnerShadow => "Inner Shadow",
            LayerEffectKind::OuterGlow => "Outer Glow",
            LayerEffectKind::InnerGlow => "Inner Glow",
        }
    }
}

impl LayerEffects {
    pub fn is_empty(&self) -> bool {
        self.stroke.is_none()
            && self.shadow.is_none()
            && self.color_overlay.is_none()
            && self.inner_shadow.is_none()
            && self.outer_glow.is_none()
            && self.inner_glow.is_none()
    }

    pub fn is_valid(&self) -> bool {
        self.stroke.map(|v| v.is_valid()).unwrap_or(true)
            && self.shadow.map(|v| v.is_valid()).unwrap_or(true)
            && self.color_overlay.map(|v| v.is_valid()).unwrap_or(true)
            && self.inner_shadow.map(|v| v.is_valid()).unwrap_or(true)
            && self.outer_glow.map(|v| v.is_valid()).unwrap_or(true)
            && self.inner_glow.map(|v| v.is_valid()).unwrap_or(true)
    }

    pub fn kinds(&self) -> Vec<LayerEffectKind> {
        LayerEffectKind::ALL
            .iter()
            .copied()
            .filter(|k| self.contains(*k))
            .collect()
    }

    pub fn contains(&self, kind: LayerEffectKind) -> bool {
        match kind {
            LayerEffectKind::Stroke => self.stroke.is_some(),
            LayerEffectKind::Shadow => self.shadow.is_some(),
            LayerEffectKind::ColorOverlay => self.color_overlay.is_some(),
            LayerEffectKind::InnerShadow => self.inner_shadow.is_some(),
            LayerEffectKind::OuterGlow => self.outer_glow.is_some(),
            LayerEffectKind::InnerGlow => self.inner_glow.is_some(),
        }
    }

    pub fn is_enabled(&self, kind: LayerEffectKind) -> bool {
        match kind {
            LayerEffectKind::Stroke => self.stroke.map(|s| s.is_enabled()).unwrap_or(false),
            LayerEffectKind::Shadow => self.shadow.map(|s| s.is_enabled()).unwrap_or(false),
            LayerEffectKind::ColorOverlay => self.color_overlay.map(|s| s.is_enabled()).unwrap_or(false),
            LayerEffectKind::InnerShadow => self.inner_shadow.map(|s| s.is_enabled()).unwrap_or(false),
            LayerEffectKind::OuterGlow => self.outer_glow.map(|s| s.is_enabled()).unwrap_or(false),
            LayerEffectKind::InnerGlow => self.inner_glow.map(|s| s.is_enabled()).unwrap_or(false),
        }
    }

    /// The effect's own color.
    pub fn color(&self, kind: LayerEffectKind) -> Option<PaletteColor> {
        match kind {
            LayerEffectKind::Stroke => self.stroke.map(|s| s.color()),
            LayerEffectKind::Shadow => self.shadow.map(|s| s.color()),
            LayerEffectKind::ColorOverlay => self.color_overlay.map(|s| s.color()),
            LayerEffectKind::InnerShadow => self.inner_shadow.map(|s| s.color()),
            LayerEffectKind::OuterGlow => self.outer_glow.map(|s| s.color()),
            LayerEffectKind::InnerGlow => self.inner_glow.map(|s| s.color()),
        }
    }

    pub fn set_color(&mut self, color: PaletteColor, kind: LayerEffectKind) {
        macro_rules! set {
            ($effect:expr) => {
                if let Some(effect) = $effect.as_mut() {
                    effect.red = color.red;
                    effect.green = color.green;
                    effect.blue = color.blue;
                }
            };
        }
        match kind {
            LayerEffectKind::Stroke => set!(self.stroke),
            LayerEffectKind::Shadow => set!(self.shadow),
            LayerEffectKind::ColorOverlay => set!(self.color_overlay),
            LayerEffectKind::InnerShadow => set!(self.inner_shadow),
            LayerEffectKind::OuterGlow => set!(self.outer_glow),
            LayerEffectKind::InnerGlow => set!(self.inner_glow),
        }
    }

    pub fn remove(&mut self, kind: LayerEffectKind) {
        match kind {
            LayerEffectKind::Stroke => self.stroke = None,
            LayerEffectKind::Shadow => self.shadow = None,
            LayerEffectKind::ColorOverlay => self.color_overlay = None,
            LayerEffectKind::InnerShadow => self.inner_shadow = None,
            LayerEffectKind::OuterGlow => self.outer_glow = None,
            LayerEffectKind::InnerGlow => self.inner_glow = None,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool, kind: LayerEffectKind) {
        macro_rules! set {
            ($effect:expr) => {
                if let Some(effect) = $effect.as_mut() {
                    effect.enabled = Some(enabled);
                }
            };
        }
        match kind {
            LayerEffectKind::Stroke => set!(self.stroke),
            LayerEffectKind::Shadow => set!(self.shadow),
            LayerEffectKind::ColorOverlay => set!(self.color_overlay),
            LayerEffectKind::InnerShadow => set!(self.inner_shadow),
            LayerEffectKind::OuterGlow => set!(self.outer_glow),
            LayerEffectKind::InnerGlow => set!(self.inner_glow),
        }
    }

    /// Only the enabled effects.
    pub fn visible(&self) -> LayerEffects {
        LayerEffects {
            stroke: self.stroke.filter(|s| s.is_enabled()),
            shadow: self.shadow.filter(|s| s.is_enabled()),
            color_overlay: self.color_overlay.filter(|s| s.is_enabled()),
            inner_shadow: self.inner_shadow.filter(|s| s.is_enabled()),
            outer_glow: self.outer_glow.filter(|s| s.is_enabled()),
            inner_glow: self.inner_glow.filter(|s| s.is_enabled()),
        }
    }
}

fn default_angle() -> CGFloat {
    90.0
}

fn default_white_r() -> CGFloat {
    1.0
}

/// The gray levels of a mask as 0–1 floats.
fn mask_levels(mask: &GrayMask) -> Vec<f32> {
    mask.data.iter().map(|v| *v as f32 / 255.0).collect()
}

/// 0–1 levels back to a gray mask of the given size.
fn levels_mask(levels: &[f32], width: usize, height: usize) -> GrayMask {
    let mut out = GrayMask::new(width, height);
    for (i, v) in levels.iter().take(width * height).enumerate() {
        out.data[i] = (v * 255.0 + 0.5) as u8;
    }
    out
}

/// The largest (or smallest) value within `reach` on each side: two
/// sliding-window passes (`LayerEffectsRenderer.extreme`).
pub fn extreme(source: &[f32], width: usize, height: usize, reach: usize, smallest: bool) -> Vec<f32> {
    if width == 0 || height == 0 || source.len() != width * height {
        return Vec::new();
    }
    let radius = reach;
    let mut pass = vec![0.0f32; source.len()];
    let mut result = vec![0.0f32; source.len()];
    let mut queue = vec![0usize; width.max(height)];
    let mut sweep = |input: &[f32], output: &mut [f32], lines: usize, count: usize, line_step: usize, element_step: usize| {
        for line in 0..lines {
            let base = line * line_step;
            let mut head = 0usize;
            let mut tail = 0usize;
            let mut next = 0usize;
            for center in 0..count {
                while next <= (count - 1).min(center + radius) {
                    let value = input[base + next * element_step];
                    while tail > head {
                        let previous = input[base + queue[tail - 1] * element_step];
                        if if smallest { previous < value } else { previous > value } {
                            break;
                        }
                        tail -= 1;
                    }
                    queue[tail] = next;
                    tail += 1;
                    next += 1;
                }
                while head < tail && queue[head] + radius < center {
                    head += 1;
                }
                let outside = center < radius || center + radius >= count;
                output[base + center * element_step] =
                    if smallest && outside { 0.0 } else { input[base + queue[head] * element_step] };
            }
        }
    };
    sweep(source, &mut pass, height, width, width, 1);
    sweep(&pass, &mut result, width, height, 1, width);
    result
}

/// The layer's shape alpha as a gray coverage in a larger canvas, optionally blurred.
fn shape_coverage(image: &Bitmap, placed: CGRect, width: usize, height: usize, blur: CGFloat) -> GrayMask {
    let mut mask = GrayMask::new(width, height);
    // The source's alpha, drawn into the placed rect (nearest, as BrushRaster.draw).
    if placed.is_null() || placed.is_empty() {
        return mask;
    }
    for y in 0..height {
        let v = (y as f64 + 0.5 - placed.min_y()) / placed.height();
        if v < 0.0 || v >= 1.0 {
            continue;
        }
        for x in 0..width {
            let u = (x as f64 + 0.5 - placed.min_x()) / placed.width();
            if u < 0.0 || u >= 1.0 {
                continue;
            }
            let sx = (u * image.width as f64) as usize;
            let sy = (v * image.height as f64) as usize;
            mask.data[y * width + x] = image.pixel(sx, sy)[3];
        }
    }
    if blur > 0.0 {
        mask = gaussian_blur_gray(&mask, blur / 2.0);
    }
    mask
}

/// `LayerEffectsRenderer.render`: `image` with `effects` around it, on a
/// canvas grown by the effect margin. Returns the composited bitmap and inset.
pub fn render_effects(image: &Bitmap, mask: Option<&GrayMask>, effects: &LayerEffects) -> (Bitmap, CGFloat) {
    let effects = effects.visible();
    let inset = effect_margin(&effects);
    let width = image.width + inset as usize * 2;
    let height = image.height + inset as usize * 2;
    // The 100MP result guard (`LayerEffects.swift:441`): beyond it the layer
    // draws without its effects rather than exhausting memory.
    if width * height > 100_000_000 {
        return (image.clone(), 0.0);
    }
    // The layer as it is shown: its pixels through its mask.
    let shown = match mask {
        None => image.clone(),
        Some(m) => {
            let mut result = image.clone();
            debug_assert_eq!((m.width, m.height), (image.width, image.height));
            // Release safety: a mismatched mask (e.g. a 1x1 solid mask under a
            // larger layer) must not index out of bounds — treat a smaller
            // mask as its clamped extension, ignore out-of-range entries.
            let mw = m.width.min(result.width);
            let mh = m.height.min(result.height);
            for y in 0..mh {
                for x in 0..mw {
                    let i = y * result.width + x;
                    let coverage = m.data[y * m.width + x] as u32;
                    if coverage == 255 {
                        continue;
                    }
                    let o = i * 4;
                    if coverage == 0 {
                        result.data[o..o + 4].copy_from_slice(&[0, 0, 0, 0]);
                    } else {
                        for c in 0..4 {
                            result.data[o + c] = (result.data[o + c] as u32 * coverage / 255) as u8;
                        }
                    }
                }
            }
            result
        }
    };
    let placed = CGRect::new(inset, inset, image.width as f64, image.height as f64);
    let mut canvas = Bitmap::new(width, height);
    let full = CGRect::new(0.0, 0.0, width as f64, height as f64);

    let fill = |canvas: &mut Bitmap, color: PaletteColor, alpha: f64, coverage: &GrayMask| {
        let cr = (color.red * 255.0).round() as u8;
        let cg = (color.green * 255.0).round() as u8;
        let cb = (color.blue * 255.0).round() as u8;
        let opacity = alpha as f32;
        for i in 0..(canvas.width * canvas.height) {
            let cov = coverage.data[i] as f32 / 255.0 * opacity;
            if cov <= 0.0 {
                continue;
            }
            let o = i * 4;
            // Paint the straight color through the coverage, source-over.
            let sa = cov;
            let da = canvas.data[o + 3] as f32 / 255.0;
            let out_a = sa + da * (1.0 - sa);
            if out_a <= 0.0 {
                continue;
            }
            for c in 0..3 {
                let sc = match c {
                    0 => cr,
                    1 => cg,
                    _ => cb,
                } as f32 / 255.0;
                // Coverage multiplies alpha only; color stays as given (as
                // BrushRaster.fill does: the color is not premultiplied by the
                // gray coverage twice).
                let dc = canvas.data[o + c] as f32 / 255.0 / da.max(f32::EPSILON);
                let value = (sc * sa + dc * da * (1.0 - sa)) / out_a;
                canvas.data[o + c] = (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
            canvas.data[o + 3] = (out_a * 255.0 + 0.5) as u8;
        }
    };

    if let Some(shadow) = effects.shadow {
        if shadow.opacity > 0.0 {
            let offset = shadow.offset();
            let alpha = shape_coverage(
                &shown,
                placed.offset_by(offset.width, offset.height),
                width,
                height,
                shadow.blur,
            );
            fill(&mut canvas, shadow.color(), shadow.opacity, &alpha);
        }
    }
    if let Some(glow) = effects.outer_glow {
        if glow.opacity > 0.0 {
            let shape = shape_coverage(&shown, placed, width, height, 0.0);
            let soft = shape_coverage(&shown, placed, width, height, glow.size);
            let mut inside = mask_levels(&soft);
            let outside = mask_levels(&shape);
            for (i, v) in inside.iter_mut().enumerate() {
                *v = (*v * (1.0 - outside[i])).clamp(0.0, 1.0);
            }
            fill(&mut canvas, glow.color(), glow.opacity, &levels_mask(&inside, width, height));
        }
    }
    // An outside stroke sits behind the layer's own pixels; an inside one is
    // drawn over them.
    let stroke = effects.stroke.filter(|s| s.size > 0.0 && s.opacity > 0.0);
    let mut draw_stroke = |canvas: &mut Bitmap, stroke: &StrokeEffect| {
        let shape = shape_coverage(&shown, placed, width, height, 0.0);
        let mut levels = mask_levels(&shape);
        let reach = (stroke.size.round() as usize).max(1);
        let moved = extreme(&levels, width, height, reach, stroke.inside);
        // The ring between the two shapes.
        for (i, v) in levels.iter_mut().enumerate() {
            *v = if stroke.inside {
                (*v - moved[i]).max(0.0)
            } else {
                (moved[i] - *v).max(0.0)
            };
        }
        fill(canvas, stroke.color(), stroke.opacity, &levels_mask(&levels, width, height));
    };
    if let Some(stroke) = &stroke {
        if !stroke.inside {
            draw_stroke(&mut canvas, stroke);
        }
    }
    // The pixels themselves, source-over.
    {
        let mut over = canvas.clone();
        crate::raster::draw_over(
            &mut over,
            &shown,
            placed,
            crate::transform::LayerSampling::Nearest,
        );
        canvas = over;
    }
    if let Some(overlay) = effects.color_overlay {
        if overlay.is_enabled() && overlay.opacity > 0.0 {
            let shape = shape_coverage(&shown, placed, width, height, 0.0);
            fill(&mut canvas, overlay.color(), overlay.opacity, &shape);
        }
    }
    if let Some(inner_glow) = effects.inner_glow {
        if inner_glow.is_enabled() && inner_glow.opacity > 0.0 {
            let shape = shape_coverage(&shown, placed, width, height, 0.0);
            let blurred = shape_coverage(&shown, placed, width, height, inner_glow.size);
            let mut inside = mask_levels(&shape);
            let outside = mask_levels(&blurred);
            for (i, v) in inside.iter_mut().enumerate() {
                *v = (*v * (1.0 - outside[i])).clamp(0.0, 1.0);
            }
            fill(&mut canvas, inner_glow.color(), inner_glow.opacity, &levels_mask(&inside, width, height));
        }
    }
    if let Some(inner) = effects.inner_shadow {
        if inner.is_enabled() && inner.opacity > 0.0 {
            let shape = shape_coverage(&shown, placed, width, height, 0.0);
            let offset = inner.offset();
            let moved = shape_coverage(
                &shown,
                placed.offset_by(offset.width, offset.height),
                width,
                height,
                inner.blur,
            );
            let mut inside = mask_levels(&shape);
            let outside = mask_levels(&moved);
            for (i, v) in inside.iter_mut().enumerate() {
                *v = (*v * (1.0 - outside[i])).clamp(0.0, 1.0);
            }
            fill(&mut canvas, inner.color(), inner.opacity, &levels_mask(&inside, width, height));
        }
    }
    if let Some(stroke) = &stroke {
        if stroke.inside {
            draw_stroke(&mut canvas, stroke);
        }
    }
    (canvas, inset)
}

/// The room the effects need around the layer.
pub fn effect_margin(effects: &LayerEffects) -> CGFloat {
    let effects = effects.visible();
    let mut margin: CGFloat = 0.0;
    if let Some(stroke) = effects.stroke {
        if !stroke.inside {
            margin = margin.max(stroke.size);
        }
    }
    if let Some(shadow) = effects.shadow {
        margin = margin.max(shadow.distance + shadow.blur * 3.0);
    }
    if let Some(glow) = effects.outer_glow {
        margin = margin.max(glow.size * 3.0);
    }
    (margin.ceil() + 2.0) as CGFloat
}

/// `LayerEffectsRenderer.placed`: the layer's transform grown by the margin.
pub fn placed_transform(transform: &LayerTransform, image: &Bitmap, inset: CGFloat) -> LayerTransform {
    let mut grown = *transform;
    let width = image.width as f64;
    let height = image.height as f64;
    if width <= inset * 2.0 || height <= inset * 2.0 {
        return *transform;
    }
    grown.size = CGSize::new(
        transform.size.width * width / (width - inset * 2.0),
        transform.size.height * height / (height - inset * 2.0),
    );
    grown.origin = CGPoint::new(
        transform.center().x - grown.size.width / 2.0,
        transform.center().y - grown.size.height / 2.0,
    );
    grown
}

/// A selected effect (`LayerEffectSelection`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerEffectSelection {
    pub layer_id: uuid::Uuid,
    pub kind: LayerEffectKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_defaults_mean_visible() {
        let s: StrokeEffect = serde_json::from_str("{}").unwrap();
        assert!(s.is_enabled());
        assert_eq!(s.size, 4.0);
        assert_eq!(s.opacity, 1.0);
    }

    #[test]
    fn shadow_offset_angle() {
        // 90° (light straight above) drops the shadow straight down.
        let s = ShadowEffect::default();
        let offset = s.offset();
        assert!(offset.width.abs() < 1e-9);
        assert!((offset.height - 20.0).abs() < 1e-9);
        // 0° (light from the right) drops the shadow to the left.
        let s = ShadowEffect {
            angle: 0.0,
            ..Default::default()
        };
        let offset = s.offset();
        assert!((offset.width + 20.0).abs() < 1e-9);
    }

    #[test]
    fn effects_kinds_and_colors() {
        let mut e = LayerEffects::default();
        assert!(e.is_empty());
        e.shadow = Some(ShadowEffect::default());
        assert_eq!(e.kinds(), vec![LayerEffectKind::Shadow]);
        e.set_color(PaletteColor::new(1.0, 0.0, 0.0), LayerEffectKind::Shadow);
        assert_eq!(e.color(LayerEffectKind::Shadow), Some(PaletteColor::new(1.0, 0.0, 0.0)));
        e.set_enabled(false, LayerEffectKind::Shadow);
        assert!(!e.is_enabled(LayerEffectKind::Shadow));
        assert!(e.visible().shadow.is_none());
        assert!(e.contains(LayerEffectKind::Shadow)); // still listed
        e.remove(LayerEffectKind::Shadow);
        assert!(e.is_empty());
    }

    #[test]
    fn extreme_sliding_window_max() {
        let data: Vec<f32> = (0..25).map(|v| v as f32).collect();
        let maxed = extreme(&data, 5, 5, 1, false);
        assert_eq!(maxed[12], 18.0); // max of the 3×3 neighborhood of 12
        let mined = extreme(&data, 5, 5, 1, true);
        assert_eq!(mined[12], 6.0);
        // Outside the reachable band, `smallest` yields 0.
        assert_eq!(mined[0], 0.0);
    }

    #[test]
    fn margin_grows_with_reach() {
        let mut e = LayerEffects::default();
        assert_eq!(effect_margin(&e), 2.0);
        e.shadow = Some(ShadowEffect::default());
        // 20 + 20*3 + 2 = 82.
        assert_eq!(effect_margin(&e), 82.0);
    }

    #[test]
    fn render_drop_shadow_composited() {
        // A small opaque square layer.
        let mut img = Bitmap::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                img.set_pixel(x, y, [255, 0, 0, 255]);
            }
        }
        let mut effects = LayerEffects::default();
        effects.shadow = Some(ShadowEffect {
            distance: 2.0,
            blur: 0.0,
            opacity: 1.0,
            ..Default::default()
        });
        let (result, inset) = render_effects(&img, None, &effects);
        assert!(inset >= 4.0);
        // The output is inset by the margin: the square sits offset and the
        // shadow (distance 2 down) is black below it.
        let square = [255u8, 0, 0, 255];
        let black = [0u8, 0, 0, 255];
        assert!(
            (0..result.height).any(|y| (0..result.width).any(|x| result.pixel(x, y) == square)),
            "the square stays"
        );
        assert!(
            (0..result.height).any(|y| (0..result.width).any(|x| result.pixel(x, y) == black)),
            "a black shadow pixel is rendered"
        );
        assert!(result.height >= 4 + inset as usize, "the inset reserves room");
    }

    #[test]
    fn stroke_ring_outside_and_inside() {
        let mut img = Bitmap::new(8, 8);
        for y in 2..6 {
            for x in 2..6 {
                img.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        let mut effects = LayerEffects::default();
        effects.stroke = Some(StrokeEffect {
            size: 2.0,
            inside: false,
            opacity: 1.0,
            ..Default::default()
        });
        let (outside, _) = render_effects(&img, None, &effects);
        assert!(outside.data.iter().any(|v| *v > 0));
        effects.stroke.as_mut().unwrap().inside = true;
        let (inside, _) = render_effects(&img, None, &effects);
        assert!(inside.data.iter().any(|v| *v > 0));
    }

    #[test]
    fn placed_transform_grows_symmetrically() {
        let t = LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(100.0, 100.0));
        let img = Bitmap::new(100, 100);
        let grown = placed_transform(&t, &img, 10.0);
        assert!((grown.size.width - 125.0).abs() < 1e-9);
        assert_eq!(grown.center(), t.center());
    }
}
