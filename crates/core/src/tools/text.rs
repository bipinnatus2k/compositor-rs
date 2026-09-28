//! Port of `Document/TypeTool.swift`'s typesetting half: text attributes, box
//! measurement, and rasterizing a text layer's pixels with a real font.
//! AppKit's text stack is replaced by fontdue plus a system-font lookup.

use crate::document::{LayerTextStyle, TextAlignment};
use crate::geometry::CGSize;
use crate::raster::Bitmap;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// `LayerTextStyle.padding`.
const PADDING: f64 = 12.0;

/// Candidate font directories, per platform.
const FONT_DIRECTORIES: &[&str] = &[
    "C:\\Windows\\Fonts",
    "/System/Library/Fonts",
    "/System/Library/Fonts/Supplemental",
    "/Library/Fonts",
    "/usr/share/fonts",
    "/usr/local/share/fonts",
];

/// Well-known stand-ins for fonts the host may not ship (Helvetica first).
const FONT_SUBSTITUTES: &[(&str, &[&str])] = &[
    ("helvetica", &["arial.ttf", "Helvetica.ttc", "segoeui.ttf", "DejaVuSans.ttf", "LiberationSans-Regular.ttf"]),
    ("arial", &["arial.ttf", "Helvetica.ttc", "DejaVuSans.ttf"]),
    ("times new roman", &["times.ttf", "Times New Roman.ttf", "DejaVuSerif.ttf"]),
    ("courier new", &["cour.ttf", "DejaVuSansMono.ttf"]),
];

fn font_cache() -> &'static Mutex<HashMap<String, Option<fontdue::Font>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<fontdue::Font>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Loads (and caches) a font by name, with platform stand-ins; falls back to
/// any installed font so text never hard-fails on an unknown name.
pub fn load_font(name: &str) -> Option<fontdue::Font> {
    let key = name.to_lowercase();
    // The cache holds only plain values, so a poisoned lock (a parse panic)
    // is recovered instead of masquerading as "no font".
    let mut cache = font_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }
    let loaded = find_font_file(&key).and_then(parse_font_file);
    cache.insert(key, loaded.clone());
    loaded
}

fn find_font_file(name: &str) -> Option<std::path::PathBuf> {
    let candidates: &[&str] = FONT_SUBSTITUTES
        .iter()
        .find(|(font, _)| *font == name)
        .map(|(_, list)| *list)
        .unwrap_or(&[]);
    for directory in FONT_DIRECTORIES {
        let dir = std::path::Path::new(directory);
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<std::path::PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                matches!(
                    path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(),
                    Some("ttf") | Some("otf") | Some("ttc")
                )
            })
            .collect();
        files.sort();
        // A file whose name contains the requested font name wins.
        if let Some(matched) = files
            .iter()
            .find(|path| {
                path.file_name()
                    .map(|f| f.to_string_lossy().to_lowercase().contains(name))
                    .unwrap_or(false)
            })
            .cloned()
        {
            return Some(matched);
        }
        for candidate in candidates {
            if let Some(matched) = files
                .iter()
                .find(|path| {
                    path.file_name()
                        .map(|f| f.to_string_lossy().to_lowercase() == *candidate)
                        .unwrap_or(false)
                })
                .cloned()
            {
                return Some(matched);
            }
        }
        // Any installed font at all: text keeps working.
        if let Some(any) = files.first().cloned() {
            return Some(any);
        }
    }
    None
}

fn parse_font_file(path: std::path::PathBuf) -> Option<fontdue::Font> {
    let data = std::fs::read(path).ok()?;
    // fontdue panics on bytes it cannot parse; check the sfnt magic first.
    if data.len() < 4 {
        return None;
    }
    let magic = &data[0..4];
    let supported = magic == [0x00, 0x01, 0x00, 0x00]
        || magic == b"OTTO"
        || magic == b"true"
        || magic == b"ttcf";
    if !supported {
        return None;
    }
    let settings = fontdue::FontSettings {
        scale: 40.0,
        collection_index: 0,
        load_substitutions: true,
    };
    // A pathological font can still panic the parser; keep it contained.
    std::panic::catch_unwind(move || fontdue::Font::from_bytes(data, settings))
        .ok()?
        .ok()
}

/// One laid-out line: its text, pixel width, and where it starts.
struct LayoutLine {
    text: String,
    width: f64,
}

/// Word-wraps the content into lines that fit `container_width` (point text
/// with no box never wraps), measuring with the style's font, size, tracking.
fn layout_lines(style: &LayerTextStyle, container_width: f64) -> Vec<LayoutLine> {
    let font = load_font(&style.font_name);
    let size = style.font_size as f32;
    let advance_of = |text: &str| -> f64 {
        match &font {
            Some(font) => {
                let mut width = 0.0;
                for ch in text.chars() {
                    width += font.metrics(ch, size).advance_width as f64 + style.tracking;
                }
                width
            }
            None => text.chars().count() as f64 * style.font_size * 0.6,
        }
    };
    let wraps = style.box_size.is_some();
    let mut lines = Vec::new();
    for paragraph in style.content.split('\n') {
        if !wraps {
            lines.push(LayoutLine { text: paragraph.to_string(), width: advance_of(paragraph) });
            continue;
        }
        // By-word wrapping: a line breaks at the last space that fits.
        let mut current = String::new();
        let mut current_width = 0.0;
        for word in paragraph.split_inclusive(' ') {
            let word_width = advance_of(word.trim_end_matches(' ')) + advance_of(" ") * word.matches(' ').count() as f64;
            if !current.is_empty() && current_width + word_width > container_width {
                lines.push(LayoutLine { width: current_width, text: current.trim_end().to_string() });
                current = word.trim_start().to_string();
                current_width = advance_of(current.trim_start().to_string().as_str());
            } else {
                current.push_str(word);
                current_width += word_width;
            }
        }
        lines.push(LayoutLine { width: current_width, text: current.trim_end().to_string() });
    }
    if lines.is_empty() {
        lines.push(LayoutLine { text: String::new(), width: 0.0 });
    }
    lines
}

/// `textBoxSize`: a fixed box as-is; point text measures its lines plus the
/// padding, with a caret's worth of width so an empty line still has room.
pub fn text_box_size(style: &LayerTextStyle) -> CGSize {
    if let Some(box_size) = style.box_size {
        return box_size;
    }
    let lines = layout_lines(style, 100_000.0);
    let measured_width = lines.iter().map(|l| l.width).fold(0.0f64, f64::max);
    let measured_height = lines.len() as f64 * style.line_height();
    let line = style.line_height().ceil();
    CGSize::new(
        (measured_width + PADDING * 2.0 + style.font_size * 0.1).ceil().max(16.0),
        (measured_height.max(line) + PADDING * 2.0).ceil().max(16.0),
    )
}

/// `LayerTextStyle::layerName` — the first words on one line.
pub fn layer_name(content: &str) -> String {
    let flattened: Vec<&str> = content
        .split(|c: char| c.is_whitespace() || c == '\n' || c == '\r')
        .filter(|part| !part.is_empty())
        .collect();
    let joined = flattened.join(" ");
    if joined.is_empty() {
        "Text".to_string()
    } else {
        joined.chars().take(40).collect()
    }
}

/// `textImage`: the text rasterized into a transparent bitmap, laid out with
/// the style's alignment, tracking, and leading.
pub fn text_image(style: &LayerTextStyle) -> Result<Bitmap, String> {
    if !style.is_valid() {
        return Err("invalid text".to_string());
    }
    let size = text_box_size(style);
    let width = size.width.ceil();
    let height = size.height.ceil();
    if !width.is_finite()
        || !height.is_finite()
        || width < 1.0
        || height < 1.0
        || width > 30_000.0
        || height > 30_000.0
        || width * height > 100_000_000.0
    {
        return Err("the text layer is too large".to_string());
    }
    let width = width as usize;
    let height = height as usize;
    let Some(font) = load_font(&style.font_name) else {
        return Err("no font could be loaded".to_string());
    };
    let size = style.font_size as f32;
    // The font's ascent above the baseline, at this pixel size.
    let ascent = font
        .horizontal_line_metrics(size)
        .map(|m| m.ascent as f64)
        .unwrap_or(style.font_size * 0.8);
    let container_width = (width as f64 - PADDING * 2.0).max(1.0);
    let lines = layout_lines(style, container_width);
    let mut canvas = Bitmap::new(width, height);
    let color = [
        ((style.red * 255.0).round() as f32 / 255.0),
        ((style.green * 255.0).round() as f32 / 255.0),
        ((style.blue * 255.0).round() as f32 / 255.0),
    ];
    for (index, line) in lines.iter().enumerate() {
        let baseline = PADDING + index as f64 * style.line_height() + ascent;
        // Alignment positions the line within the container.
        let free = container_width - line.width;
        let mut pen_x = match style.alignment {
            TextAlignment::Left => PADDING,
            TextAlignment::Center => PADDING + (free / 2.0).max(0.0),
            TextAlignment::Right => PADDING + free.max(0.0),
        };
        for ch in line.text.chars() {
            let (metrics, coverage) = font.rasterize(ch, size);
            // `ymin` is the bitmap's bottom edge above the baseline.
            let left = pen_x + metrics.xmin as f64;
            let top = baseline - metrics.ymin as f64 - metrics.height as f64;
            // Whitespace has no bitmap at all; only the pen advances.
            for (row, line_bytes) in (metrics.width > 0)
                .then(|| coverage.chunks_exact(metrics.width).enumerate())
                .into_iter()
                .flatten()
            {
                for (column, &alpha) in line_bytes.iter().enumerate() {
                    if alpha == 0 {
                        continue;
                    }
                    let px = left as isize + column as isize;
                    let py = top as isize + row as isize;
                    if px < 0 || py < 0 || px >= width as isize || py >= height as isize {
                        continue;
                    }
                    let o = (py as usize * width + px as usize) * 4;
                    blend_glyph(&mut canvas.data[o..o + 4], color, alpha);
                }
            }
            pen_x += font.metrics(ch, size).advance_width as f64 + style.tracking;
        }
    }
    Ok(canvas)
}

/// Blends one glyph-coverage byte over the canvas: the text color is
/// straight, the canvas stores premultiplied pixels.
fn blend_glyph(pixel: &mut [u8], color: [f32; 3], alpha: u8) {
    let a = alpha as f32 / 255.0;
    let src = [color[0] * a, color[1] * a, color[2] * a, a];
    let dst_a = pixel[3] as f32 / 255.0;
    let out_a = src[3] + dst_a * (1.0 - src[3]);
    if out_a <= 0.0 {
        return;
    }
    for c in 0..3 {
        let out = (src[c] * src[3] + (pixel[c] as f32 / 255.0) * dst_a * (1.0 - src[3])) / out_a;
        pixel[c] = (out * 255.0).round() as u8;
    }
    pixel[3] = (out_a * 255.0).round() as u8;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::LayerTextStyle;

    fn style(content: &str) -> LayerTextStyle {
        LayerTextStyle {
            content: content.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn point_text_box_measures_the_content() {
        let small = style("Hi");
        let big = style("Hi");
        let mut big = big;
        big.font_size = 200.0;
        let small_box = text_box_size(&small);
        let big_box = text_box_size(&big);
        assert!(small_box.width > 16.0 + 24.0, "{small_box:?}");
        assert!(big_box.width > small_box.width, "{small_box:?} vs {big_box:?}");
        assert!(big_box.height > small_box.height, "taller type measures taller");
        // Padding surrounds the text on both sides.
        assert!(small_box.height >= small.line_height() + PADDING * 2.0 - 2.0);
    }

    #[test]
    fn a_fixed_box_is_used_verbatim() {
        let mut s = style("Hi");
        s.box_size = Some(CGSize::new(200.0, 120.0));
        assert_eq!(text_box_size(&s), CGSize::new(200.0, 120.0));
    }

    #[test]
    fn word_wrapping_breaks_long_paragraphs_at_the_box() {
        let mut s = style("one two three four five six seven eight");
        s.box_size = Some(CGSize::new(220.0, 400.0));
        s.font_size = 40.0;
        let image = text_image(&s).unwrap();
        assert_eq!(image.width, 220);
        assert_eq!(image.height, 400);
        // Text pixels exist, and they live inside the box's padding.
        let mut ink_rows = 0;
        for y in 0..image.height {
            let row_ink = (0..image.width).any(|x| image.pixel(x, y)[3] > 0);
            ink_rows += u32::from(row_ink);
        }
        assert!(ink_rows > 10, "glyphs are rasterized: {ink_rows} ink rows");
    }

    #[test]
    fn glyphs_carry_the_style_color_and_align_right() {
        let mut s = style("A");
        s.red = 1.0;
        s.green = 0.0;
        s.blue = 0.0;
        s.alignment = TextAlignment::Right;
        let image = text_image(&s).unwrap();
        // Some pixel is saturated red.
        let reddest = (0..image.height * image.width)
            .map(|i| image.data[i * 4])
            .max()
            .unwrap();
        assert!(reddest > 200, "glyph pixels carry the color: {reddest}");
        // Nothing is drawn to the left of the padding on a right-aligned line.
        for y in 0..image.height {
            assert_eq!(image.pixel(1, y)[3], 0, "left padding stays clean");
        }
    }

    #[test]
    fn layer_names_flatten_the_first_words() {
        assert_eq!(layer_name("Hello\n  world   again"), "Hello world again");
        assert_eq!(layer_name("   "), "Text");
        let long: String = "word ".repeat(20);
        assert_eq!(layer_name(&long).chars().count(), 40);
    }

    #[test]
    fn tracking_spreads_the_line() {
        let mut tight = style("II II");
        tight.font_size = 40.0;
        let mut wide = tight.clone();
        wide.tracking = 40.0;
        assert!(
            text_box_size(&wide).width > text_box_size(&tight).width,
            "tracking widens the measured box"
        );
    }
}
