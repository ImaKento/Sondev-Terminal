//! キー入力 → PTY に送るバイト列。
//!
//! 修飾キーなしの普通の文字はここでは扱わず `None` を返す。
//! その場合 GPUI が IME（入力ハンドラ）に回すので、日本語入力と英字入力が同じ経路になる。

use gpui_kit::Keystroke;

/// `app_cursor`: DECCKM（アプリケーションカーソルモード）が有効か。vim / less などが有効にする。
pub fn to_esc(ks: &Keystroke, app_cursor: bool) -> Option<Vec<u8>> {
    let m = ks.modifiers;
    // xterm の修飾キー番号: 1 + shift(1) + alt(2) + ctrl(4)
    let modnum = 1 + m.shift as u8 + (m.alt as u8) * 2 + (m.control as u8) * 4;
    let has_mod = modnum > 1;

    let csi = |final_byte: &str| -> Vec<u8> {
        if has_mod {
            format!("\x1b[1;{modnum}{final_byte}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{final_byte}").into_bytes()
        } else {
            format!("\x1b[{final_byte}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if has_mod {
            format!("\x1b[{n};{modnum}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };

    let bytes = match ks.key.as_str() {
        "enter" => {
            if m.alt {
                b"\x1b\r".to_vec()
            } else if m.control || m.shift {
                // Ctrl/Shift+Enter → LF（Claude Code などでは「改行」扱い）
                b"\n".to_vec()
            } else {
                b"\r".to_vec()
            }
        }
        "tab" => {
            if m.shift {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            }
        }
        "backspace" => {
            if m.control {
                vec![0x08]
            } else if m.alt {
                b"\x1b\x7f".to_vec()
            } else {
                vec![0x7f]
            }
        }
        "escape" => vec![0x1b],
        "up" => csi("A"),
        "down" => csi("B"),
        "right" => csi("C"),
        "left" => csi("D"),
        "home" => csi("H"),
        "end" => csi("F"),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => fkey_ss3("P", has_mod, modnum),
        "f2" => fkey_ss3("Q", has_mod, modnum),
        "f3" => fkey_ss3("R", has_mod, modnum),
        "f4" => fkey_ss3("S", has_mod, modnum),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "space" if m.control => vec![0x00],
        key if m.control && !m.alt => return ctrl_key(key),
        key if m.alt && !m.control => {
            // Alt+文字 → ESC を前置（Meta 扱い）
            let ch = ks.key_char.as_deref().unwrap_or(key);
            if ch.chars().count() != 1 {
                return None;
            }
            let mut v = vec![0x1b];
            v.extend_from_slice(ch.as_bytes());
            v
        }
        _ => return None,
    };
    Some(bytes)
}

fn fkey_ss3(final_byte: &str, has_mod: bool, modnum: u8) -> Vec<u8> {
    if has_mod {
        format!("\x1b[1;{modnum}{final_byte}").into_bytes()
    } else {
        format!("\x1bO{final_byte}").into_bytes()
    }
}

/// Ctrl+文字 → 制御コード。
fn ctrl_key(key: &str) -> Option<Vec<u8>> {
    let mut chars = key.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let code = match c.to_ascii_lowercase() {
        c @ 'a'..='z' => c as u8 - b'a' + 1,
        '@' | '2' => 0x00,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '7' => 0x1f,
        '/' => 0x1f,
        '8' | '?' => 0x7f,
        _ => return None,
    };
    Some(vec![code])
}

// ---------------------------------------------------------------------------
// win32-input-mode
//
// ConPTY は起動時に `CSI ? 9001 h` を送り、「キーを Win32 の KEY_EVENT 形式で送ってほしい」と
// 要求してくる。この形式で送ると、Ctrl+Enter や Shift+Enter などの修飾キー付き入力が
// そのまま Windows のアプリ（Claude Code など）に届く。Windows Terminal / WezTerm と同じ方式。
//
// 形式: ESC [ Vk ; Sc ; Uc ; Kd ; Cs ; Rc _
//   Vk=仮想キーコード Sc=スキャンコード Uc=Unicode文字 Kd=1押下/0解放 Cs=修飾キー状態 Rc=リピート数
// ---------------------------------------------------------------------------

const SHIFT_PRESSED: u32 = 0x0010;
const LEFT_CTRL_PRESSED: u32 = 0x0008;
const LEFT_ALT_PRESSED: u32 = 0x0002;
const ENHANCED_KEY: u32 = 0x0100;

/// 修飾キー付きの特殊キーを win32-input-mode 形式（押下＋解放）に変換する。
/// 対応しないキー（普通の文字など）は `None`。その場合は従来どおりの VT 形式 / IME で送る。
pub fn to_win32_input(ks: &Keystroke) -> Option<Vec<u8>> {
    let m = ks.modifiers;
    // (仮想キーコード, Unicode 文字, 拡張キーか)
    let (vk, uc, enhanced): (u32, u32, bool) = match ks.key.as_str() {
        "enter" => (0x0D, if m.control { 0x0A } else { 0x0D }, false),
        "tab" => (0x09, 0x09, false),
        "backspace" => (0x08, if m.control { 0x7F } else { 0x08 }, false),
        "escape" => (0x1B, 0x1B, false),
        "space" => (0x20, if m.control { 0 } else { 0x20 }, false),
        "pageup" => (0x21, 0, true),
        "pagedown" => (0x22, 0, true),
        "end" => (0x23, 0, true),
        "home" => (0x24, 0, true),
        "left" => (0x25, 0, true),
        "up" => (0x26, 0, true),
        "right" => (0x27, 0, true),
        "down" => (0x28, 0, true),
        "insert" => (0x2D, 0, true),
        "delete" => (0x2E, 0, true),
        f if f.len() >= 2
            && f.starts_with('f')
            && f[1..].parse::<u32>().is_ok_and(|n| (1..=12).contains(&n)) =>
        {
            (0x6F + f[1..].parse::<u32>().unwrap(), 0, false)
        }
        key => {
            // 英字・数字は Ctrl / Alt 付きのときだけ（普通の入力は IME 経由）
            if !(m.control || m.alt) {
                return None;
            }
            let mut chars = key.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            match c.to_ascii_lowercase() {
                c @ 'a'..='z' => {
                    let vk = c.to_ascii_uppercase() as u32;
                    let uc = if m.control {
                        c as u32 - 'a' as u32 + 1
                    } else if m.shift {
                        c.to_ascii_uppercase() as u32
                    } else {
                        c as u32
                    };
                    (vk, uc, false)
                }
                c @ '0'..='9' => (c as u32, if m.control { 0 } else { c as u32 }, false),
                _ => return None,
            }
        }
    };

    let mut cs = 0;
    if m.shift {
        cs |= SHIFT_PRESSED;
    }
    if m.control {
        cs |= LEFT_CTRL_PRESSED;
    }
    if m.alt {
        cs |= LEFT_ALT_PRESSED;
    }
    if enhanced {
        cs |= ENHANCED_KEY;
    }
    let sc = scan_code(vk);
    Some(format!("\x1b[{vk};{sc};{uc};1;{cs};1_\x1b[{vk};{sc};{uc};0;{cs};1_").into_bytes())
}

#[cfg(windows)]
fn scan_code(vk: u32) -> u32 {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MAPVK_VK_TO_VSC, MapVirtualKeyW};
    // SAFETY: 単純な変換関数で、副作用はない
    unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) }
}

#[cfg(not(windows))]
fn scan_code(_vk: u32) -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(s: &'static str) -> Keystroke {
        Keystroke::parse(s).unwrap()
    }

    #[test]
    fn basic_keys() {
        assert_eq!(to_esc(&ks("enter"), false), Some(b"\r".to_vec()));
        assert_eq!(to_esc(&ks("ctrl-c"), false), Some(vec![0x03]));
        assert_eq!(to_esc(&ks("up"), false), Some(b"\x1b[A".to_vec()));
        assert_eq!(to_esc(&ks("up"), true), Some(b"\x1bOA".to_vec()));
        assert_eq!(to_esc(&ks("ctrl-left"), false), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(to_esc(&ks("shift-tab"), false), Some(b"\x1b[Z".to_vec()));
    }

    #[test]
    fn ctrl_enter_is_newline() {
        assert_eq!(to_esc(&ks("ctrl-enter"), false), Some(b"\n".to_vec()));
        assert_eq!(to_esc(&ks("shift-enter"), false), Some(b"\n".to_vec()));
    }

    #[test]
    fn win32_input_mode() {
        assert_eq!(
            to_win32_input(&ks("ctrl-enter")),
            Some(b"\x1b[13;0;10;1;8;1_\x1b[13;0;10;0;8;1_".to_vec())
        );
        assert_eq!(
            to_win32_input(&ks("up")),
            Some(b"\x1b[38;0;0;1;256;1_\x1b[38;0;0;0;256;1_".to_vec())
        );
        assert_eq!(
            to_win32_input(&ks("ctrl-c")),
            Some(b"\x1b[67;0;3;1;8;1_\x1b[67;0;3;0;8;1_".to_vec())
        );
        assert_eq!(to_win32_input(&ks("a")), None);
    }

    #[test]
    fn plain_chars_go_to_ime() {
        assert_eq!(to_esc(&ks("a"), false), None);
        assert_eq!(to_esc(&ks("shift-a"), false), None);
    }
}
