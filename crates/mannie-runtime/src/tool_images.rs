//! The images of tool results (ADR-0044): the format and the size of each
//! one, read from its file's header without decoding its pixels, and the file
//! without its metadata, which the cleaner removes by the file's structure. A
//! photo that a tool returns would otherwise send where it was taken to the
//! provider (T-33).

use mannie_types::images::{ImageBytes, ImageFormat};
use mannie_types::tools::{MAX_TOOL_IMAGE_BYTES, MAX_TOOL_IMAGE_SIDE, MAX_TOOL_IMAGES};

use crate::texts;

/// An image of a result within the limits, without its metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanImage {
    /// The format of the bytes.
    pub(crate) format: ImageFormat,
    /// The width in pixels.
    pub(crate) width: u32,
    /// The height in pixels.
    pub(crate) height: u32,
    /// The file without its metadata.
    pub(crate) data: ImageBytes,
}

/// Why an image of a result does not reach the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LeftOut {
    /// It is neither PNG nor JPEG.
    Format,
    /// Its file is larger than [`MAX_TOOL_IMAGE_BYTES`].
    TooLarge,
    /// A side is longer than [`MAX_TOOL_IMAGE_SIDE`].
    TooWide {
        /// The width in pixels.
        width: u32,
        /// The height in pixels.
        height: u32,
    },
    /// Its file is damaged, so its size or its metadata cannot be read.
    Damaged,
}

/// The output of a result, ending with a line for each of its images that
/// does not reach the model, and the images that do, in their order with
/// their positions among the result's images. Only the first
/// [`MAX_TOOL_IMAGES`] are read, and one line names the rest.
pub(crate) fn clean_result(
    mut output: String,
    images: &[ImageBytes],
) -> (String, Vec<(u32, CleanImage)>) {
    let mut kept = Vec::new();
    let mut notes = Vec::new();
    for (index, image) in images.iter().take(MAX_TOOL_IMAGES).enumerate() {
        match clean(image.as_slice()) {
            Ok(image) => kept.push((u32::try_from(index).unwrap_or(u32::MAX), image)),
            Err(left_out) => notes.push(texts::tool_image_left_out(index + 1, left_out)),
        }
    }
    if images.len() > MAX_TOOL_IMAGES {
        notes.push(texts::tool_images_past_limit(
            MAX_TOOL_IMAGES + 1,
            images.len(),
        ));
    }
    for note in notes {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&note);
    }
    (output, kept)
}

/// The image of a file without its metadata, or why it does not reach the
/// model.
pub(crate) fn clean(data: &[u8]) -> Result<CleanImage, LeftOut> {
    if data.len() > MAX_TOOL_IMAGE_BYTES {
        return Err(LeftOut::TooLarge);
    }
    let (format, cleaned) = match ImageFormat::of(data) {
        Some(ImageFormat::Png) => (ImageFormat::Png, clean_png(data)?),
        Some(ImageFormat::Jpeg) => (ImageFormat::Jpeg, clean_jpeg(data)?),
        None => return Err(LeftOut::Format),
    };
    let Cleaned {
        width,
        height,
        bytes,
    } = cleaned;
    if width > MAX_TOOL_IMAGE_SIDE || height > MAX_TOOL_IMAGE_SIDE {
        return Err(LeftOut::TooWide { width, height });
    }
    Ok(CleanImage {
        format,
        width,
        height,
        data: ImageBytes::from(bytes),
    })
}

/// A file without its metadata, with the size that its header gives.
struct Cleaned {
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}

/// The big-endian `u16` at `at`, if the bytes hold one there.
fn be_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let field = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes(field.try_into().ok()?))
}

/// The big-endian `u32` at `at`, if the bytes hold one there.
fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let field = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes(field.try_into().ok()?))
}

/// The length of PNG's signature, which `ImageFormat::of` found.
const PNG_SIGNATURE_BYTES: usize = 8;

/// The ancillary PNG chunks that a clean file keeps: those that change how
/// its pixels look or move. A clean file keeps every critical chunk, which a
/// decoder needs, and drops every other ancillary one: text, EXIF, time,
/// provenance and whatever a later tool writes.
const PNG_KEPT: [&[u8; 4]; 14] = [
    b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"iCCP", b"sBIT", b"cICP", b"mDCV", b"cLLI", b"pHYs",
    b"bKGD", b"acTL", b"fcTL", b"fdAT",
];

/// A PNG file without its metadata. A chunk is its data's length, its type,
/// its data and a checksum; the header chunk comes first and the end chunk
/// last, and what follows the end chunk is dropped.
fn clean_png(data: &[u8]) -> Result<Cleaned, LeftOut> {
    let mut bytes = Vec::with_capacity(data.len());
    bytes.extend_from_slice(data.get(..PNG_SIGNATURE_BYTES).ok_or(LeftOut::Damaged)?);
    let mut at = PNG_SIGNATURE_BYTES;
    let mut size = None;
    loop {
        let length = be_u32(data, at).ok_or(LeftOut::Damaged)?;
        let end = usize::try_from(length)
            .ok()
            .and_then(|length| at.checked_add(12)?.checked_add(length))
            .ok_or(LeftOut::Damaged)?;
        let chunk = data.get(at..end).ok_or(LeftOut::Damaged)?;
        let kind = chunk.get(4..8).ok_or(LeftOut::Damaged)?;
        if !kind.iter().all(u8::is_ascii_alphabetic) {
            return Err(LeftOut::Damaged);
        }
        if size.is_none() {
            if kind != b"IHDR" || length != 13 {
                return Err(LeftOut::Damaged);
            }
            let width = be_u32(chunk, 8).ok_or(LeftOut::Damaged)?;
            let height = be_u32(chunk, 12).ok_or(LeftOut::Damaged)?;
            if width == 0 || height == 0 {
                return Err(LeftOut::Damaged);
            }
            size = Some((width, height));
        }
        // A chunk's first letter is upper case when a decoder needs it.
        let critical = kind[0].is_ascii_uppercase();
        if critical || PNG_KEPT.iter().any(|kept| kept.as_slice() == kind) {
            bytes.extend_from_slice(chunk);
        }
        if kind == b"IEND" {
            break;
        }
        at = end;
    }
    let (width, height) = size.ok_or(LeftOut::Damaged)?;
    Ok(Cleaned {
        width,
        height,
        bytes,
    })
}

/// The JPEG markers that the cleaner tells apart.
const SOI: u8 = 0xD8;
const EOI: u8 = 0xD9;
const SOS: u8 = 0xDA;
const TEM: u8 = 0x01;
const APP0: u8 = 0xE0;
const APP1: u8 = 0xE1;
const APP2: u8 = 0xE2;
const APP14: u8 = 0xEE;
const APP15: u8 = 0xEF;
const COM: u8 = 0xFE;

/// The starts of the application segments that a clean file keeps: the color
/// profile, and Adobe's segment, which says how the colors are coded.
const ICC_PROFILE: &[u8] = b"ICC_PROFILE\0";
const ADOBE: &[u8] = b"Adobe";

/// The start of an EXIF segment, which a clean file replaces with one that
/// holds only the orientation.
const EXIF: &[u8] = b"Exif\0\0";

/// Whether a marker starts a frame, whose header gives the image's size.
fn is_frame(marker: u8) -> bool {
    matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF)
}

/// A JPEG file without its metadata: without its application segments other
/// than the color profile and Adobe's, without comments, and with an EXIF
/// segment only for an orientation other than upright. A segment is a marker,
/// its length and its data; a scan's coded data follows its segment up to the
/// next marker, and what follows the end of the image is dropped.
fn clean_jpeg(data: &[u8]) -> Result<Cleaned, LeftOut> {
    let mut bytes = Vec::with_capacity(data.len());
    bytes.extend_from_slice(&[0xFF, SOI]);
    let mut at = 2;
    let mut size = None;
    let mut oriented = false;
    loop {
        if data.get(at) != Some(&0xFF) {
            return Err(LeftOut::Damaged);
        }
        // Any number of fill bytes may come before a marker's code.
        while data.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        let marker = *data.get(at + 1).ok_or(LeftOut::Damaged)?;
        match marker {
            EOI => {
                bytes.extend_from_slice(&[0xFF, EOI]);
                break;
            }
            0xD0..=0xD7 | TEM => {
                bytes.extend_from_slice(&[0xFF, marker]);
                at += 2;
                continue;
            }
            0x00 | SOI => return Err(LeftOut::Damaged),
            _ => {}
        }
        let length = usize::from(be_u16(data, at + 2).ok_or(LeftOut::Damaged)?);
        if length < 2 {
            return Err(LeftOut::Damaged);
        }
        let end = at + 2 + length;
        let segment = data.get(at..end).ok_or(LeftOut::Damaged)?;
        let content = &segment[4..];
        if is_frame(marker) && size.is_none() {
            // A frame's header: the precision, then the height and the width.
            let height = be_u16(content, 1).ok_or(LeftOut::Damaged)?;
            let width = be_u16(content, 3).ok_or(LeftOut::Damaged)?;
            // A height of 0 comes later in a segment that this reader does
            // not follow, so the file counts as damaged.
            if width == 0 || height == 0 {
                return Err(LeftOut::Damaged);
            }
            size = Some((u32::from(width), u32::from(height)));
        }
        match marker {
            APP1 if content.starts_with(EXIF) => {
                let orientation = exif_orientation(&content[EXIF.len()..]);
                if let Some(orientation) = orientation.filter(|_| !oriented) {
                    bytes.extend_from_slice(&orientation_segment(orientation));
                    oriented = true;
                }
            }
            APP2 if content.starts_with(ICC_PROFILE) => bytes.extend_from_slice(segment),
            APP14 if content.starts_with(ADOBE) => bytes.extend_from_slice(segment),
            APP0..=APP15 | COM => {}
            _ => bytes.extend_from_slice(segment),
        }
        at = end;
        if marker == SOS {
            let coded_end = scan_end(data, at).ok_or(LeftOut::Damaged)?;
            bytes.extend_from_slice(&data[at..coded_end]);
            at = coded_end;
        }
    }
    let (width, height) = size.ok_or(LeftOut::Damaged)?;
    Ok(Cleaned {
        width,
        height,
        bytes,
    })
}

/// Where a scan's coded data ends: at the first marker other than a restart.
/// Inside coded data, a byte 0xFF is followed by 0x00.
fn scan_end(data: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    loop {
        at += data.get(at..)?.iter().position(|byte| *byte == 0xFF)?;
        match data.get(at + 1)? {
            0x00 | 0xD0..=0xD7 => at += 2,
            _ => return Some(at),
        }
    }
}

/// EXIF's tag of the orientation, and its type, a `u16`.
const ORIENTATION_TAG: u16 = 0x0112;
const SHORT: u16 = 3;

/// The orientation that an EXIF segment's first directory gives, when it is
/// one of the turned or mirrored ones, 2 to 8. A TIFF header says the byte
/// order and where the first directory is; a directory is its number of
/// entries and the entries of 12 bytes each.
fn exif_orientation(tiff: &[u8]) -> Option<u16> {
    let big_endian = match tiff.get(..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let field: [u8; 2] = tiff.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(if big_endian {
            u16::from_be_bytes(field)
        } else {
            u16::from_le_bytes(field)
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let field: [u8; 4] = tiff.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if big_endian {
            u32::from_be_bytes(field)
        } else {
            u32::from_le_bytes(field)
        })
    };
    if u16_at(2)? != 42 {
        return None;
    }
    let directory = usize::try_from(u32_at(4)?).ok()?;
    let entries = usize::from(u16_at(directory)?);
    (0..entries)
        .find_map(|index| {
            let entry = directory
                .checked_add(2)?
                .checked_add(index.checked_mul(12)?)?;
            let is_orientation = u16_at(entry)? == ORIENTATION_TAG
                && u16_at(entry + 2)? == SHORT
                && u32_at(entry + 4)? == 1;
            is_orientation.then(|| u16_at(entry + 8)).flatten()
        })
        .filter(|orientation| (2..=8).contains(orientation))
}

/// An EXIF segment that holds only an orientation, so that a turned photo
/// still shows as it was taken: a big-endian TIFF header, then one directory
/// with one entry and no next directory.
fn orientation_segment(orientation: u16) -> Vec<u8> {
    let [high, low] = orientation.to_be_bytes();
    let mut segment = vec![0xFF, APP1, 0x00, 0x22];
    segment.extend_from_slice(EXIF);
    segment.extend_from_slice(&[b'M', b'M', 0x00, 0x2A, 0x00, 0x00, 0x00, 0x08]);
    segment.extend_from_slice(&[0x00, 0x01]);
    segment.extend_from_slice(&[
        0x01, 0x12, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01, high, low, 0x00, 0x00,
    ]);
    segment.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    segment
}

#[cfg(test)]
mod tests {
    use mannie_types::images::{ImageBytes, ImageFormat};
    use mannie_types::tools::{MAX_TOOL_IMAGE_BYTES, MAX_TOOL_IMAGE_SIDE, MAX_TOOL_IMAGES};

    use super::{LeftOut, clean, clean_result, exif_orientation};

    /// Planted metadata that no cleaned file may hold.
    const PLANTED: &[u8] = b"planted-metadata-51.5007N-0.1246W";

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    fn png_chunk(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
        let mut chunk = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
        chunk.extend_from_slice(&kind);
        chunk.extend_from_slice(data);
        // The cleaner keeps checksums as they are, so any four bytes do.
        chunk.extend_from_slice(&[0xC0, 0xFF, 0xEE, 0x00]);
        chunk
    }

    fn png(width: u32, height: u32, between: &[Vec<u8>]) -> Vec<u8> {
        let mut header = width.to_be_bytes().to_vec();
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut file = b"\x89PNG\r\n\x1a\n".to_vec();
        file.extend(png_chunk(*b"IHDR", &header));
        for chunk in between {
            file.extend_from_slice(chunk);
        }
        file.extend(png_chunk(*b"IDAT", b"pixels"));
        file.extend(png_chunk(*b"IEND", b""));
        file
    }

    fn jpeg_segment(marker: u8, content: &[u8]) -> Vec<u8> {
        let mut segment = vec![0xFF, marker];
        segment.extend_from_slice(&u16::try_from(content.len() + 2).unwrap().to_be_bytes());
        segment.extend_from_slice(content);
        segment
    }

    /// A TIFF of EXIF in either byte order, with the orientation and a
    /// planted entry after it.
    fn tiff(big_endian: bool, orientation: u16) -> Vec<u8> {
        let u16_bytes = |value: u16| {
            if big_endian {
                value.to_be_bytes()
            } else {
                value.to_le_bytes()
            }
        };
        let u32_bytes = |value: u32| {
            if big_endian {
                value.to_be_bytes()
            } else {
                value.to_le_bytes()
            }
        };
        let mut tiff = if big_endian {
            b"MM".to_vec()
        } else {
            b"II".to_vec()
        };
        tiff.extend_from_slice(&u16_bytes(42));
        tiff.extend_from_slice(&u32_bytes(8));
        tiff.extend_from_slice(&u16_bytes(2));
        tiff.extend_from_slice(&u16_bytes(0x8825));
        tiff.extend_from_slice(&u16_bytes(4));
        tiff.extend_from_slice(&u32_bytes(1));
        tiff.extend_from_slice(&u32_bytes(38));
        tiff.extend_from_slice(&u16_bytes(0x0112));
        tiff.extend_from_slice(&u16_bytes(3));
        tiff.extend_from_slice(&u32_bytes(1));
        tiff.extend_from_slice(&u16_bytes(orientation));
        tiff.extend_from_slice(&[0, 0]);
        tiff.extend_from_slice(&u32_bytes(0));
        tiff.extend_from_slice(PLANTED);
        tiff
    }

    fn jpeg(width: u16, height: u16, orientation: u16) -> Vec<u8> {
        let mut exif = b"Exif\0\0".to_vec();
        exif.extend(tiff(false, orientation));
        let mut xmp = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
        xmp.extend_from_slice(PLANTED);
        let mut iptc = b"Photoshop 3.0\0".to_vec();
        iptc.extend_from_slice(PLANTED);
        let mut frame = vec![8];
        frame.extend_from_slice(&height.to_be_bytes());
        frame.extend_from_slice(&width.to_be_bytes());
        frame.extend_from_slice(&[1, 1, 0x11, 0]);
        let mut file = vec![0xFF, 0xD8];
        file.extend(jpeg_segment(0xE0, b"JFIF\0\x01\x02\0\0\x01\0\x01\0\0"));
        file.extend(jpeg_segment(0xE1, &exif));
        file.extend(jpeg_segment(0xE1, &xmp));
        file.extend(jpeg_segment(0xED, &iptc));
        file.extend(jpeg_segment(0xFE, PLANTED));
        file.extend(jpeg_segment(0xE2, b"ICC_PROFILE\0\x01\x01profile"));
        file.extend(jpeg_segment(0xDB, &[0; 65]));
        file.extend(jpeg_segment(0xC0, &frame));
        file.extend(jpeg_segment(0xC4, &[0; 20]));
        file.extend(jpeg_segment(0xDA, &[1, 1, 0, 0, 0x3F, 0]));
        // Coded data, with a stuffed 0xFF and a restart marker inside it.
        file.extend_from_slice(&[0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56]);
        file.extend_from_slice(&[0xFF, 0xD9]);
        file.extend_from_slice(PLANTED);
        file
    }

    #[test]
    fn a_png_keeps_its_pixels_and_color_and_loses_its_text_exif_and_time() {
        let file = png(
            1280,
            720,
            &[
                png_chunk(*b"iCCP", b"profile"),
                png_chunk(*b"tEXt", PLANTED),
                png_chunk(*b"iTXt", PLANTED),
                png_chunk(*b"zTXt", PLANTED),
                png_chunk(*b"eXIf", PLANTED),
                png_chunk(*b"tIME", PLANTED),
                png_chunk(*b"caBX", PLANTED),
            ],
        );
        let mut trailing = file;
        trailing.extend_from_slice(PLANTED);

        let image = clean(&trailing).unwrap();

        assert_eq!(image.format, ImageFormat::Png);
        assert_eq!((image.width, image.height), (1280, 720));
        let bytes = image.data.as_slice();
        assert!(!contains(bytes, PLANTED));
        assert!(contains(bytes, b"iCCPprofile"));
        assert!(contains(bytes, b"IDATpixels"));
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR"));
        assert!(bytes.ends_with(b"IEND\xC0\xFF\xEE\x00"));
    }

    #[test]
    fn a_jpeg_keeps_its_scan_profile_and_orientation_and_loses_its_metadata() {
        let image = clean(&jpeg(4000 / 2, 3000 / 2, 6)).unwrap();

        assert_eq!(image.format, ImageFormat::Jpeg);
        assert_eq!((image.width, image.height), (2000, 1500));
        let bytes = image.data.as_slice();
        assert!(!contains(bytes, PLANTED));
        assert!(!contains(bytes, b"JFIF"));
        assert!(!contains(bytes, b"http://ns.adobe.com"));
        assert!(contains(bytes, b"ICC_PROFILE\0\x01\x01profile"));
        assert!(contains(bytes, &[0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56]));
        assert!(bytes.starts_with(&[0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x22]));
        assert!(bytes.ends_with(&[0xFF, 0xD9]));
        // The new EXIF segment holds the orientation alone, after the start
        // of the image, the segment's marker and length, and "Exif".
        assert_eq!(exif_orientation(&bytes[12..]), Some(6));
    }

    #[test]
    fn an_upright_jpeg_gets_no_exif_segment() {
        let image = clean(&jpeg(640, 480, 1)).unwrap();

        assert!(!contains(image.data.as_slice(), b"Exif"));
    }

    #[test]
    fn the_orientation_reads_in_either_byte_order() {
        assert_eq!(exif_orientation(&tiff(true, 8)), Some(8));
        assert_eq!(exif_orientation(&tiff(false, 3)), Some(3));
        assert_eq!(exif_orientation(&tiff(false, 1)), None);
        assert_eq!(exif_orientation(&tiff(false, 9)), None);
        assert_eq!(exif_orientation(b"XX\0*"), None);
        assert_eq!(exif_orientation(&tiff(true, 6)[..12]), None);
    }

    #[test]
    fn a_file_of_another_format_or_past_a_limit_is_left_out() {
        let side = u16::try_from(MAX_TOOL_IMAGE_SIDE + 1).unwrap();
        let mut large = png(16, 16, &[]);
        large.resize(MAX_TOOL_IMAGE_BYTES + 1, 0);

        assert_eq!(clean(b"GIF89a\x01\0\x01\0"), Err(LeftOut::Format));
        assert_eq!(clean(b"RIFF\0\0\0\0WEBPVP8 "), Err(LeftOut::Format));
        assert_eq!(clean(&large), Err(LeftOut::TooLarge));
        assert_eq!(
            clean(&png(MAX_TOOL_IMAGE_SIDE + 1, 10, &[])),
            Err(LeftOut::TooWide {
                width: MAX_TOOL_IMAGE_SIDE + 1,
                height: 10
            })
        );
        assert_eq!(
            clean(&jpeg(10, side, 1)),
            Err(LeftOut::TooWide {
                width: 10,
                height: MAX_TOOL_IMAGE_SIDE + 1
            })
        );
        let widest = clean(&png(MAX_TOOL_IMAGE_SIDE, MAX_TOOL_IMAGE_SIDE, &[])).unwrap();
        assert_eq!(widest.width, MAX_TOOL_IMAGE_SIDE);
    }

    #[test]
    fn a_damaged_file_is_left_out() {
        let whole = png(16, 16, &[]);
        let mut no_header = b"\x89PNG\r\n\x1a\n".to_vec();
        no_header.extend(png_chunk(*b"IDAT", b"pixels"));
        let zero_wide = png(0, 16, &[]);
        let jpeg_whole = jpeg(16, 16, 1);
        let mut jpeg_no_frame = vec![0xFF, 0xD8];
        jpeg_no_frame.extend_from_slice(&[0xFF, 0xD9]);

        for damaged in [
            &whole[..whole.len() - 3],
            &whole[..20],
            no_header.as_slice(),
            zero_wide.as_slice(),
            &jpeg_whole[..jpeg_whole.len() - PLANTED.len() - 2],
            jpeg_no_frame.as_slice(),
            &jpeg(0, 16, 1),
            b"\xff\xd8\xff",
        ] {
            assert_eq!(clean(damaged), Err(LeftOut::Damaged), "{damaged:?}");
        }
    }

    #[test]
    fn a_result_keeps_its_first_images_and_names_each_one_left_out() {
        let good = ImageBytes::from(png(8, 8, &[]));
        let gif = ImageBytes::from(b"GIF89a\x01\0\x01\0".to_vec());
        let mut images = vec![good.clone(), gif];
        images.extend((0..MAX_TOOL_IMAGES).map(|_| good.clone()));

        let (output, kept) = clean_result("A screenshot.".to_owned(), &images);

        let positions: Vec<u32> = kept.iter().map(|(position, _image)| *position).collect();
        assert_eq!(positions, vec![0, 2, 3]);
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3, "{output}");
        assert_eq!(lines[0], "A screenshot.");
        assert!(lines[1].contains("Image 2"), "{output}");
        assert!(lines[1].contains("neither a PNG nor a JPEG"), "{output}");
        assert!(lines[2].contains('5'), "{output}");
        assert!(lines[2].contains(&images.len().to_string()), "{output}");
        assert_eq!(
            clean_result("Done.".to_owned(), &[]),
            ("Done.".to_owned(), Vec::new())
        );
    }
}
