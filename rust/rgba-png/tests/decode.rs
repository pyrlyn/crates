use png::{BitDepth, ColorType};
use rgba_png::{decode_png, encode_png, read_png_header};

type Result = std::result::Result<(), Box<dyn std::error::Error>>;

fn png(
    width: u32,
    color: ColorType,
    depth: BitDepth,
    data: &[u8],
    palette: Option<&[u8]>,
) -> std::result::Result<Vec<u8>, png::EncodingError> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, 1);
    encoder.set_color(color);
    encoder.set_depth(depth);
    if let Some(palette) = palette {
        encoder.set_palette(palette.to_vec());
    }
    let mut writer = encoder.write_header()?;
    writer.write_image_data(data)?;
    writer.finish()?;
    Ok(out)
}

fn rgba(bytes: &[u8]) -> std::result::Result<Vec<u8>, png::DecodingError> {
    read_png_header(bytes, usize::MAX)?.decode_rgba()
}

#[test]
fn every_color_type_widens_to_rgba() -> Result {
    let eight = BitDepth::Eight;
    let cases: [(ColorType, &[u8], [u8; 8]); 4] = [
        (
            ColorType::Grayscale,
            &[10, 20],
            [10, 10, 10, 255, 20, 20, 20, 255],
        ),
        (
            ColorType::GrayscaleAlpha,
            &[10, 1, 20, 2],
            [10, 10, 10, 1, 20, 20, 20, 2],
        ),
        (
            ColorType::Rgb,
            &[1, 2, 3, 4, 5, 6],
            [1, 2, 3, 255, 4, 5, 6, 255],
        ),
        (
            ColorType::Rgba,
            &[1, 2, 3, 4, 5, 6, 7, 8],
            [1, 2, 3, 4, 5, 6, 7, 8],
        ),
    ];
    for (color, data, want) in cases {
        assert_eq!(rgba(&png(2, color, eight, data, None)?)?, want, "{color:?}");
    }
    Ok(())
}

#[test]
fn palette_low_depth_and_sixteen_bit_become_eight_bit() -> Result {
    let palette = png(
        2,
        ColorType::Indexed,
        BitDepth::Eight,
        &[1, 0],
        Some(&[9, 8, 7, 1, 2, 3]),
    )?;
    assert_eq!(rgba(&palette)?, [1, 2, 3, 255, 9, 8, 7, 255]);
    // 0b10 then 0b01 in the top bits; 2-bit grey scales 1 to 85 and 2 to 170.
    let low = png(2, ColorType::Grayscale, BitDepth::Two, &[0b1001_0000], None)?;
    assert_eq!(rgba(&low)?, [170, 170, 170, 255, 85, 85, 85, 255]);
    let wide = png(
        1,
        ColorType::Rgb,
        BitDepth::Sixteen,
        &[1, 0, 2, 0, 3, 0],
        None,
    )?;
    assert_eq!(rgba(&wide)?, [1, 2, 3, 255]);
    Ok(())
}

#[test]
fn header_gives_the_size_before_decoding() -> Result {
    let bytes = encode_png(3, 1, &[0; 12])?;
    let header = read_png_header(&bytes, usize::MAX)?;
    assert_eq!(header.size(), (3, 1));
    assert_eq!(header.decode_rgba()?, vec![0; 12]);
    Ok(())
}

#[test]
fn allocation_cap_refuses_a_wide_row() -> Result {
    let bytes = encode_png(4096, 1, &[0; 4096 * 4])?;
    // The cap covers the decoder's row buffers, so a row wider than it fails.
    let refused = read_png_header(&bytes, 1024).and_then(|h| h.decode_rgba());
    assert!(
        matches!(refused, Err(png::DecodingError::LimitsExceeded)),
        "{refused:?}"
    );
    Ok(())
}

#[test]
fn garbage_is_an_error() {
    assert!(read_png_header(b"\x89PNG\r\n\x1a\nnot a png", usize::MAX).is_err());
    assert!(decode_png(b"not a png").is_err());
}
