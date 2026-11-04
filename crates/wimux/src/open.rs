//! URL・ファイルを外のアプリで開く（ターミナルの Ctrl+クリック）。
//!
//! - URL → 既定のブラウザ
//! - ファイル → 設定のエディタ（`editor`）。未設定なら VS Code（`code -g file:line:col`）、
//!   なければ Cursor、どれもなければ既定のアプリ
//! - フォルダ → エクスプローラー

use std::path::{Path, PathBuf};
use std::process::Command;

/// URL を既定のブラウザで開く
pub fn open_url(url: &str) {
    let url = url.to_string();
    std::thread::spawn(move || shell_open(&url));
}

/// ファイル・フォルダを開く。`editor` は設定のエディタのコマンド（`{file}` `{line}` `{col}` を置き換える）
pub fn open_path(path: &Path, line: Option<u32>, col: Option<u32>, editor: Option<&str>) {
    let path = path.to_path_buf();
    let editor = editor.map(str::to_string);
    std::thread::spawn(move || {
        if path.is_dir() {
            shell_open(&path.to_string_lossy());
            return;
        }
        if let Some(cmd) = editor_command(&path, line, col, editor.as_deref()) {
            match spawn(cmd) {
                Ok(()) => return,
                Err(e) => log::warn!("エディタを起動できませんでした: {e}"),
            }
        }
        shell_open(&path.to_string_lossy());
    });
}

/// エディタを起動するコマンド（プログラム, 引数）
fn editor_command(
    path: &Path,
    line: Option<u32>,
    col: Option<u32>,
    template: Option<&str>,
) -> Option<(PathBuf, Vec<String>)> {
    let file = path.to_string_lossy().to_string();
    let line_s = line.unwrap_or(1).to_string();
    let col_s = col.unwrap_or(1).to_string();
    if let Some(t) = template.map(str::trim).filter(|t| !t.is_empty()) {
        let mut parts = split_command(t);
        if parts.is_empty() {
            return None;
        }
        let program = parts.remove(0);
        let has_file = parts.iter().any(|a| a.contains("{file}"));
        let mut args: Vec<String> = parts
            .iter()
            .map(|a| {
                a.replace("{file}", &file)
                    .replace("{line}", &line_s)
                    .replace("{col}", &col_s)
            })
            .collect();
        if !has_file {
            args.push(file);
        }
        return Some((find_program(&program)?, args));
    }
    // VS Code 系（-g file:line:col で行に移動できる）
    for name in ["code", "cursor", "windsurf", "code-insiders"] {
        if let Some(program) = find_program(name) {
            let target = match line {
                Some(_) => format!("{file}:{line_s}:{col_s}"),
                None => file.clone(),
            };
            return Some((program, vec!["-g".into(), target]));
        }
    }
    None
}

fn spawn((program, args): (PathBuf, Vec<String>)) -> std::io::Result<()> {
    let mut cmd = Command::new(&program);
    cmd.args(&args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // code.cmd などのバッチを動かすときに黒い窓を出さない
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().map(|_| ())
}

/// PATH からプログラムを探す（Windows では .exe / .cmd / .bat も試す）。パスが書いてあればそのまま
fn find_program(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.is_absolute() || name.contains('/') || name.contains('\\') {
        return p.is_file().then(|| p.to_path_buf());
    }
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &[""]
    };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        exts.iter()
            .map(|ext| dir.join(format!("{name}{ext}")))
            .find(|c| c.is_file())
    })
}

/// `"C:\Program Files\x.exe" -n{line} {file}` → ["C:\Program Files\x.exe", "-n{line}", "{file}"]
fn split_command(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut has = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                has = true;
            }
            c if c.is_whitespace() && !quoted => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

/// 既定のアプリで開く（URL ならブラウザ、フォルダならエクスプローラー）
#[cfg(any(windows, wimux_check))]
fn shell_open(target: &str) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let verb = wide("open");
    let file = wide(target);
    // 戻り値が 32 以下なら失敗
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if (r as isize) <= 32 {
        log::warn!("開けませんでした: {target}");
    }
}

#[cfg(not(any(windows, wimux_check)))]
fn shell_open(target: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(e) = Command::new(opener).arg(target).spawn() {
        log::warn!("開けませんでした: {target}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::split_command;

    #[test]
    fn split() {
        assert_eq!(
            split_command(r#""C:\Program Files\np++.exe" -n{line} {file}"#),
            vec![r"C:\Program Files\np++.exe", "-n{line}", "{file}"]
        );
        assert_eq!(
            split_command("code -g {file}:{line}"),
            vec!["code", "-g", "{file}:{line}"]
        );
    }
}
