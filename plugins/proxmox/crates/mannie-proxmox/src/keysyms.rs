//! X11 `KeySym` conversion for RFB 3.8 keyboard events (RFC 6143 §7.5.4).
//! Maps printable Unicode text, named special keys, and modifier combinations
//! such as `Ctrl+Alt+Delete` or `Alt+F2` into standard X11 `KeySyms`.

use crate::model::ProxmoxError;

/// Standard X11 `KeySym` constants used by QEMU's VNC server.
pub(crate) const KEYSYM_BACKSPACE: u32 = 0xff08;
pub(crate) const KEYSYM_TAB: u32 = 0xff09;
pub(crate) const KEYSYM_RETURN: u32 = 0xff0d;
pub(crate) const KEYSYM_PAUSE: u32 = 0xff13;
pub(crate) const KEYSYM_SCROLL_LOCK: u32 = 0xff14;
pub(crate) const KEYSYM_ESCAPE: u32 = 0xff1b;
pub(crate) const KEYSYM_DELETE: u32 = 0xffff;
pub(crate) const KEYSYM_HOME: u32 = 0xff50;
pub(crate) const KEYSYM_LEFT: u32 = 0xff51;
pub(crate) const KEYSYM_UP: u32 = 0xff52;
pub(crate) const KEYSYM_RIGHT: u32 = 0xff53;
pub(crate) const KEYSYM_DOWN: u32 = 0xff54;
pub(crate) const KEYSYM_PAGE_UP: u32 = 0xff55;
pub(crate) const KEYSYM_PAGE_DOWN: u32 = 0xff56;
pub(crate) const KEYSYM_END: u32 = 0xff57;
pub(crate) const KEYSYM_PRINT: u32 = 0xff61;
pub(crate) const KEYSYM_INSERT: u32 = 0xff63;
pub(crate) const KEYSYM_MENU: u32 = 0xff67;
pub(crate) const KEYSYM_NUM_LOCK: u32 = 0xff7f;
pub(crate) const KEYSYM_KP_ENTER: u32 = 0xff8d;
pub(crate) const KEYSYM_F1: u32 = 0xffbe;
pub(crate) const KEYSYM_F2: u32 = 0xffbf;
pub(crate) const KEYSYM_F3: u32 = 0xffc0;
pub(crate) const KEYSYM_F4: u32 = 0xffc1;
pub(crate) const KEYSYM_F5: u32 = 0xffc2;
pub(crate) const KEYSYM_F6: u32 = 0xffc3;
pub(crate) const KEYSYM_F7: u32 = 0xffc4;
pub(crate) const KEYSYM_F8: u32 = 0xffc5;
pub(crate) const KEYSYM_F9: u32 = 0xffc6;
pub(crate) const KEYSYM_F10: u32 = 0xffc7;
pub(crate) const KEYSYM_F11: u32 = 0xffc8;
pub(crate) const KEYSYM_F12: u32 = 0xffc9;
pub(crate) const KEYSYM_SHIFT_L: u32 = 0xffe1;
pub(crate) const KEYSYM_SHIFT_R: u32 = 0xffe2;
pub(crate) const KEYSYM_CONTROL_L: u32 = 0xffe3;
pub(crate) const KEYSYM_CONTROL_R: u32 = 0xffe4;
pub(crate) const KEYSYM_CAPS_LOCK: u32 = 0xffe5;
pub(crate) const KEYSYM_META_L: u32 = 0xffe7;
pub(crate) const KEYSYM_META_R: u32 = 0xffe8;
pub(crate) const KEYSYM_ALT_L: u32 = 0xffe9;
pub(crate) const KEYSYM_ALT_R: u32 = 0xffea;
pub(crate) const KEYSYM_SUPER_L: u32 = 0xffeb;
pub(crate) const KEYSYM_SUPER_R: u32 = 0xffec;

/// A single key press or modifier chord to send over RFB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyStroke {
    /// Modifiers held down before pressing `key` and released in reverse order after.
    pub(crate) modifiers: Vec<u32>,
    /// Primary X11 `KeySym` pressed and released.
    pub(crate) key: u32,
}

/// Whether `ch` requires holding `Shift_L` on a standard US QWERTY keyboard.
///
/// QEMU's VNC server (`ui/vnc.c`) only auto-synthesizes Shift/CapsLock for
/// ASCII letters (`A`..`Z`). For shifted punctuation symbols it masks off
/// `SCANCODE_SHIFT` and injects the unshifted PS/2 scancode unless the VNC
/// client explicitly holds `Shift_L` down around the keystroke.
pub(crate) fn requires_us_shift(ch: char) -> bool {
    ch.is_ascii_uppercase() || is_us_shifted_punctuation(ch)
}

fn is_us_shifted_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '~' | '!'
            | '@'
            | '#'
            | '$'
            | '%'
            | '^'
            | '&'
            | '*'
            | '('
            | ')'
            | '_'
            | '+'
            | '{'
            | '}'
            | '|'
            | ':'
            | '"'
            | '<'
            | '>'
            | '?'
    )
}

/// Converts a single character to its X11 `KeySym`.
pub(crate) fn char_to_keysym(ch: char) -> Option<u32> {
    match ch {
        '\n' | '\r' => Some(KEYSYM_RETURN),
        '\t' => Some(KEYSYM_TAB),
        '\x08' => Some(KEYSYM_BACKSPACE),
        '\x1b' => Some(KEYSYM_ESCAPE),
        '\x7f' => Some(KEYSYM_DELETE),
        c if (' '..='~').contains(&c) => Some(c as u32),
        c if ('\u{00a0}'..='\u{00ff}').contains(&c) => Some(c as u32),
        c if !c.is_control() => Some(0x0100_0000 | (c as u32)),
        _ => None,
    }
}

/// Looks up a named key (case-insensitive) or single printable character as an
/// X11 `KeySym`.
pub(crate) fn name_to_keysym(name: &str) -> Option<u32> {
    let trimmed = name.trim();

    if trimmed.is_empty() {
        return None;
    }

    let mut chars = trimmed.chars();
    if let (Some(single), None) = (chars.next(), chars.next()) {
        return char_to_keysym(single);
    }

    match trimmed.to_ascii_lowercase().as_str() {
        "enter" | "return" | "cr" => Some(KEYSYM_RETURN),
        "kp_enter" | "numpadenter" => Some(KEYSYM_KP_ENTER),
        "tab" => Some(KEYSYM_TAB),
        "esc" | "escape" => Some(KEYSYM_ESCAPE),
        "backspace" | "bs" => Some(KEYSYM_BACKSPACE),
        "del" | "delete" => Some(KEYSYM_DELETE),
        "ins" | "insert" => Some(KEYSYM_INSERT),
        "space" | "spacebar" => Some(0x0020),
        "plus" => Some(u32::from(b'+')),
        "minus" => Some(u32::from(b'-')),
        "home" => Some(KEYSYM_HOME),
        "end" => Some(KEYSYM_END),
        "pageup" | "page_up" | "pgup" => Some(KEYSYM_PAGE_UP),
        "pagedown" | "page_down" | "pgdn" | "pgdown" => Some(KEYSYM_PAGE_DOWN),
        "up" | "arrowup" => Some(KEYSYM_UP),
        "down" | "arrowdown" => Some(KEYSYM_DOWN),
        "left" | "arrowleft" => Some(KEYSYM_LEFT),
        "right" | "arrowright" => Some(KEYSYM_RIGHT),
        "f1" => Some(KEYSYM_F1),
        "f2" => Some(KEYSYM_F2),
        "f3" => Some(KEYSYM_F3),
        "f4" => Some(KEYSYM_F4),
        "f5" => Some(KEYSYM_F5),
        "f6" => Some(KEYSYM_F6),
        "f7" => Some(KEYSYM_F7),
        "f8" => Some(KEYSYM_F8),
        "f9" => Some(KEYSYM_F9),
        "f10" => Some(KEYSYM_F10),
        "f11" => Some(KEYSYM_F11),
        "f12" => Some(KEYSYM_F12),
        "shift" | "shift_l" | "shiftleft" => Some(KEYSYM_SHIFT_L),
        "shift_r" | "shiftright" => Some(KEYSYM_SHIFT_R),
        "ctrl" | "control" | "control_l" | "controlleft" => Some(KEYSYM_CONTROL_L),
        "ctrl_r" | "control_r" | "controlright" => Some(KEYSYM_CONTROL_R),
        "alt" | "alt_l" | "altleft" => Some(KEYSYM_ALT_L),
        "alt_r" | "altright" | "altgraph" => Some(KEYSYM_ALT_R),
        "meta" | "meta_l" => Some(KEYSYM_META_L),
        "meta_r" => Some(KEYSYM_META_R),
        "super" | "super_l" | "win" | "cmd" | "command" | "os" => Some(KEYSYM_SUPER_L),
        "super_r" => Some(KEYSYM_SUPER_R),
        "capslock" | "caps_lock" => Some(KEYSYM_CAPS_LOCK),
        "numlock" | "num_lock" => Some(KEYSYM_NUM_LOCK),
        "scrolllock" | "scroll_lock" => Some(KEYSYM_SCROLL_LOCK),
        "print" | "printscreen" | "prtsc" => Some(KEYSYM_PRINT),
        "pause" | "break" => Some(KEYSYM_PAUSE),
        "menu" | "contextmenu" => Some(KEYSYM_MENU),
        _ => None,
    }
}

/// Parses a key or modifier chord such as `"Enter"`, `"Alt+F2"`, or `"Ctrl+Alt+Delete"`.
pub(crate) fn parse_key_combo(spec: &str) -> Result<KeyStroke, ProxmoxError> {
    let trimmed = spec.trim();

    if trimmed.is_empty() {
        return Err(ProxmoxError::InvalidRequest {
            detail: "key name cannot be empty".to_owned(),
        });
    }

    // Handle literal `+` as a single shifted character on US layouts.
    if trimmed == "+" {
        return Ok(KeyStroke {
            modifiers: vec![KEYSYM_SHIFT_L],
            key: u32::from(b'+'),
        });
    }

    let parts: Vec<&str> = trimmed.split('+').collect();
    if parts.len() == 1 {
        let key = name_to_keysym(parts[0]).ok_or_else(|| ProxmoxError::InvalidRequest {
            detail: format!("unknown key {trimmed:?}"),
        })?;
        let single_char = single_char_of(parts[0]);
        let needs_shift = single_char.is_some_and(requires_us_shift)
            || parts[0].trim().eq_ignore_ascii_case("plus");
        let modifiers = if needs_shift {
            vec![KEYSYM_SHIFT_L]
        } else {
            Vec::new()
        };
        return Ok(KeyStroke { modifiers, key });
    }

    let mut modifiers = Vec::with_capacity(parts.len() - 1);
    for part in &parts[..parts.len() - 1] {
        let modifier = modifier_to_keysym(part).ok_or_else(|| ProxmoxError::InvalidRequest {
            detail: format!("unknown modifier {part:?} in {trimmed:?}"),
        })?;
        if !modifiers.contains(&modifier) {
            modifiers.push(modifier);
        }
    }

    let last = parts[parts.len() - 1];
    let (key, needs_punct_shift) = if last.is_empty() && trimmed.ends_with("++") {
        (u32::from(b'+'), true)
    } else {
        let keysym = name_to_keysym(last).ok_or_else(|| ProxmoxError::InvalidRequest {
            detail: format!("unknown key {last:?} in {trimmed:?}"),
        })?;
        let needs_shift = single_char_of(last).is_some_and(is_us_shifted_punctuation)
            || last.trim().eq_ignore_ascii_case("plus");
        (keysym, needs_shift)
    };
    let has_shift = modifiers.contains(&KEYSYM_SHIFT_L) || modifiers.contains(&KEYSYM_SHIFT_R);
    if needs_punct_shift && !has_shift {
        modifiers.push(KEYSYM_SHIFT_L);
    }

    Ok(KeyStroke { modifiers, key })
}

/// Converts a text string into a sequence of RFB keystrokes. Handles `\r\n` as
/// a single Return key press, and holds `Shift_L` around US-shifted characters
/// so QEMU's `en-us` VNC keymap emits shifted symbols accurately.
pub(crate) fn text_to_keystrokes(text: &str) -> Result<Vec<KeyStroke>, ProxmoxError> {
    let mut strokes = Vec::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\r' && chars.peek() == Some(&'\n') {
            chars.next();
        }

        let keysym = char_to_keysym(ch).ok_or_else(|| ProxmoxError::InvalidRequest {
            detail: format!("unsupported control character U+{:04X}", ch as u32),
        })?;
        let modifiers = if requires_us_shift(ch) {
            vec![KEYSYM_SHIFT_L]
        } else {
            Vec::new()
        };
        strokes.push(KeyStroke {
            modifiers,
            key: keysym,
        });
    }

    Ok(strokes)
}

fn single_char_of(raw: &str) -> Option<char> {
    let mut chars = raw.trim().chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => Some(ch),
        _ => None,
    }
}

/// Standard `Ctrl+Alt+Delete` chord.
pub(crate) fn ctrl_alt_delete() -> KeyStroke {
    KeyStroke {
        modifiers: vec![KEYSYM_CONTROL_L, KEYSYM_ALT_L],
        key: KEYSYM_DELETE,
    }
}

fn modifier_to_keysym(name: &str) -> Option<u32> {
    match name.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" | "control_l" => Some(KEYSYM_CONTROL_L),
        "ctrl_r" | "control_r" => Some(KEYSYM_CONTROL_R),
        "alt" | "alt_l" => Some(KEYSYM_ALT_L),
        "alt_r" => Some(KEYSYM_ALT_R),
        "shift" | "shift_l" => Some(KEYSYM_SHIFT_L),
        "shift_r" => Some(KEYSYM_SHIFT_R),
        "meta" | "meta_l" => Some(KEYSYM_META_L),
        "super" | "super_l" | "win" | "cmd" => Some(KEYSYM_SUPER_L),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        KEYSYM_ALT_L, KEYSYM_CONTROL_L, KEYSYM_DELETE, KEYSYM_F2, KEYSYM_RETURN, KEYSYM_SHIFT_L,
        parse_key_combo, text_to_keystrokes,
    };

    #[test]
    fn parses_chords_and_special_keys() {
        let cad = parse_key_combo("Ctrl+Alt+Delete").unwrap();
        assert_eq!(cad.modifiers, vec![KEYSYM_CONTROL_L, KEYSYM_ALT_L]);
        assert_eq!(cad.key, KEYSYM_DELETE);

        let alt_f2 = parse_key_combo("Alt+F2").unwrap();
        assert_eq!(alt_f2.modifiers, vec![KEYSYM_ALT_L]);
        assert_eq!(alt_f2.key, KEYSYM_F2);

        let enter = parse_key_combo("Enter").unwrap();
        assert_eq!(enter.modifiers, Vec::<u32>::new());
        assert_eq!(enter.key, KEYSYM_RETURN);

        // Single shifted punctuation and uppercase letters hold Shift_L.
        for spec in ["+", "plus", "|", ":", "%", "{", "}", "_", "A"] {
            let stroke = parse_key_combo(spec).unwrap();
            assert_eq!(
                stroke.modifiers,
                vec![KEYSYM_SHIFT_L],
                "expected Shift_L for single key {spec:?}"
            );
        }

        // Explicit Ctrl+C must not inject Shift_L (which would become Ctrl+Shift+C).
        let ctrl_c = parse_key_combo("Ctrl+C").unwrap();
        assert_eq!(ctrl_c.modifiers, vec![KEYSYM_CONTROL_L]);

        let ctrl_shift_c = parse_key_combo("Ctrl+Shift+C").unwrap();
        assert_eq!(
            ctrl_shift_c.modifiers,
            vec![KEYSYM_CONTROL_L, KEYSYM_SHIFT_L]
        );
    }

    #[test]
    fn converts_text_and_crlf_into_keystrokes() {
        let strokes = text_to_keystrokes("hi\r\n").unwrap();
        let keys: Vec<u32> = strokes.iter().map(|s| s.key).collect();
        assert_eq!(keys, vec![u32::from(b'h'), u32::from(b'i'), KEYSYM_RETURN]);
        assert!(strokes.iter().all(|s| s.modifiers.is_empty()));
    }

    #[test]
    fn holds_shift_for_all_us_shifted_punctuation_and_uppercase() {
        let shifted = "ABCDEFGHIJKLMNOPQRSTUVWXYZ~!@#$%^&*()_+{}|:\"<>?";
        for stroke in text_to_keystrokes(shifted).unwrap() {
            assert_eq!(
                stroke.modifiers,
                vec![KEYSYM_SHIFT_L],
                "missing Shift_L for keysym 0x{:x}",
                stroke.key
            );
        }

        let unshifted = "abcdefghijklmnopqrstuvwxyz0123456789`-=[]\\;',./ \n";
        for stroke in text_to_keystrokes(unshifted).unwrap() {
            assert!(
                stroke.modifiers.is_empty(),
                "unexpected modifier {:?} for keysym 0x{:x}",
                stroke.modifiers,
                stroke.key
            );
        }
    }
}
