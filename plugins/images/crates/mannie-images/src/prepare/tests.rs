use std::io::Cursor;

use image::{
    DynamicImage, GenericImageView, ImageBuffer, ImageFormat as Format, Rgb, RgbImage, Rgba,
    RgbaImage,
};
use mannie_types::images::{ImageFormat, MAX_IMAGE_BYTES};

use super::prepare;
use crate::commands::{ImagesError, MAX_DECODED_MEGAPIXELS, MAX_DECODED_SIDE, MAX_FILE_BYTES};

/// An image with a gradient, so that it compresses as photos do.
fn gradient(width: u32, height: u32) -> RgbImage {
    ImageBuffer::from_fn(width, height, |x, y| {
        let red = u8::try_from(x % 256).unwrap();
        let green = u8::try_from(y % 256).unwrap();
        Rgb([red, green, 128])
    })
}

fn encoded(image: &DynamicImage, format: Format) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, format).unwrap();
    out.into_inner()
}

fn decoded(bytes: &[u8]) -> DynamicImage {
    image::load_from_memory(bytes).unwrap()
}

/// Bytes that look like noise, from a fixed seed.
fn noise(length: usize) -> Vec<u8> {
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[0]
        })
        .collect()
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

/// A PNG whose header says `width` by `height` and whose pixels are missing,
/// as a hostile file would be.
fn png_header(width: u32, height: u32) -> Vec<u8> {
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [
        (*b"IHDR", header),
        (*b"IDAT", vec![0x78, 0x01]),
        (*b"IEND", Vec::new()),
    ] {
        png.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
        let mut chunk = kind.to_vec();
        chunk.extend_from_slice(&data);
        png.extend_from_slice(&chunk);
        png.extend_from_slice(&crc32(&chunk).to_be_bytes());
    }
    png
}

/// A JPEG with an EXIF segment that says the camera was turned: orientation
/// 6, which shows the image turned 90 degrees clockwise.
fn turned_jpeg(width: u32, height: u32) -> Vec<u8> {
    let jpeg = encoded(
        &DynamicImage::ImageRgb8(gradient(width, height)),
        Format::Jpeg,
    );
    let mut exif = b"Exif\0\0MM\0\x2a\0\0\0\x08".to_vec();
    exif.extend_from_slice(&[0, 1]);
    exif.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0]);
    exif.extend_from_slice(&[0, 0, 0, 0]);
    let length = u16::try_from(exif.len() + 2).unwrap();
    let mut turned = jpeg[..2].to_vec();
    turned.extend_from_slice(&[0xff, 0xe1]);
    turned.extend_from_slice(&length.to_be_bytes());
    turned.extend_from_slice(&exif);
    turned.extend_from_slice(&jpeg[2..]);
    turned
}

#[test]
fn a_small_png_keeps_its_size_and_format_and_gets_a_jpeg_thumbnail() {
    let png = encoded(&DynamicImage::ImageRgb8(gradient(300, 200)), Format::Png);

    let prepared = prepare("C:\\Shots\\shot.png", &png).unwrap();

    assert_eq!(prepared.name, "shot.png");
    assert_eq!(prepared.format, ImageFormat::Png);
    assert_eq!((prepared.width, prepared.height), (300, 200));
    assert_eq!(decoded(prepared.data.as_slice()).dimensions(), (300, 200));
    assert_eq!(prepared.thumbnail_format, ImageFormat::Jpeg);
    assert_eq!(
        decoded(prepared.thumbnail.as_slice()).dimensions(),
        (300, 200)
    );
}

#[test]
fn a_jpeg_stays_a_jpeg_and_gif_and_webp_become_png() {
    let picture = DynamicImage::ImageRgb8(gradient(120, 80));
    let jpeg = encoded(&picture, Format::Jpeg);
    let gif = encoded(&DynamicImage::ImageRgba8(picture.to_rgba8()), Format::Gif);
    let webp = encoded(&DynamicImage::ImageRgba8(picture.to_rgba8()), Format::WebP);

    assert_eq!(prepare("a.jpg", &jpeg).unwrap().format, ImageFormat::Jpeg);
    for (name, bytes) in [("a.gif", gif), ("a.webp", webp)] {
        let prepared = prepare(name, &bytes).unwrap();
        assert_eq!(prepared.format, ImageFormat::Png, "{name}");
        assert_eq!((prepared.width, prepared.height), (120, 80), "{name}");
    }
}

#[test]
fn a_large_image_shrinks_to_2048_pixels_and_its_thumbnail_to_320() {
    let png = encoded(
        &DynamicImage::ImageRgb8(gradient(3_000, 1_000)),
        Format::Png,
    );

    let prepared = prepare("wide.png", &png).unwrap();

    assert_eq!((prepared.width, prepared.height), (2_048, 683));
    assert_eq!(decoded(prepared.data.as_slice()).dimensions(), (2_048, 683));
    let (width, height) = decoded(prepared.thumbnail.as_slice()).dimensions();
    assert_eq!(width, 320);
    assert!((106..=107).contains(&height), "{height}");
}

#[test]
fn a_turned_photo_is_set_upright_and_loses_its_metadata() {
    let jpeg = turned_jpeg(40, 20);
    assert!(jpeg.windows(4).any(|window| window == b"Exif"));

    let prepared = prepare("photo.jpg", &jpeg).unwrap();

    assert_eq!((prepared.width, prepared.height), (20, 40));
    assert!(
        !prepared
            .data
            .as_slice()
            .windows(4)
            .any(|window| window == b"Exif"),
        "the prepared image holds no EXIF segment"
    );
}

#[test]
fn sixteen_bits_become_eight_and_alpha_stays_in_a_png() {
    let deep: ImageBuffer<Rgba<u16>, Vec<u16>> = ImageBuffer::from_fn(16, 16, |x, _y| {
        Rgba([u16::try_from(x).unwrap() * 4_000, 0, 0, 30_000])
    });
    let png = encoded(&DynamicImage::ImageRgba16(deep), Format::Png);

    let prepared = prepare("deep.png", &png).unwrap();

    let image = decoded(prepared.data.as_slice());
    assert_eq!(image.color(), image::ColorType::Rgba8);
    assert_eq!(prepared.thumbnail_format, ImageFormat::Png);
}

#[test]
fn a_png_that_stays_too_large_becomes_a_jpeg_on_white() {
    let side = 1_200;
    let mut pixels = noise(side * side * 4);
    // A corner without coverage turns white.
    for row in 0..32 {
        for column in 0..32 {
            pixels[(row * side + column) * 4 + 3] = 0;
        }
    }
    let side = u32::try_from(side).unwrap();
    let image = RgbaImage::from_raw(side, side, pixels).unwrap();
    let png = encoded(&DynamicImage::ImageRgba8(image), Format::Png);
    assert!(png.len() > MAX_IMAGE_BYTES && png.len() <= MAX_FILE_BYTES);

    let prepared = prepare("noise.png", &png).unwrap();

    assert_eq!(prepared.format, ImageFormat::Jpeg);
    assert!(prepared.data.len() <= MAX_IMAGE_BYTES);
    let corner = decoded(prepared.data.as_slice())
        .to_rgb8()
        .get_pixel(4, 4)
        .0;
    assert!(corner.iter().all(|channel| *channel > 240), "{corner:?}");
}

#[test]
fn files_and_images_beyond_the_limits_are_refused_before_decoding() {
    let large = vec![0; MAX_FILE_BYTES + 1];

    assert_eq!(
        prepare("large.png", &large).unwrap_err(),
        ImagesError::file_too_large()
    );
    assert_eq!(
        prepare("wide.png", &png_header(20_000, 10)).unwrap_err(),
        ImagesError::TooWide {
            max_side: MAX_DECODED_SIDE
        }
    );
    assert_eq!(
        prepare("huge.png", &png_header(9_000, 9_000)).unwrap_err(),
        ImagesError::TooManyPixels {
            max_megapixels: MAX_DECODED_MEGAPIXELS
        }
    );
}

#[test]
fn damaged_files_and_other_formats_are_refused() {
    let png = encoded(&DynamicImage::ImageRgb8(gradient(200, 200)), Format::Png);
    let cut = &png[..png.len() / 2];
    // The signatures of formats that the plugin does not take: BMP and TIFF.
    let mut bmp = b"BM".to_vec();
    bmp.resize(64, 0);
    let mut tiff = b"II*\0".to_vec();
    tiff.resize(64, 0);

    assert_eq!(
        prepare("cut.png", cut).unwrap_err(),
        ImagesError::Unreadable
    );
    for (name, bytes) in [
        ("notes.png", b"not an image at all".to_vec()),
        ("a.bmp", bmp),
        ("a.tiff", tiff),
    ] {
        assert_eq!(
            prepare(name, &bytes).unwrap_err(),
            ImagesError::UnsupportedFormat,
            "{name}"
        );
    }
}
