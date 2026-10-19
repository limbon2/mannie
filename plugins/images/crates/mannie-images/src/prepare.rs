//! Preparing an image for the model (plugins/images/docs/README.md, what Mannie does
//! with an image): check its size and its dimensions before decoding, decode
//! it within limits, turn it upright, shrink it, and encode it again, which
//! drops its metadata (T-32, T-33).

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::error::LimitErrorKind;
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{
    ColorType, DynamicImage, ImageDecoder, ImageError, ImageReader, Limits, Rgb, RgbImage,
};
use mannie_plugin_sdk::contracts::{
    ImageBytes, ImageFormat, MAX_IMAGE_BYTES, MAX_IMAGE_SIDE, MAX_THUMBNAIL_SIDE, clean_image_name,
};
use mannie_plugin_sdk::points::images::SourcedImage;

use crate::commands::{ImagesError, MAX_DECODED_MEGAPIXELS, MAX_DECODED_SIDE, MAX_FILE_BYTES};

/// The quality of the JPEG images that the plugin encodes.
const JPEG_QUALITY: u8 = 85;

/// The most bytes that decoding one image may allocate: 64 megapixels of
/// 16-bit RGBA, the widest kind of pixel that the plugin's formats decode to.
const MAX_DECODED_BYTES: u64 = 64 * 1_000_000 * 8;

/// Prepares an image from a file's bytes. Runs on a blocking thread: decoding
/// and encoding take a while for a large photo.
pub(crate) fn prepare(name: &str, bytes: &[u8]) -> Result<SourcedImage, ImagesError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ImagesError::file_too_large());
    }
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_io| ImagesError::Unreadable)?;
    let source = match reader.format() {
        Some(image::ImageFormat::Png | image::ImageFormat::Gif | image::ImageFormat::WebP) => {
            ImageFormat::Png
        }
        Some(image::ImageFormat::Jpeg) => ImageFormat::Jpeg,
        _ => return Err(ImagesError::UnsupportedFormat),
    };
    let image = decode(reader)?;
    let image = shrink(eight_bits(image));
    let (data, format) = encode(&image, source)?;
    let thumbnail_image = if image.width().max(image.height()) > MAX_THUMBNAIL_SIDE {
        image.thumbnail(MAX_THUMBNAIL_SIDE, MAX_THUMBNAIL_SIDE)
    } else {
        image.clone()
    };
    let thumbnail_format = if thumbnail_image.color().has_alpha() {
        ImageFormat::Png
    } else {
        ImageFormat::Jpeg
    };
    let thumbnail = encode_as(&thumbnail_image, thumbnail_format)?;
    let clean_name = clean_image_name(name);
    let width = image.width();
    let height = image.height();
    Ok(SourcedImage::new(
        clean_name,
        format,
        width,
        height,
        ImageBytes::from(data),
        thumbnail_format,
        ImageBytes::from(thumbnail),
    ))
}

/// Decodes an image whose header is within the limits, turned upright by its
/// EXIF orientation. An animation gives its first frame.
fn decode(reader: ImageReader<Cursor<&[u8]>>) -> Result<DynamicImage, ImagesError> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DECODED_SIDE);
    limits.max_image_height = Some(MAX_DECODED_SIDE);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    let mut reader = reader;
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(decoding_error)?;
    let (width, height) = decoder.dimensions();
    if width > MAX_DECODED_SIDE || height > MAX_DECODED_SIDE {
        return Err(ImagesError::TooWide {
            max_side: MAX_DECODED_SIDE,
        });
    }
    let pixels = u64::from(width) * u64::from(height);
    if pixels > u64::from(MAX_DECODED_MEGAPIXELS) * 1_000_000 {
        return Err(ImagesError::TooManyPixels {
            max_megapixels: MAX_DECODED_MEGAPIXELS,
        });
    }
    // The limits bound what the decoder allocates on its own; the image's
    // buffer is checked here, before it is allocated.
    if decoder.total_bytes() > MAX_DECODED_BYTES {
        return Err(ImagesError::TooManyPixels {
            max_megapixels: MAX_DECODED_MEGAPIXELS,
        });
    }
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(decoding_error)?;
    image.apply_orientation(orientation);
    Ok(image)
}

/// The error of an image that the decoder refused.
fn decoding_error(error: ImageError) -> ImagesError {
    match error {
        // The limits on dimensions are the only ones on a side.
        ImageError::Limits(limit) if limit.kind() == LimitErrorKind::DimensionError => {
            ImagesError::TooWide {
                max_side: MAX_DECODED_SIDE,
            }
        }
        ImageError::Limits(_) => ImagesError::TooManyPixels {
            max_megapixels: MAX_DECODED_MEGAPIXELS,
        },
        ImageError::Unsupported(_) => ImagesError::UnsupportedFormat,
        _other => ImagesError::Unreadable,
    }
}

/// The image with 8 bits for each channel, keeping gray and alpha, which both
/// encoders take.
fn eight_bits(image: DynamicImage) -> DynamicImage {
    match image.color() {
        ColorType::L8 | ColorType::La8 | ColorType::Rgb8 | ColorType::Rgba8 => image,
        ColorType::L16 => DynamicImage::ImageLuma8(image.to_luma8()),
        ColorType::La16 => DynamicImage::ImageLumaA8(image.to_luma_alpha8()),
        color if color.has_alpha() => DynamicImage::ImageRgba8(image.to_rgba8()),
        _ => DynamicImage::ImageRgb8(image.to_rgb8()),
    }
}

/// The image with its longer side at most [`MAX_IMAGE_SIDE`] pixels.
fn shrink(image: DynamicImage) -> DynamicImage {
    if image.width().max(image.height()) <= MAX_IMAGE_SIDE {
        return image;
    }
    image.resize(MAX_IMAGE_SIDE, MAX_IMAGE_SIDE, FilterType::Lanczos3)
}

/// Encodes the prepared image in the format of its source: a JPEG stays a
/// JPEG, and the rest become PNG. One that is still larger than Mannie sends
/// becomes a JPEG on white.
fn encode(
    image: &DynamicImage,
    source: ImageFormat,
) -> Result<(Vec<u8>, ImageFormat), ImagesError> {
    let data = encode_as(image, source)?;
    if data.len() <= MAX_IMAGE_BYTES {
        return Ok((data, source));
    }
    let data = encode_as(image, ImageFormat::Jpeg)?;
    if data.len() <= MAX_IMAGE_BYTES {
        return Ok((data, ImageFormat::Jpeg));
    }
    Err(ImagesError::cannot_shrink())
}

/// Encodes an image as PNG, or as a JPEG on white, which has no alpha.
fn encode_as(image: &DynamicImage, format: ImageFormat) -> Result<Vec<u8>, ImagesError> {
    let mut out = Vec::new();
    let written = match format {
        ImageFormat::Png => image.write_with_encoder(PngEncoder::new(&mut out)),
        ImageFormat::Jpeg => {
            let opaque = on_white(image);
            opaque.write_with_encoder(JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY))
        }
    };
    written.map_err(|error| {
        tracing::error!(%error, "an image could not be encoded");
        ImagesError::Unreadable
    })?;
    Ok(out)
}

/// The image without alpha: transparent pixels become white, as a page shows
/// them.
fn on_white(image: &DynamicImage) -> DynamicImage {
    match image.color() {
        ColorType::L8 | ColorType::Rgb8 => image.clone(),
        color if !color.has_alpha() => DynamicImage::ImageRgb8(image.to_rgb8()),
        _ => {
            let rgba = image.to_rgba8();
            let mut flat = RgbImage::new(rgba.width(), rgba.height());
            for (target, source) in flat.pixels_mut().zip(rgba.pixels()) {
                let [red, green, blue, alpha] = source.0;
                let blend = |channel: u8| {
                    let covered = u16::from(channel) * u16::from(alpha);
                    let white = 255 * u16::from(255 - alpha);
                    u8::try_from((covered + white + 127) / 255).unwrap_or(u8::MAX)
                };
                *target = Rgb([blend(red), blend(green), blend(blue)]);
            }
            DynamicImage::ImageRgb8(flat)
        }
    }
}

#[cfg(test)]
mod tests;
