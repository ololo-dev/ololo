//! xterm key → PTY byte translation for the ololo TUI.

use crossterm::event::{KeyCode, KeyModifiers};

/// xterm's modifier parameter for CSI sequences: 1=none, then +1 shift,
/// +2 alt, +4 ctrl (e.g. Ctrl+Shift = 1+1+4 = 6).
fn xterm_modifier_code(modifiers: KeyModifiers) -> u8 {
    use crossterm::event::KeyModifiers;
    let mut n = 1u8;
    if modifiers.contains(KeyModifiers::SHIFT) {
        n += 1;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        n += 2;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        n += 4;
    }
    n
}

pub(crate) fn key_to_pty_bytes(code: KeyCode, modifiers: KeyModifiers) -> Option<Vec<u8>> {
    use crossterm::event::{KeyCode, KeyModifiers};

    // Cursor keys (arrows + Home/End): unmodified is the terse `ESC[<letter>`
    // form; any modifier switches to the extended `ESC[1;<n><letter>` form.
    let cursor_letter: Option<u8> = match code {
        KeyCode::Up => Some(b'A'),
        KeyCode::Down => Some(b'B'),
        KeyCode::Right => Some(b'C'),
        KeyCode::Left => Some(b'D'),
        KeyCode::Home => Some(b'H'),
        KeyCode::End => Some(b'F'),
        _ => None,
    };
    if let Some(letter) = cursor_letter {
        return Some(if modifiers.is_empty() {
            vec![0x1b, b'[', letter]
        } else {
            format!(
                "\x1b[1;{}{}",
                xterm_modifier_code(modifiers),
                letter as char
            )
            .into_bytes()
        });
    }

    // Tilde keys (Insert/Delete/PageUp/PageDown): unmodified is `ESC[<n>~`;
    // any modifier switches to `ESC[<n>;<m>~`.
    let tilde_code: Option<u8> = match code {
        KeyCode::Insert => Some(2),
        KeyCode::Delete => Some(3),
        KeyCode::PageUp => Some(5),
        KeyCode::PageDown => Some(6),
        _ => None,
    };
    if let Some(n) = tilde_code {
        return Some(if modifiers.is_empty() {
            format!("\x1b[{n}~").into_bytes()
        } else {
            format!("\x1b[{n};{}~", xterm_modifier_code(modifiers)).into_bytes()
        });
    }

    match code {
        KeyCode::Enter => Some(b"\r".to_vec()),
        KeyCode::Backspace => Some(b"\x7f".to_vec()),
        KeyCode::Tab => Some(b"\t".to_vec()),
        KeyCode::BackTab => Some(b"\x1b[Z".to_vec()),
        KeyCode::Esc => Some(b"\x1b".to_vec()),
        KeyCode::F(n) => Some(
            match n {
                1 => &b"\x1bOP"[..],
                2 => &b"\x1bOQ"[..],
                3 => &b"\x1bOR"[..],
                4 => &b"\x1bOS"[..],
                5 => &b"\x1b[15~"[..],
                6 => &b"\x1b[17~"[..],
                7 => &b"\x1b[18~"[..],
                8 => &b"\x1b[19~"[..],
                9 => &b"\x1b[20~"[..],
                _ => return None,
            }
            .to_vec(),
        ),
        KeyCode::Char(c) => {
            if modifiers.contains(KeyModifiers::CONTROL) {
                match c {
                    'a'..='z' => Some(vec![(c as u8) - b'a' + 1]),
                    _ => None,
                }
            } else if modifiers.contains(KeyModifiers::ALT) {
                // xterm's Alt+key form: ESC followed by the plain key bytes.
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                let mut out = vec![0x1b];
                out.extend_from_slice(s.as_bytes());
                Some(out)
            } else {
                // ponytail: no modifiers -> caller falls back to raw utf8
                // passthrough (handles multi-byte unicode; this function
                // only special-cases the ASCII control/escape keys above).
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod key_to_pty_bytes_tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn unmodified_basics_unchanged() {
        assert_eq!(
            key_to_pty_bytes(KeyCode::Enter, KeyModifiers::NONE),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::Backspace, KeyModifiers::NONE),
            Some(b"\x7f".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::Tab, KeyModifiers::NONE),
            Some(b"\t".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::Esc, KeyModifiers::NONE),
            Some(b"\x1b".to_vec())
        );
    }

    #[test]
    fn back_tab_sends_csi_z() {
        assert_eq!(
            key_to_pty_bytes(KeyCode::BackTab, KeyModifiers::NONE),
            Some(b"\x1b[Z".to_vec())
        );
    }

    #[test]
    fn unmodified_arrows_unchanged() {
        assert_eq!(
            key_to_pty_bytes(KeyCode::Left, KeyModifiers::NONE),
            Some(b"\x1b[D".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::Right, KeyModifiers::NONE),
            Some(b"\x1b[C".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::Up, KeyModifiers::NONE),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::Down, KeyModifiers::NONE),
            Some(b"\x1b[B".to_vec())
        );
    }

    #[test]
    fn modified_arrows_use_extended_xterm_form() {
        assert_eq!(
            key_to_pty_bytes(KeyCode::Left, KeyModifiers::SHIFT),
            Some(b"\x1b[1;2D".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::Right, KeyModifiers::CONTROL),
            Some(b"\x1b[1;5C".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::Up, KeyModifiers::ALT),
            Some(b"\x1b[1;3A".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::Down, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
            Some(b"\x1b[1;6B".to_vec())
        );
    }

    #[test]
    fn unmodified_home_end_unchanged() {
        assert_eq!(
            key_to_pty_bytes(KeyCode::Home, KeyModifiers::NONE),
            Some(b"\x1b[H".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::End, KeyModifiers::NONE),
            Some(b"\x1b[F".to_vec())
        );
    }

    #[test]
    fn modified_home_end_use_extended_xterm_form() {
        assert_eq!(
            key_to_pty_bytes(KeyCode::Home, KeyModifiers::CONTROL),
            Some(b"\x1b[1;5H".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::End, KeyModifiers::SHIFT),
            Some(b"\x1b[1;2F".to_vec())
        );
    }

    #[test]
    fn unmodified_pgup_pgdn_delete_insert_unchanged() {
        assert_eq!(
            key_to_pty_bytes(KeyCode::PageUp, KeyModifiers::NONE),
            Some(b"\x1b[5~".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::PageDown, KeyModifiers::NONE),
            Some(b"\x1b[6~".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::Delete, KeyModifiers::NONE),
            Some(b"\x1b[3~".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::Insert, KeyModifiers::NONE),
            Some(b"\x1b[2~".to_vec())
        );
    }

    #[test]
    fn modified_tilde_keys_use_extended_xterm_form() {
        assert_eq!(
            key_to_pty_bytes(KeyCode::PageUp, KeyModifiers::CONTROL),
            Some(b"\x1b[5;5~".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::Delete, KeyModifiers::SHIFT),
            Some(b"\x1b[3;2~".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::Insert, KeyModifiers::ALT),
            Some(b"\x1b[2;3~".to_vec())
        );
    }

    #[test]
    fn function_keys_unchanged() {
        assert_eq!(
            key_to_pty_bytes(KeyCode::F(1), KeyModifiers::NONE),
            Some(b"\x1bOP".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::F(5), KeyModifiers::NONE),
            Some(b"\x1b[15~".to_vec())
        );
        assert_eq!(key_to_pty_bytes(KeyCode::F(10), KeyModifiers::NONE), None);
    }

    #[test]
    fn ctrl_letter_still_maps_to_control_byte() {
        assert_eq!(
            key_to_pty_bytes(KeyCode::Char('a'), KeyModifiers::CONTROL),
            Some(vec![1])
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(vec![3])
        );
        assert_eq!(
            key_to_pty_bytes(KeyCode::Char('z'), KeyModifiers::CONTROL),
            Some(vec![26])
        );
    }

    #[test]
    fn ctrl_non_letter_char_returns_none() {
        assert_eq!(
            key_to_pty_bytes(KeyCode::Char('1'), KeyModifiers::CONTROL),
            None
        );
    }

    #[test]
    fn alt_char_prefixes_esc() {
        assert_eq!(
            key_to_pty_bytes(KeyCode::Char('b'), KeyModifiers::ALT),
            Some(b"\x1bb".to_vec())
        );
    }

    #[test]
    fn plain_char_returns_none_for_callers_raw_passthrough_fallback() {
        assert_eq!(
            key_to_pty_bytes(KeyCode::Char('x'), KeyModifiers::NONE),
            None
        );
    }
}
