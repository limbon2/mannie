//! Pure-Rust RGBA compositing for the system tray icon badge and Windows
//! taskbar count overlay (ADR-0074, decision 8).
//!
//! Neither Windows nor Linux `AppIndicator` places a separate icon next to a
//! tray icon, so the core composites a high-contrast badge dot onto a copy of
//! the app's default icon and swaps it through `TrayIcon::set_icon`. For the
//! Windows taskbar button, `taskbar_count_overlay` renders a 32x32 RGBA
//! numbered badge (`1` to `99` and `99+`) for `WebviewWindow::set_overlay_icon`.

use tauri::image::Image;

/// High-contrast notification badge color (vivid red-coral) that stands out on
/// both dark and light taskbars and against Mannie's monochrome icon.
const DOT_COLOR: [u8; 4] = [239, 68, 68, 255];

/// Contrasting ring behind the badge so it separates cleanly from icon edges
/// and bright or dark taskbars.
const RING_COLOR: [u8; 4] = [18, 18, 18, 240];

#[cfg(all(target_os = "windows", not(feature = "validation")))]
pub(crate) use overlay::taskbar_count_overlay;

/// Returns a copy of `base` with a circular badge dot composited onto the
/// bottom-right corner, or `None` if `base` has empty or mismatched pixel data.
pub(crate) fn with_badge_dot(base: &Image<'_>) -> Option<Image<'static>> {
    let width = base.width();
    let height = base.height();
    let rgba = composite_badge_rgba(width, height, base.rgba())?;
    Some(Image::new_owned(rgba, width, height))
}

/// Composites the badge dot onto a copy of `rgba` (`width * height * 4` bytes).
pub(crate) fn composite_badge_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    if width == 0 || height == 0 {
        return None;
    }

    let expected = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;

    if rgba.len() != expected {
        return None;
    }

    let mut out = rgba.to_vec();
    let w = f64::from(width);
    let h = f64::from(height);
    let size = w.min(h);

    // Sized to remain clearly visible when Windows scales the tray icon to
    // 16x16 while staying crisp at 32x32 and 128x128.
    let outer_radius = (size * 0.25).max(2.5);
    let inner_radius = (size * 0.19).max(1.8);
    let center_x = w - outer_radius - (size * 0.02);
    let center_y = h - outer_radius - (size * 0.02);

    let min_x = floor_clamped(center_x - outer_radius - 1.5, width);
    let max_x = ceil_clamped(center_x + outer_radius + 1.5, width);
    let min_y = floor_clamped(center_y - outer_radius - 1.5, height);
    let max_y = ceil_clamped(center_y + outer_radius + 1.5, height);

    for y in min_y..max_y {
        let dy = (f64::from(y) + 0.5) - center_y;

        for x in min_x..max_x {
            let dx = (f64::from(x) + 0.5) - center_x;
            let distance = dx.hypot(dy);
            let ring_cov = circle_coverage(distance, outer_radius);

            if ring_cov <= 0.0 {
                continue;
            }

            let offset = ((y as usize) * (width as usize) + (x as usize)) * 4;
            let mut pixel = [
                out[offset],
                out[offset + 1],
                out[offset + 2],
                out[offset + 3],
            ];
            pixel = blend_over(pixel, scale_alpha(RING_COLOR, ring_cov));

            let dot_cov = circle_coverage(distance, inner_radius);

            if dot_cov > 0.0 {
                pixel = blend_over(pixel, scale_alpha(DOT_COLOR, dot_cov));
            }

            out[offset..offset + 4].copy_from_slice(&pixel);
        }
    }

    Some(out)
}

/// Smooth anti-aliased edge coverage in `[0.0, 1.0]` for a circle of `radius`.
fn circle_coverage(distance: f64, radius: f64) -> f64 {
    (radius + 0.5 - distance).clamp(0.0, 1.0)
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "coverage is clamped to [0.0, 1.0], so the product stays within [0, 255]"
)]
fn scale_alpha(color: [u8; 4], coverage: f64) -> [u8; 4] {
    let alpha = (f64::from(color[3]) * coverage).round().clamp(0.0, 255.0) as u8;
    [color[0], color[1], color[2], alpha]
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "values are clamped to [0.0, max] where max fits in u32"
)]
fn floor_clamped(value: f64, max: u32) -> u32 {
    value.floor().clamp(0.0, f64::from(max)) as u32
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "values are clamped to [0.0, max] where max fits in u32"
)]
fn ceil_clamped(value: f64, max: u32) -> u32 {
    value.ceil().clamp(0.0, f64::from(max)) as u32
}

/// Porter-Duff source-over alpha blending on straight RGBA pixels.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "output channels are normalized in [0.0, 1.0] and scaled to [0, 255]"
)]
fn blend_over(dst: [u8; 4], src: [u8; 4]) -> [u8; 4] {
    let sa = f64::from(src[3]) / 255.0;

    if sa <= 0.0 {
        return dst;
    }
    if sa >= 1.0 {
        return src;
    }

    let da = f64::from(dst[3]) / 255.0;
    let out_a = sa + da * (1.0 - sa);

    if out_a <= 0.0 {
        return [0, 0, 0, 0];
    }

    let blend = |s: u8, d: u8| -> u8 {
        let value = (f64::from(s) * sa + f64::from(d) * da * (1.0 - sa)) / out_a;
        value.round().clamp(0.0, 255.0) as u8
    };
    [
        blend(src[0], dst[0]),
        blend(src[1], dst[1]),
        blend(src[2], dst[2]),
        (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

/// Windows taskbar overlay renderer (`1` to `99` and `99+`). Compiled on
/// Windows non-validation builds and under unit tests on every platform.
#[cfg(any(all(target_os = "windows", not(feature = "validation")), test))]
mod overlay {
    use tauri::image::Image;

    use super::{DOT_COLOR, RING_COLOR, blend_over, circle_coverage, scale_alpha};

    /// Foreground color for numeric digits inside the taskbar overlay badge.
    pub(super) const DIGIT_COLOR: [u8; 4] = [255, 255, 255, 255];

    /// Pixel size of the Windows taskbar overlay icon (scales cleanly to 16x16
    /// at 100% DPI and 32x32 at 200% DPI).
    pub(super) const OVERLAY_SIZE: u32 = 32;

    /// Bold 10x16 bitmaps for single digits `1` to `9` (MSB = leftmost column
    /// of the 10-bit row, stored in the low 10 bits of each `u16`).
    const LARGE_WIDTH: usize = 10;
    const LARGE_HEIGHT: usize = 16;
    const LARGE_DIGITS: [[u16; LARGE_HEIGHT]; 9] = [
        // 1
        [
            0b00_0011_1000,
            0b00_0111_1000,
            0b00_1111_1000,
            0b01_1111_1000,
            0b01_1011_1000,
            0b00_0011_1000,
            0b00_0011_1000,
            0b00_0011_1000,
            0b00_0011_1000,
            0b00_0011_1000,
            0b00_0011_1000,
            0b00_0011_1000,
            0b00_0011_1000,
            0b01_1111_1110,
            0b01_1111_1110,
            0b01_1111_1110,
        ],
        // 2
        [
            0b00_1111_1100,
            0b01_1111_1110,
            0b11_1111_1111,
            0b11_1000_0111,
            0b11_1000_0111,
            0b00_0000_0111,
            0b00_0000_1110,
            0b00_0001_1100,
            0b00_0011_1000,
            0b00_0111_0000,
            0b00_1110_0000,
            0b01_1100_0000,
            0b11_1000_0000,
            0b11_1111_1111,
            0b11_1111_1111,
            0b11_1111_1111,
        ],
        // 3
        [
            0b00_1111_1100,
            0b01_1111_1110,
            0b11_1111_1111,
            0b11_1000_0111,
            0b00_0000_0111,
            0b00_0000_1110,
            0b00_0111_1100,
            0b00_0111_1100,
            0b00_0111_1110,
            0b00_0000_0111,
            0b00_0000_0111,
            0b11_1000_0111,
            0b11_1000_0111,
            0b11_1111_1111,
            0b01_1111_1110,
            0b00_1111_1100,
        ],
        // 4
        [
            0b00_0001_1100,
            0b00_0011_1100,
            0b00_0111_1100,
            0b00_1111_1100,
            0b01_1101_1100,
            0b11_1001_1100,
            0b11_1001_1100,
            0b11_1001_1100,
            0b11_1001_1100,
            0b11_1111_1111,
            0b11_1111_1111,
            0b11_1111_1111,
            0b00_0001_1100,
            0b00_0001_1100,
            0b00_0001_1100,
            0b00_0001_1100,
        ],
        // 5
        [
            0b11_1111_1111,
            0b11_1111_1111,
            0b11_1111_1111,
            0b11_1000_0000,
            0b11_1000_0000,
            0b11_1111_1100,
            0b11_1111_1110,
            0b11_1111_1111,
            0b00_0000_0111,
            0b00_0000_0111,
            0b00_0000_0111,
            0b11_1000_0111,
            0b11_1000_0111,
            0b11_1111_1111,
            0b01_1111_1110,
            0b00_1111_1100,
        ],
        // 6
        [
            0b00_1111_1100,
            0b01_1111_1110,
            0b11_1111_1111,
            0b11_1000_0111,
            0b11_1000_0000,
            0b11_1000_0000,
            0b11_1111_1100,
            0b11_1111_1110,
            0b11_1111_1111,
            0b11_1000_0111,
            0b11_1000_0111,
            0b11_1000_0111,
            0b11_1000_0111,
            0b11_1111_1111,
            0b01_1111_1110,
            0b00_1111_1100,
        ],
        // 7
        [
            0b11_1111_1111,
            0b11_1111_1111,
            0b11_1111_1111,
            0b00_0000_0111,
            0b00_0000_1110,
            0b00_0000_1110,
            0b00_0001_1100,
            0b00_0001_1100,
            0b00_0011_1000,
            0b00_0011_1000,
            0b00_0111_0000,
            0b00_0111_0000,
            0b00_1110_0000,
            0b00_1110_0000,
            0b00_1110_0000,
            0b00_1110_0000,
        ],
        // 8
        [
            0b00_1111_1100,
            0b01_1111_1110,
            0b11_1111_1111,
            0b11_1000_0111,
            0b11_1000_0111,
            0b11_1000_0111,
            0b01_1111_1110,
            0b00_1111_1100,
            0b01_1111_1110,
            0b11_1000_0111,
            0b11_1000_0111,
            0b11_1000_0111,
            0b11_1000_0111,
            0b11_1111_1111,
            0b01_1111_1110,
            0b00_1111_1100,
        ],
        // 9
        [
            0b00_1111_1100,
            0b01_1111_1110,
            0b11_1111_1111,
            0b11_1000_0111,
            0b11_1000_0111,
            0b11_1000_0111,
            0b11_1000_0111,
            0b11_1111_1111,
            0b01_1111_1111,
            0b00_1111_1111,
            0b00_0000_0111,
            0b00_0000_0111,
            0b11_1000_0111,
            0b11_1111_1111,
            0b01_1111_1110,
            0b00_1111_1100,
        ],
    ];

    /// Bold 7x12 bitmaps for multi-digit badges `10` to `99` and `99+` (stored
    /// in the low 7 bits of each `u8`).
    const COMPACT_WIDTH: usize = 7;
    const COMPACT_HEIGHT: usize = 12;
    const COMPACT_DIGITS: [[u8; COMPACT_HEIGHT]; 10] = [
        // 0
        [
            0b011_1110, 0b111_1111, 0b110_0011, 0b110_0011, 0b110_0011, 0b110_0011, 0b110_0011,
            0b110_0011, 0b110_0011, 0b110_0011, 0b111_1111, 0b011_1110,
        ],
        // 1
        [
            0b001_1100, 0b011_1100, 0b111_1100, 0b110_1100, 0b000_1100, 0b000_1100, 0b000_1100,
            0b000_1100, 0b000_1100, 0b000_1100, 0b111_1111, 0b111_1111,
        ],
        // 2
        [
            0b011_1110, 0b111_1111, 0b110_0011, 0b000_0011, 0b000_0110, 0b000_1100, 0b001_1000,
            0b011_0000, 0b110_0000, 0b110_0000, 0b111_1111, 0b111_1111,
        ],
        // 3
        [
            0b011_1110, 0b111_1111, 0b110_0011, 0b000_0011, 0b000_0110, 0b001_1110, 0b001_1110,
            0b000_0011, 0b000_0011, 0b110_0011, 0b111_1111, 0b011_1110,
        ],
        // 4
        [
            0b000_1110, 0b001_1110, 0b011_0110, 0b110_0110, 0b110_0110, 0b110_0110, 0b111_1111,
            0b111_1111, 0b000_0110, 0b000_0110, 0b000_0110, 0b000_0110,
        ],
        // 5
        [
            0b111_1111, 0b111_1111, 0b110_0000, 0b110_0000, 0b111_1110, 0b111_1111, 0b000_0011,
            0b000_0011, 0b000_0011, 0b110_0011, 0b111_1111, 0b011_1110,
        ],
        // 6
        [
            0b011_1110, 0b111_1111, 0b110_0011, 0b110_0000, 0b111_1110, 0b111_1111, 0b110_0011,
            0b110_0011, 0b110_0011, 0b110_0011, 0b111_1111, 0b011_1110,
        ],
        // 7
        [
            0b111_1111, 0b111_1111, 0b000_0011, 0b000_0110, 0b000_0110, 0b000_1100, 0b000_1100,
            0b001_1000, 0b001_1000, 0b011_0000, 0b011_0000, 0b011_0000,
        ],
        // 8
        [
            0b011_1110, 0b111_1111, 0b110_0011, 0b110_0011, 0b110_0011, 0b011_1110, 0b011_1110,
            0b110_0011, 0b110_0011, 0b110_0011, 0b111_1111, 0b011_1110,
        ],
        // 9
        [
            0b011_1110, 0b111_1111, 0b110_0011, 0b110_0011, 0b110_0011, 0b111_1111, 0b011_1111,
            0b000_0011, 0b000_0011, 0b110_0011, 0b111_1111, 0b011_1110,
        ],
    ];

    /// Bold 6x6 `+` sign used beside `99` when the count reaches 100 or more.
    const PLUS_WIDTH: usize = 6;
    const PLUS_ROWS: [u8; 6] = [
        0b00_1100, 0b00_1100, 0b11_1111, 0b11_1111, 0b00_1100, 0b00_1100,
    ];

    /// Returns a 32x32 RGBA taskbar overlay icon showing `count` (`1` to `99`,
    /// and `99+` for 100 or more), or `None` when `count == 0`.
    pub(crate) fn taskbar_count_overlay(count: u32) -> Option<Image<'static>> {
        let rgba = render_taskbar_count_rgba(count)?;
        Some(Image::new_owned(rgba, OVERLAY_SIZE, OVERLAY_SIZE))
    }

    /// Renders a 32x32 RGBA numbered badge (`1` to `99` in a circle, `99+` in a
    /// pill), or `None` when `count == 0`.
    pub(super) fn render_taskbar_count_rgba(count: u32) -> Option<Vec<u8>> {
        if count == 0 {
            return None;
        }

        let size = OVERLAY_SIZE as usize;
        let mut out = vec![0_u8; size * size * 4];

        if count >= 100 {
            draw_pill_background(&mut out, OVERLAY_SIZE);
            draw_compact_digit(&mut out, OVERLAY_SIZE, 9, 5, 10);
            draw_compact_digit(&mut out, OVERLAY_SIZE, 9, 13, 10);
            draw_plus(&mut out, OVERLAY_SIZE, 21, 13);
        } else if count >= 10 {
            draw_circle_background(&mut out, OVERLAY_SIZE);

            let tens = (count / 10) as usize;
            let ones = (count % 10) as usize;
            draw_compact_digit(&mut out, OVERLAY_SIZE, tens, 8, 10);
            draw_compact_digit(&mut out, OVERLAY_SIZE, ones, 17, 10);
        } else {
            draw_circle_background(&mut out, OVERLAY_SIZE);
            draw_large_digit(&mut out, OVERLAY_SIZE, count as usize, 11, 8);
        }

        Some(out)
    }

    fn draw_circle_background(out: &mut [u8], size: u32) {
        let center = f64::from(size) * 0.5;
        let outer_radius = center - 0.5;
        let inner_radius = center - 1.7;

        for y in 0..size {
            let dy = (f64::from(y) + 0.5) - center;

            for x in 0..size {
                let dx = (f64::from(x) + 0.5) - center;
                let distance = dx.hypot(dy);
                let ring_cov = circle_coverage(distance, outer_radius);

                if ring_cov <= 0.0 {
                    continue;
                }

                let mut pixel = scale_alpha(RING_COLOR, ring_cov);
                let fill_cov = circle_coverage(distance, inner_radius);

                if fill_cov > 0.0 {
                    pixel = blend_over(pixel, scale_alpha(DOT_COLOR, fill_cov));
                }

                put_pixel(out, size, x as usize, y as usize, pixel);
            }
        }
    }

    fn draw_pill_background(out: &mut [u8], size: u32) {
        let center = f64::from(size) * 0.5;
        let outer_radius = 12.5;
        let inner_radius = 11.3;
        let half_span = (center - outer_radius - 0.5).max(0.0);

        for y in 0..size {
            let dy = (f64::from(y) + 0.5) - center;

            for x in 0..size {
                let dx = (f64::from(x) + 0.5) - center;
                let clamped_dx = dx.clamp(-half_span, half_span);
                let distance = (dx - clamped_dx).hypot(dy);
                let ring_cov = circle_coverage(distance, outer_radius);

                if ring_cov <= 0.0 {
                    continue;
                }

                let mut pixel = scale_alpha(RING_COLOR, ring_cov);
                let fill_cov = circle_coverage(distance, inner_radius);

                if fill_cov > 0.0 {
                    pixel = blend_over(pixel, scale_alpha(DOT_COLOR, fill_cov));
                }

                put_pixel(out, size, x as usize, y as usize, pixel);
            }
        }
    }

    fn draw_large_digit(out: &mut [u8], size: u32, digit: usize, origin_x: usize, origin_y: usize) {
        let Some(rows) = digit.checked_sub(1).and_then(|idx| LARGE_DIGITS.get(idx)) else {
            return;
        };

        for (row_idx, &bits) in rows.iter().enumerate() {
            for col in 0..LARGE_WIDTH {
                let mask = 1_u16 << (LARGE_WIDTH - 1 - col);

                if (bits & mask) != 0 {
                    put_pixel(out, size, origin_x + col, origin_y + row_idx, DIGIT_COLOR);
                }
            }
        }
    }

    fn draw_compact_digit(
        out: &mut [u8],
        size: u32,
        digit: usize,
        origin_x: usize,
        origin_y: usize,
    ) {
        let Some(rows) = COMPACT_DIGITS.get(digit) else {
            return;
        };

        for (row_idx, &bits) in rows.iter().enumerate() {
            for col in 0..COMPACT_WIDTH {
                let mask = 1_u8 << (COMPACT_WIDTH - 1 - col);

                if (bits & mask) != 0 {
                    put_pixel(out, size, origin_x + col, origin_y + row_idx, DIGIT_COLOR);
                }
            }
        }
    }

    fn draw_plus(out: &mut [u8], size: u32, origin_x: usize, origin_y: usize) {
        for (row_idx, &bits) in PLUS_ROWS.iter().enumerate() {
            for col in 0..PLUS_WIDTH {
                let mask = 1_u8 << (PLUS_WIDTH - 1 - col);

                if (bits & mask) != 0 {
                    put_pixel(out, size, origin_x + col, origin_y + row_idx, DIGIT_COLOR);
                }
            }
        }
    }

    fn put_pixel(out: &mut [u8], width: u32, x: usize, y: usize, color: [u8; 4]) {
        let offset = (y * (width as usize) + x) * 4;

        if offset + 4 <= out.len() {
            out[offset..offset + 4].copy_from_slice(&color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::overlay::{
        DIGIT_COLOR, OVERLAY_SIZE, render_taskbar_count_rgba, taskbar_count_overlay,
    };
    use super::{DOT_COLOR, composite_badge_rgba};

    #[test]
    fn draws_badge_in_bottom_right_and_preserves_top_left_across_icon_sizes() {
        for size in [16_u32, 24, 32, 64, 128] {
            let base = vec![30_u8; (size as usize) * (size as usize) * 4];
            let badged = composite_badge_rgba(size, size, &base).expect("valid RGBA image");
            assert_eq!(badged.len(), base.len());

            // Top-left pixel stays untouched.
            assert_eq!(&badged[0..4], &[30, 30, 30, 30]);

            // Center of the badge dot in the bottom-right corner receives the
            // dot color.
            let sample = ((size as usize) * 3) / 4;
            let offset = (sample * (size as usize) + sample) * 4;
            assert_eq!(&badged[offset..offset + 4], &DOT_COLOR, "size {size}");
        }
    }

    #[test]
    fn rejects_empty_or_mismatched_buffers() {
        assert_eq!(composite_badge_rgba(0, 32, &[]), None);
        assert_eq!(composite_badge_rgba(32, 0, &[]), None);
        assert_eq!(composite_badge_rgba(32, 32, &[0; 10]), None);
    }

    #[test]
    fn taskbar_overlay_renders_distinct_numbered_badges_and_caps_at_99_plus() {
        assert!(render_taskbar_count_rgba(0).is_none());
        assert!(taskbar_count_overlay(0).is_none());

        let one = render_taskbar_count_rgba(1).expect("badge for 1");
        let two = render_taskbar_count_rgba(2).expect("badge for 2");
        let twelve = render_taskbar_count_rgba(12).expect("badge for 12");
        let ninety_nine = render_taskbar_count_rgba(99).expect("badge for 99");
        let hundred = render_taskbar_count_rgba(100).expect("badge for 99+");
        let many = render_taskbar_count_rgba(250).expect("badge for 99+");

        let expected_len = (OVERLAY_SIZE as usize) * (OVERLAY_SIZE as usize) * 4;

        for badge in [&one, &two, &twelve, &ninety_nine, &hundred] {
            assert_eq!(badge.len(), expected_len);
            // Corner pixel outside circle/pill remains fully transparent.
            assert_eq!(&badge[0..4], &[0, 0, 0, 0]);
            // Contains both vivid badge background and white digit pixels.
            assert!(badge.as_chunks::<4>().0.contains(&DOT_COLOR));
            assert!(badge.as_chunks::<4>().0.contains(&DIGIT_COLOR));
        }

        assert_ne!(one, two);
        assert_ne!(two, twelve);
        assert_ne!(twelve, ninety_nine);
        assert_ne!(ninety_nine, hundred);
        assert_eq!(hundred, many);

        let image = taskbar_count_overlay(3).expect("overlay image for 3");
        assert_eq!(image.width(), OVERLAY_SIZE);
        assert_eq!(image.height(), OVERLAY_SIZE);
    }
}
