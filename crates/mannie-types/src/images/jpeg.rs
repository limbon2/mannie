//! Read JPEG markers and image dimensions without decoding pixels.

/// Start of image.
pub const SOI: u8 = 0xD8;
/// End of image.
pub const EOI: u8 = 0xD9;
/// Start of scan.
pub const SOS: u8 = 0xDA;

const TEM: u8 = 0x01;
const MARKER_PREFIX: u8 = 0xFF;

/// A JPEG marker and its bytes, without the fill bytes before the marker.
#[derive(Debug, Clone, Copy)]
pub struct Segment<'a> {
    /// The marker code.
    pub marker: u8,
    /// The segment, including the marker prefix, code and any length field.
    pub bytes: &'a [u8],
}

impl<'a> Segment<'a> {
    /// The bytes after the marker and length field, empty for a standalone marker.
    pub fn content(self) -> &'a [u8] {
        self.bytes.get(4..).unwrap_or_default()
    }

    /// Whether the marker starts a frame.
    pub fn is_frame(self) -> bool {
        matches!(self.marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF)
    }

    /// The width and height of a frame, in pixels.
    pub fn frame_dimensions(self) -> Option<(u32, u32)> {
        // A frame starts with sample precision, then height and width. A zero
        // height would require reading a later Define Number of Lines segment.
        let &[_, high_h, low_h, high_w, low_w, ..] = self.content() else {
            return None;
        };
        let height = u32::from(u16::from_be_bytes([high_h, low_h]));
        let width = u32::from(u16::from_be_bytes([high_w, low_w]));
        (width > 0 && height > 0).then_some((width, height))
    }
}

/// Reads a segment at a marker boundary, and returns the bytes after it.
/// After [`SOS`], the caller must skip the scan data to find the next boundary.
pub fn next_segment(mut data: &[u8]) -> Option<(Segment<'_>, &[u8])> {
    if data.first() != Some(&MARKER_PREFIX) {
        return None;
    }
    // Any number of fill bytes may come before the marker's code.
    while data.get(1) == Some(&MARKER_PREFIX) {
        data = &data[1..];
    }
    let marker = *data.get(1)?;
    let length = match marker {
        EOI | 0xD0..=0xD7 | TEM => 2,
        0x00 | SOI => return None,
        _ => {
            let length = u16::from_be_bytes(data.get(2..4)?.try_into().ok()?);
            if length < 2 {
                return None;
            }
            2 + usize::from(length)
        }
    };
    let (bytes, rest) = data.split_at_checked(length)?;
    Some((Segment { marker, bytes }, rest))
}

/// The width and height recorded in the first frame header of a JPEG image.
pub fn dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut data = data.strip_prefix(&[MARKER_PREFIX, SOI])?;
    loop {
        let (segment, rest) = next_segment(data)?;
        if segment.is_frame() {
            return segment.frame_dimensions();
        }
        if matches!(segment.marker, SOS | EOI) {
            return None;
        }
        data = rest;
    }
}

#[cfg(test)]
mod tests {
    use super::dimensions;

    #[test]
    fn reads_the_frame_after_application_segments_and_fill_bytes() {
        let data =
            b"\xff\xd8\xff\xe0\x00\x06JFIF\xff\xff\xc2\x00\x0b\x08\x05\x95\x07\xce\x01\x01\x11\x00";
        assert_eq!(dimensions(data), Some((1998, 1429)));
    }

    #[test]
    fn missing_truncated_and_empty_frames_have_no_dimensions() {
        for data in [
            &b"GIF89a"[..],
            &b"\xff\xd8\xff\xd9"[..],
            &b"\xff\xd8\xff\xc0\x00\x0b\x08\x05\x95\x07\xce"[..],
            &b"\xff\xd8\xff\xc0\x00\x0b\x08\x00\x00\x07\xce\x01\x01\x11\x00"[..],
        ] {
            assert_eq!(dimensions(data), None);
        }
    }
}
