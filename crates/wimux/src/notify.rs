//! デスクトップ通知（Windows のトースト通知）。
//!
//! 追加の依存を増やさないよう、Windows PowerShell 経由で WinRT の通知 API を呼ぶ。
//! 通知の頻度は低いので、起動コスト（数百 ms、別プロセス）は問題にならない。

/// 通知を出す（失敗しても無視する）。呼び出し元をブロックしない。
pub fn toast(title: &str, body: &str) {
    let title = title.to_string();
    let body = body.to_string();
    std::thread::spawn(move || {
        if let Err(e) = show(&title, &body) {
            log::warn!("通知を出せませんでした: {e}");
        }
    });
}

#[cfg(windows)]
fn show(title: &str, body: &str) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // 文字列はスクリプトに埋め込まず環境変数で渡す（引用符などで壊れないように）
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null
$xml = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02)
$texts = $xml.GetElementsByTagName('text')
$texts.Item(0).AppendChild($xml.CreateTextNode($env:WIMUX_TOAST_TITLE)) > $null
$texts.Item(1).AppendChild($xml.CreateTextNode($env:WIMUX_TOAST_BODY)) > $null
$toast = [Windows.UI.Notifications.ToastNotification]::new($xml)
$appId = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($appId).Show($toast)
"#;
    std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            SCRIPT,
        ])
        .env("WIMUX_TOAST_TITLE", title)
        .env("WIMUX_TOAST_BODY", body)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map(|_| ())
}

#[cfg(not(windows))]
fn show(title: &str, body: &str) -> std::io::Result<()> {
    log::info!("[通知] {title}: {body}");
    Ok(())
}
