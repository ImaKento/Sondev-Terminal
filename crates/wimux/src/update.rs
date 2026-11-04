//! アップデートの確認と、ワンクリックでの更新。
//!
//! - 起動時（とその後 12 時間ごと）に GitHub Releases の最新版を確かめる
//! - 新しければサイドバーにお知らせを出し、［更新］でインストーラーをダウンロードして
//!   画面を出さずに上書きインストールする（Inno Setup の `/SILENT`）。終わると新しい版が起動する
//!   （`installer/wimux.iss` の `/relaunch=1`）
//!
//! HTTP には Windows 10 以降に入っている `curl.exe` を使う（依存を増やさないため）。
//! 開発ビルド（debug）では確認しない（Cargo.toml の版が古いままなので、いつも「新しい版がある」になる）。

use std::path::PathBuf;
use std::process::Command;

/// リリースを置いている GitHub のリポジトリ
const REPO: &str = "ImaKento/Sondev-Terminal";

/// 今動いているアプリの版
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// "0.3.0"
    pub version: String,
    /// リリースのページ（変更点）
    pub page: String,
    /// インストーラーのダウンロード URL
    pub installer: Option<String>,
}

/// 自動で確認するか（開発ビルドと `WIMUX_UPDATE_CHECK=0` では確認しない。`=1` なら開発ビルドでも確認する）
pub fn auto_check_enabled() -> bool {
    match std::env::var("WIMUX_UPDATE_CHECK").as_deref() {
        Ok("0") => false,
        // 開発ビルドでも試したいとき
        Ok("1") => true,
        _ => !cfg!(debug_assertions),
    }
}

/// ［更新］でインストールまでできるか（Windows のインストーラーだけ）
pub const CAN_INSTALL: bool = cfg!(windows);

/// 最新のリリースが今の版より新しければ返す（通信できない・リリースが無いときは None）
pub fn check() -> Option<Release> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let out = curl()
        .args(["-sSfL", "--max-time", "15"])
        .args(["-H", "Accept: application/vnd.github+json"])
        .args(["-H", "User-Agent: SondevTerminal"])
        .arg(&url)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let release = parse_release(&String::from_utf8_lossy(&out.stdout))?;
    is_newer(&release.version, CURRENT).then_some(release)
}

fn parse_release(json: &str) -> Option<Release> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    if v.get("draft").and_then(|d| d.as_bool()) == Some(true) {
        return None;
    }
    let tag = v.get("tag_name")?.as_str()?;
    let version = tag.trim_start_matches('v').to_string();
    parse_version(&version)?;
    let page = v
        .get("html_url")
        .and_then(|u| u.as_str())
        .unwrap_or_default()
        .to_string();
    let installer = v
        .get("assets")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
        .find(|a| {
            a.get("name")
                .and_then(|n| n.as_str())
                .is_some_and(|n| n.starts_with("SondevTerminal-setup-") && n.ends_with(".exe"))
        })
        .and_then(|a| a.get("browser_download_url")?.as_str())
        .map(str::to_string);
    Some(Release {
        version,
        page,
        installer,
    })
}

/// "1.2.3"（"-beta.1" などは無視）→ (1, 2, 3)
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    let major = it.next()??;
    let minor = it.next().flatten().unwrap_or(0);
    let patch = it.next().flatten().unwrap_or(0);
    Some((major, minor, patch))
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// インストーラーをダウンロードして、画面を出さずに上書きインストールを始める。
/// 始められたら Ok。このあとアプリは自分で終了する（インストーラーが新しい版を起動する）
pub fn download_and_install(release: &Release) -> Result<(), String> {
    let url = release
        .installer
        .as_deref()
        .ok_or("このリリースにはインストーラーがありません")?;
    let path: PathBuf =
        std::env::temp_dir().join(format!("SondevTerminal-setup-{}.exe", release.version));
    let status = curl()
        .args(["-sSfL", "--max-time", "600", "-o"])
        .arg(&path)
        .arg(url)
        .status()
        .map_err(|e| format!("ダウンロードできませんでした: {e}"))?;
    if !status.success() || !path.is_file() {
        return Err("ダウンロードできませんでした".into());
    }
    // /SILENT: 進み具合だけを出してインストール。/relaunch=1: 終わったら新しい版を起動
    let mut cmd = Command::new(&path);
    cmd.args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/relaunch=1"]);
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("インストーラーを起動できませんでした: {e}"))
}

/// 黒い窓を出さずに curl を動かす
fn curl() -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = Command::new("curl.exe");
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
    #[cfg(not(windows))]
    {
        Command::new("curl")
    }
}

#[cfg(test)]
mod tests {
    use super::{is_newer, parse_release, parse_version};

    #[test]
    fn versions() {
        assert_eq!(parse_version("0.2.10"), Some((0, 2, 10)));
        assert_eq!(parse_version("1.0.0-beta.1"), Some((1, 0, 0)));
        assert_eq!(parse_version("x"), None);
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn release_json() {
        let json = r#"{
            "tag_name": "v0.3.0", "draft": false,
            "html_url": "https://github.com/ImaKento/Sondev-Terminal/releases/tag/v0.3.0",
            "assets": [
                {"name": "THIRD_PARTY_LICENSES.html", "browser_download_url": "https://x/l.html"},
                {"name": "SondevTerminal-setup-0.3.0.exe", "browser_download_url": "https://x/s.exe"}
            ]
        }"#;
        let r = parse_release(json).unwrap();
        assert_eq!(r.version, "0.3.0");
        assert_eq!(r.installer.as_deref(), Some("https://x/s.exe"));
        assert!(parse_release(r#"{"tag_name":"v1.0.0","draft":true}"#).is_none());
    }
}
