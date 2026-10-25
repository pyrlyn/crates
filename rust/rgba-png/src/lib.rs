//! RGBA8 frames and PNG files: encoding, decoding, pixel comparison.
//!
//! Frames are tightly packed, row-major, 4 bytes per pixel.

use std::io::Cursor;

/// Encodes a frame as an sRGB PNG.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, png::EncodingError> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    // The samples are already sRGB. The chunk stops a viewer from converting them.
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    writer.finish()?;
    Ok(out)
}

/// Decodes a PNG into `(width, height, samples)` as stored: an RGBA8 file
/// gives an RGBA8 frame, other formats are not converted.
pub fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), png::DecodingError> {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf)?;
    buf.truncate(info.line_size * info.height as usize);
    Ok((info.width, info.height, buf))
}

/// A PNG whose header has been read but whose pixels have not, so the caller
/// can refuse its size before anything is decoded.
pub struct PngHeader<'a> {
    reader: png::Reader<Cursor<&'a [u8]>>,
}

/// Reads the header of a PNG of any color type and bit depth. `max_bytes`
/// caps the decoder's own buffers, not the frame: check `size()` for that.
pub fn read_png_header(
    bytes: &[u8],
    max_bytes: usize,
) -> Result<PngHeader<'_>, png::DecodingError> {
    let limits = png::Limits { bytes: max_bytes };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), limits);
    // Palette, low bit depth and 16-bit samples all become 8-bit channels.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    Ok(PngHeader {
        reader: decoder.read_info()?,
    })
}

impl PngHeader<'_> {
    /// `(width, height)` from the header.
    pub fn size(&self) -> (u32, u32) {
        let info = self.reader.info();
        (info.width, info.height)
    }

    /// Decodes the first frame into an RGBA8 frame.
    pub fn decode_rgba(mut self) -> Result<Vec<u8>, png::DecodingError> {
        // Too small a buffer (unknown size) makes `next_frame` fail cleanly.
        let mut buf = vec![0; self.reader.output_buffer_size().unwrap_or(0)];
        let out = self.reader.next_frame(&mut buf)?;
        buf.truncate(out.line_size * out.height as usize);
        Ok(widen(out.color_type.samples(), &buf))
    }
}

/// Spreads 1 to 4 channels per pixel (grey, grey+alpha, RGB, RGBA) to RGBA.
fn widen(channels: usize, buf: &[u8]) -> Vec<u8> {
    buf.chunks_exact(channels.max(1))
        .flat_map(|px| match *px {
            [g] => [g, g, g, u8::MAX],
            [g, a] => [g, g, g, a],
            [r, g, b] => [r, g, b, u8::MAX],
            [r, g, b, a, ..] => [r, g, b, a],
            [] => [0; 4],
        })
        .collect()
}

/// The largest difference of any channel between two pixels.
pub fn channel_delta(a: [u8; 4], b: [u8; 4]) -> u8 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

/// Pixels whose largest channel difference is above `threshold`. Zero
/// counts every pixel that is not identical.
pub fn differing_pixels(left: &[u8], right: &[u8], threshold: u8) -> usize {
    let (left, _) = left.as_chunks::<4>();
    let (right, _) = right.as_chunks::<4>();
    left.iter()
        .zip(right)
        .filter(|(a, b)| channel_delta(**a, **b) > threshold)
        .count()
}

/// A frame that shows `left` faded, with pixels that differ past `threshold`
/// in opaque red.
pub fn diff_image(left: &[u8], right: &[u8], threshold: u8) -> Vec<u8> {
    let (left, _) = left.as_chunks::<4>();
    let (right, _) = right.as_chunks::<4>();
    left.iter()
        .zip(right)
        .flat_map(|(a, b)| {
            if channel_delta(*a, *b) > threshold {
                [255, 0, 0, 255]
            } else {
                let grey = ((u16::from(a[0]) + u16::from(a[1]) + u16::from(a[2])) / 3) as u8;
                let faded = 255 - (255 - grey) / 4;
                [faded, faded, faded, 255]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let rgba = [1, 2, 3, 4, 250, 251, 252, 253];
        let png = encode_png(2, 1, &rgba)?;
        assert_eq!(decode_png(&png)?, (2, 1, rgba.to_vec()));
        Ok(())
    }

    #[test]
    fn threshold_ignores_small_differences() {
        let left = [10, 10, 10, 255, 10, 10, 10, 255];
        let right = [12, 10, 10, 255, 10, 40, 10, 255];
        assert_eq!(differing_pixels(&left, &right, 0), 2);
        assert_eq!(differing_pixels(&left, &right, 2), 1);
        assert_eq!(differing_pixels(&left, &right, 30), 0);
    }

    #[test]
    fn diff_marks_only_differing_pixels() {
        let left = [0, 0, 0, 255, 0, 0, 0, 255];
        let right = [0, 0, 0, 255, 200, 0, 0, 255];
        let diff = diff_image(&left, &right, 8);
        assert_ne!(&diff[..4], &[255, 0, 0, 255]);
        assert_eq!(&diff[4..], &[255, 0, 0, 255]);
    }
}
