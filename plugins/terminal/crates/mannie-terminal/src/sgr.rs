//! The attributes of text as vt100's "basic formatted" capture writes them:
//! SGR sequences that change only what differs from the row before. A row of
//! the text log starts with the attributes that it inherits, written in
//! full, so that a snapshot can start at any row
//! (plugins/terminal/docs/adr/0002-keep-a-headless-emulator-of-each-terminal-in-rust.md).
//!
//! The capture writes a closed set of parameters: 0 or none, the intensity
//! (1, 2, 22), italic (3, 23), underline (4, 24), inverse (7, 27), and the
//! colors 30 to 39, 40 to 49, 90 to 97, 100 to 107, `38;5;n`, `48;5;n`,
//! `38;2;r;g;b` and `48;2;r;g;b`. Parameters outside it are skipped.

use std::fmt::Write as _;

/// A color as the capture writes it, kept as the parameters that set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Color {
    /// The terminal's own color.
    #[default]
    Default,
    /// One of the 256 indexed colors.
    Indexed(u8),
    /// A true color.
    Rgb(u8, u8, u8),
}

/// The attributes in effect at a point of the capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Attributes {
    foreground: Color,
    background: Color,
    /// 0 for normal, 1 for bold, 2 for dim.
    intensity: u8,
    italic: bool,
    underline: bool,
    inverse: bool,
}

impl Attributes {
    /// Whether these are the terminal's defaults, which need no sequence.
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The sequence that sets these attributes from any others: a reset,
    /// then each attribute that is not the default. Empty for the defaults.
    pub(crate) fn sequence(&self) -> String {
        if self.is_default() {
            return String::new();
        }
        let mut params: Vec<String> = Vec::new();
        color_params(&mut params, self.foreground, 30);
        color_params(&mut params, self.background, 40);
        if self.intensity == 1 || self.intensity == 2 {
            params.push(self.intensity.to_string());
        }
        if self.italic {
            params.push("3".to_owned());
        }
        if self.underline {
            params.push("4".to_owned());
        }
        if self.inverse {
            params.push("7".to_owned());
        }
        let mut sequence = String::from("\x1b[m\x1b[");
        sequence.push_str(&params.join(";"));
        sequence.push('m');
        sequence
    }

    /// Applies the SGR sequences of `text`, in order.
    pub(crate) fn apply(&mut self, text: &str) {
        let mut rest = text;
        while let Some(start) = rest.find("\x1b[") {
            let after = &rest[start + 2..];
            let Some(end) = after.find('m') else {
                return;
            };
            self.apply_params(&after[..end]);
            rest = &after[end + 1..];
        }
    }

    fn apply_params(&mut self, params: &str) {
        let values: Vec<u16> = params
            .split(';')
            .map(|param| param.parse().unwrap_or(0))
            .collect();
        let mut index = 0;
        while index < values.len() {
            let value = values[index];
            index += 1;
            match value {
                0 => *self = Self::default(),
                1 => self.intensity = 1,
                2 => self.intensity = 2,
                22 => self.intensity = 0,
                3 => self.italic = true,
                23 => self.italic = false,
                4 => self.underline = true,
                24 => self.underline = false,
                7 => self.inverse = true,
                27 => self.inverse = false,
                30..=37 => self.foreground = Color::Indexed(index_of(value - 30)),
                90..=97 => self.foreground = Color::Indexed(index_of(value - 82)),
                39 => self.foreground = Color::Default,
                40..=47 => self.background = Color::Indexed(index_of(value - 40)),
                100..=107 => self.background = Color::Indexed(index_of(value - 92)),
                49 => self.background = Color::Default,
                38 | 48 => {
                    let (color, used) = extended_color(&values[index..]);
                    index += used;
                    if let Some(color) = color {
                        if value == 38 {
                            self.foreground = color;
                        } else {
                            self.background = color;
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// A parameter's color index, which the match arms keep below 16.
fn index_of(value: u16) -> u8 {
    u8::try_from(value).unwrap_or(0)
}

/// The color of the parameters after 38 or 48, and how many it used.
fn extended_color(values: &[u16]) -> (Option<Color>, usize) {
    let channel = |index: usize| {
        values
            .get(index)
            .and_then(|value| u8::try_from(*value).ok())
    };
    match values.first() {
        Some(5) => (channel(1).map(Color::Indexed), 2),
        Some(2) => {
            let color = match (channel(1), channel(2), channel(3)) {
                (Some(red), Some(green), Some(blue)) => Some(Color::Rgb(red, green, blue)),
                _ => None,
            };
            (color, 4)
        }
        _ => (None, 0),
    }
}

/// Writes the parameters of a color whose plain codes start at `base`: 30
/// for the foreground, 40 for the background.
fn color_params(params: &mut Vec<String>, color: Color, base: u16) {
    match color {
        Color::Default => {}
        Color::Indexed(index) if index < 8 => params.push((base + u16::from(index)).to_string()),
        Color::Indexed(index) if index < 16 => {
            params.push((base + 60 + u16::from(index) - 8).to_string());
        }
        Color::Indexed(index) => {
            let mut param = String::new();
            let _written = write!(param, "{};5;{index}", base + 8);
            params.push(param);
        }
        Color::Rgb(red, green, blue) => {
            let mut param = String::new();
            let _written = write!(param, "{};2;{red};{green};{blue}", base + 8);
            params.push(param);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Attributes;

    fn after(text: &str) -> Attributes {
        let mut attributes = Attributes::default();
        attributes.apply(text);
        attributes
    }

    #[test]
    fn the_defaults_need_no_sequence() {
        assert!(after("plain text").is_default());
        assert_eq!(after("plain").sequence(), "");
        assert!(after("\x1b[31mred\x1b[m").is_default());
        assert!(after("\x1b[1;31mx\x1b[22;39m").is_default());
    }

    #[test]
    fn changes_add_up_to_a_full_sequence() {
        let attributes = after("\x1b[31mred\x1b[1mbold\x1b[4m");
        assert_eq!(attributes.sequence(), "\x1b[m\x1b[31;1;4m");
        let bright = after("\x1b[92;104mx");
        assert_eq!(bright.sequence(), "\x1b[m\x1b[92;104m");
    }

    #[test]
    fn extended_colors_keep_their_parameters() {
        assert_eq!(after("\x1b[38;5;208m").sequence(), "\x1b[m\x1b[38;5;208m");
        assert_eq!(
            after("\x1b[48;2;10;20;30m").sequence(),
            "\x1b[m\x1b[48;2;10;20;30m"
        );
        assert_eq!(
            after("\x1b[38;5;208;3m").sequence(),
            "\x1b[m\x1b[38;5;208;3m"
        );
        // A broken color sets nothing.
        assert!(after("\x1b[38;2;10m").is_default());
    }

    #[test]
    fn a_sequence_sets_the_same_attributes_again() {
        let attributes = after("\x1b[7;2;95mx\x1b[23;27m");
        assert_eq!(after(&attributes.sequence()), attributes);
    }
}
