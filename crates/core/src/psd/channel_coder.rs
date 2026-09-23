//! Port of `Compositor/IO/PSD/PSDChannelCoder.swift`: unpacks Photoshop layer
//! channels from Adobe's 2019 Photoshop File Formats Specification (Image Data,
//! compression 0 raw and 1 PackBits). The PackBits *encoder* comes from
//! `CompositorTests/PSDFixture.swift`, the byte-level writer the round-trip
//! tests use.

use crate::raster::Bitmap;

use super::types::PsdError;

pub struct PsdChannelCoder;

impl PsdChannelCoder {
    /// Decodes one channel plane to `width * height` gray bytes.
    pub fn decode(compression: i64, width: usize, height: usize, data: &[u8]) -> Result<Vec<u8>, PsdError> {
        if width == 0 || height == 0 {
            return Ok(Vec::new());
        }
        let expected = width * height;
        match compression {
            0 => {
                if data.len() < expected {
                    return Err(PsdError::Truncated);
                }
                Ok(data[..expected].to_vec())
            }
            1 => Self::unpack_rle(width, height, data),
            _ => Err(PsdError::UnsupportedCompression),
        }
    }

    /// `rgbaImage`: interleaves the planes into a premultiplied RGBA bitmap
    /// (`pixels[i * 4] = (red[i] * a + 127) / 255` and so on).
    pub fn rgba_bitmap(
        width: usize,
        height: usize,
        red: &[u8],
        green: &[u8],
        blue: &[u8],
        alpha: &[u8],
    ) -> Result<Bitmap, PsdError> {
        let count = width * height;
        let mut pixels = vec![0u8; count * 4];
        for i in 0..count {
            let a = alpha[i] as u16;
            pixels[i * 4] = ((red[i] as u16 * a + 127) / 255) as u8;
            pixels[i * 4 + 1] = ((green[i] as u16 * a + 127) / 255) as u8;
            pixels[i * 4 + 2] = ((blue[i] as u16 * a + 127) / 255) as u8;
            pixels[i * 4 + 3] = a as u8;
        }
        Ok(Bitmap::from_raw(width, height, pixels))
    }

    /// The Swift `image(width:height:rgba:)` — wrapping pixels as a bitmap.
    pub fn image(width: usize, height: usize, rgba: Vec<u8>) -> Result<Bitmap, PsdError> {
        if rgba.len() != width * height * 4 {
            return Err(PsdError::Truncated);
        }
        Ok(Bitmap::from_raw(width, height, rgba))
    }

    /// The Swift `maskImage(width:height:gray:)` — an 8-bit gray plane.
    pub fn mask_image(width: usize, height: usize, gray: Vec<u8>) -> Result<crate::raster::GrayMask, PsdError> {
        if gray.len() != width * height {
            return Err(PsdError::Truncated);
        }
        Ok(crate::raster::GrayMask::from_raw(width, height, gray))
    }

    /// `unpackRLE`: per-row PackBits with a 2-byte big-endian byte count per row.
    fn unpack_rle(width: usize, height: usize, data: &[u8]) -> Result<Vec<u8>, PsdError> {
        let mut offset = 0usize;
        let mut next = || -> Result<u8, PsdError> {
            if offset >= data.len() {
                return Err(PsdError::Truncated);
            }
            let value = data[offset];
            offset += 1;
            Ok(value)
        };
        let mut counts = vec![0usize; height];
        for row in 0..height {
            let hi = next()? as usize;
            let lo = next()? as usize;
            counts[row] = hi << 8 | lo;
        }
        let mut plane = vec![0u8; width * height];
        for row in 0..height {
            let end = offset + counts[row];
            if end > data.len() {
                return Err(PsdError::Truncated);
            }
            let mut written = 0usize;
            while written < width {
                if offset >= end {
                    return Err(PsdError::Truncated);
                }
                let n = data[offset] as i8;
                offset += 1;
                if n >= 0 {
                    // Literal run of n + 1 bytes.
                    let count = n as usize + 1;
                    if written + count > width || offset + count > end {
                        return Err(PsdError::Truncated);
                    }
                    let at = row * width + written;
                    plane[at..at + count].copy_from_slice(&data[offset..offset + count]);
                    offset += count;
                    written += count;
                } else if n != -128 {
                    // Repeat run; -128 is a no-op.
                    let count = (1 - n as i32) as usize;
                    if written + count > width || offset >= end {
                        return Err(PsdError::Truncated);
                    }
                    let value = data[offset];
                    offset += 1;
                    let at = row * width + written;
                    plane[at..at + count].fill(value);
                    written += count;
                }
            }
            offset = end;
        }
        Ok(plane)
    }

    /// `PSDFixture.packBits`: the PackBits encoder, byte-exact.
    pub fn pack_bits(row: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        let mut i = 0usize;
        while i < row.len() {
            if i + 1 < row.len() && row[i] == row[i + 1] {
                let mut run = 2usize;
                while i + run < row.len() && row[i + run] == row[i] && run < 128 {
                    run += 1;
                }
                output.push((1i32 - run as i32) as u8);
                output.push(row[i]);
                i += run;
            } else {
                let start = i;
                i += 1;
                while i < row.len() && i - start < 128 {
                    if i + 1 < row.len() && row[i] == row[i + 1] {
                        break;
                    }
                    i += 1;
                }
                output.push((i - start - 1) as u8);
                output.extend_from_slice(&row[start..i]);
            }
        }
        output
    }

    /// `PSDFixture.encode`: a channel payload — 2-byte compression marker
    /// (1 RLE, 0 raw when there is nothing to encode) plus per-row counts.
    pub fn encode_plane(plane: &[u8], width: usize, height: usize) -> (u16, Vec<u8>) {
        if width == 0 || height == 0 || plane.len() < width * height {
            return (0, Vec::new());
        }
        let mut counts = Vec::with_capacity(height * 2);
        let mut packed = Vec::new();
        for row in 0..height {
            let encoded = Self::pack_bits(&plane[row * width..(row + 1) * width]);
            counts.extend_from_slice(&[(encoded.len() >> 8) as u8, encoded.len() as u8]);
            packed.extend_from_slice(&encoded);
        }
        counts.extend_from_slice(&packed);
        (1, counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_round_trip_randomish_rows() {
        // A row mixing literal and repeat runs, byte-exact through both ends.
        let mut plane = Vec::new();
        let mut value = 7u8;
        for _ in 0..64 * 9 {
            value = value.wrapping_mul(31).wrapping_add(17);
            plane.push(value);
        }
        for i in 0..64 {
            plane[i] = 5;
        }
        for i in 64..100 {
            plane[i] = 9;
        }
        let (compression, encoded) = PsdChannelCoder::encode_plane(&plane, 64, 9);
        assert_eq!(compression, 1);
        let decoded = PsdChannelCoder::decode(1, 64, 9, &encoded).unwrap();
        assert_eq!(decoded, plane);
    }

    #[test]
    fn pack_bits_run_128_noop_128() {
        // 128 equal bytes → one repeat run; -128 never emitted.
        let row = vec![200u8; 128];
        let encoded = PsdChannelCoder::pack_bits(&row);
        assert_eq!(encoded, vec![(1i32 - 128) as u8, 200]);
        let mut data = vec![0, 2];
        data.extend_from_slice(&encoded);
        let decoded = PsdChannelCoder::decode(1, 128, 1, &data).unwrap();
        assert_eq!(decoded, row);
    }

    #[test]
    fn decode_rejects_compression_two() {
        assert_eq!(
            PsdChannelCoder::decode(2, 1, 1, &[0]),
            Err(PsdError::UnsupportedCompression)
        );
    }
}
