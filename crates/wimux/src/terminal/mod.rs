//! 1 つのペインに対応するターミナル（PTY ＋ alacritty_terminal の状態）。
//!
//! スレッド構成:
//! - reader スレッド: PTY 出力を読み、OSC スキャン → alacritty の VT パーサへ流す
//! - wait スレッド  : 子プロセス終了を待つ（ConPTY は子が死んでも read が EOF にならないことがあるため）
//! - UI スレッド    : `TermMsg` を受け取って再描画・イベント発行（view.rs 側）

pub mod colors;
pub mod keys;
pub mod osc;
pub mod view;

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle, Processor};
use anyhow::{Context as _, Result};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use osc::{OscScanner, ShellEvent};

/// reader / wait スレッドから UI へ送るメッセージ。
#[derive(Debug)]
pub enum TermMsg {
    /// 画面内容が変わった（再描画が必要）
    Wakeup,
    Title(String),
    Bell,
    Shell(ShellEvent),
    Exited,
}

/// alacritty_terminal の `Dimensions` 実装。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.rows as usize
    }
    fn screen_lines(&self) -> usize {
        self.rows as usize
    }
    fn columns(&self) -> usize {
        self.cols as usize
    }
}

type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// alacritty_terminal からのイベントを受け取る。
/// 問い合わせへの応答（`PtyWrite` / 色 / 画面サイズ）は PTY にそのまま書き戻す。
/// - ConPTY は起動直後にカーソル位置問い合わせ (ESC[6n) を送ってくるので、これに答えないと固まる
/// - Claude Code などの TUI は背景色 (OSC 11) や画面サイズ (CSI 14 t) を問い合わせ、
///   応答を待つことがある
#[derive(Clone)]
pub struct EventProxy {
    tx: flume::Sender<TermMsg>,
    writer: SharedWriter,
    window_size: Arc<Mutex<WindowSize>>,
}

impl EventProxy {
    fn reply(&self, text: &str) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(text.as_bytes());
            let _ = w.flush();
        }
    }
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => self.reply(&text),
            Event::ColorRequest(index, format) => {
                // ここは Term をロック中に呼ばれるので、アプリが上書きした色は見ずに既定パレットで答える
                self.reply(&format(colors::default_rgb(index)));
            }
            Event::TextAreaSizeRequest(format) => {
                let size = *self.window_size.lock().unwrap();
                self.reply(&format(size));
            }
            Event::Title(title) => {
                let _ = self.tx.send(TermMsg::Title(title));
            }
            Event::Bell => {
                let _ = self.tx.send(TermMsg::Bell);
            }
            _ => {}
        }
    }
}

/// 起動オプション。Phase 2 以降で復元情報（cwd / conda / claude 再開）をここに足していく。
#[derive(Debug, Clone, Default)]
pub struct SpawnOptions {
    pub cwd: Option<PathBuf>,
    pub pane_id: String,
}

/// UI スレッドと各スレッドで共有する部分。
pub struct Shared {
    pub term: FairMutex<Term<EventProxy>>,
    writer: SharedWriter,
    master: Mutex<Box<dyn MasterPty + Send>>,
    size: Mutex<TermSize>,
    window_size: Arc<Mutex<WindowSize>>,
    /// ConPTY が win32-input-mode (`CSI ? 9001 h`) を要求しているか
    win32_input: AtomicBool,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
}

impl Shared {
    pub fn win32_input(&self) -> bool {
        self.win32_input.load(Ordering::Relaxed)
    }

    pub fn write(&self, bytes: &[u8]) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(bytes);
            let _ = w.flush();
        }
    }

    /// サイズが変わったときだけ Term と PTY をリサイズする。
    /// `cell_px`: セル 1 つの幅・高さ（ピクセル）。画面サイズの問い合わせへの応答に使う。
    pub fn resize(&self, new: TermSize, cell_px: (u16, u16)) {
        let new = TermSize {
            cols: new.cols.max(2),
            rows: new.rows.max(1),
        };
        {
            let mut size = self.size.lock().unwrap();
            if *size == new {
                return;
            }
            *size = new;
        }
        *self.window_size.lock().unwrap() = WindowSize {
            num_lines: new.rows,
            num_cols: new.cols,
            cell_width: cell_px.0,
            cell_height: cell_px.1,
        };
        self.term.lock().resize(new);
        if let Ok(master) = self.master.lock() {
            let _ = master.resize(PtySize {
                rows: new.rows,
                cols: new.cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        if let Ok(mut k) = self.killer.lock() {
            let _ = k.kill();
        }
    }
}

/// 既定のカーソルの形（設定画面）。0 = ブロック, 1 = 縦線, 2 = 下線
static DEFAULT_CURSOR: AtomicU8 = AtomicU8::new(0);

pub fn set_default_cursor(kind: crate::settings::CursorKind) {
    use crate::settings::CursorKind;
    let v = match kind {
        CursorKind::Block => 0,
        CursorKind::Beam => 1,
        CursorKind::Underline => 2,
    };
    DEFAULT_CURSOR.store(v, Ordering::Relaxed);
}

fn term_config() -> TermConfig {
    let shape = match DEFAULT_CURSOR.load(Ordering::Relaxed) {
        1 => CursorShape::Beam,
        2 => CursorShape::Underline,
        _ => CursorShape::Block,
    };
    TermConfig {
        default_cursor_style: CursorStyle {
            shape,
            blinking: false,
        },
        ..TermConfig::default()
    }
}

pub struct Terminal {
    pub shared: Arc<Shared>,
    pub rx: flume::Receiver<TermMsg>,
}

impl Terminal {
    pub fn spawn(opts: SpawnOptions) -> Result<Self> {
        let size = TermSize { cols: 80, rows: 24 };
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("PTY を開けませんでした")?;

        let mut cmd = shell_command();
        let cwd = opts.cwd.clone().or_else(home_dir);
        if let Some(cwd) = &cwd {
            cmd.cwd(cwd);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "wimux");
        cmd.env("WIMUX", "1");
        cmd.env("WIMUX_PANE_ID", &opts.pane_id);
        // 同梱の fzf.exe などを探す場所（exe と同じフォルダ）
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        {
            cmd.env("WIMUX_BIN_DIR", dir);
        }
        // claude 関数が使う、セッション ID を記録するフックの設定
        if let Some(path) = crate::claude_session::write_hook_settings() {
            cmd.env("WIMUX_CLAUDE_SETTINGS", path);
        }

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .context("シェルを起動できませんでした")?;
        // slave は子プロセスに渡したので手放す（持ったままだと EOF が来ない）
        drop(pair.slave);

        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader()?;
        let writer: SharedWriter = Arc::new(Mutex::new(pair.master.take_writer()?));

        let (tx, rx) = flume::unbounded();
        let window_size = Arc::new(Mutex::new(WindowSize {
            num_lines: size.rows,
            num_cols: size.cols,
            cell_width: 8,
            cell_height: 18,
        }));
        let proxy = EventProxy {
            tx: tx.clone(),
            writer: writer.clone(),
            window_size: window_size.clone(),
        };
        let term = Term::new(term_config(), &size, proxy);

        let shared = Arc::new(Shared {
            term: FairMutex::new(term),
            writer,
            master: Mutex::new(pair.master),
            size: Mutex::new(size),
            window_size,
            win32_input: AtomicBool::new(false),
            killer: Mutex::new(killer),
        });

        // reader スレッド
        {
            let shared = Arc::downgrade(&shared);
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("wimux-pty-reader".into())
                .spawn(move || {
                    let mut processor: Processor = Processor::new();
                    let mut scanner = OscScanner::new();
                    let mut events = Vec::new();
                    let mut buf = vec![0u8; 64 * 1024];
                    // デバッグ用: WIMUX_PTY_LOG にファイルパスを指定すると、PTY の生出力を追記する
                    let mut pty_log = std::env::var_os("WIMUX_PTY_LOG").and_then(|path| {
                        std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(path)
                            .ok()
                    });
                    // WIMUX_WIN32_INPUT=0 で win32-input-mode を無効化できる（不具合時の切り分け用）
                    let allow_win32_input =
                        std::env::var("WIMUX_WIN32_INPUT").map_or(true, |v| v != "0");
                    let mut mode_tail: Vec<u8> = Vec::with_capacity(64 * 1024 + 16);
                    loop {
                        let n = match reader.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => n,
                        };
                        let Some(shared) = shared.upgrade() else {
                            break;
                        };
                        let bytes = &buf[..n];
                        if let Some(log) = pty_log.as_mut() {
                            let _ = log.write_all(bytes);
                        }
                        // win32-input-mode の ON/OFF を検出（チャンク境界をまたぐ場合に備えて末尾を保持）
                        if allow_win32_input {
                            mode_tail.extend_from_slice(bytes);
                            if let Some(on) = last_win32_mode_switch(&mode_tail) {
                                shared.win32_input.store(on, Ordering::Relaxed);
                            }
                            let keep = mode_tail.len().saturating_sub(16);
                            mode_tail.drain(..keep);
                        }
                        scanner.feed(bytes, &mut events);
                        {
                            let mut term = shared.term.lock();
                            processor.advance(&mut *term, bytes);
                        }
                        for ev in events.drain(..) {
                            let _ = tx.send(TermMsg::Shell(ev));
                        }
                        // UI 側でまとめて処理するので、溜まっていれば送らない
                        if tx.is_empty() {
                            let _ = tx.send(TermMsg::Wakeup);
                        }
                    }
                    let _ = tx.send(TermMsg::Exited);
                })?;
        }

        // wait スレッド
        std::thread::Builder::new()
            .name("wimux-pty-wait".into())
            .spawn(move || {
                let _ = child.wait();
                let _ = tx.send(TermMsg::Exited);
            })?;

        Ok(Self { shared, rx })
    }

    /// 設定（既定のカーソルの形など）を反映する
    pub fn apply_config(&self) {
        self.shared.term.lock().set_options(term_config());
    }
}

/// 使うシェルを決める。pwsh (PowerShell 7) があればそれ、なければ Windows PowerShell。
/// シェル連携スクリプトが見つかれば自動で読み込む。
fn shell_command() -> CommandBuilder {
    #[cfg(windows)]
    {
        let shell = if find_in_path("pwsh.exe").is_some() {
            "pwsh.exe"
        } else {
            "powershell.exe"
        };
        let mut cmd = CommandBuilder::new(shell);
        cmd.arg("-NoLogo");
        if let Some(script) = shell_integration_script() {
            cmd.arg("-NoExit");
            cmd.arg("-Command");
            cmd.arg(format!(". '{}'", script.display()));
        }
        cmd
    }
    #[cfg(not(windows))]
    {
        // 開発用（Linux / macOS でのビルド確認用）
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
        CommandBuilder::new(shell)
    }
}

/// シェル連携スクリプト（exe に埋め込んでおき、配布先でも exe 1 つで動くようにする）
const SHELL_INTEGRATION: &str = include_str!("../../../../shell-integration/wimux.ps1");

/// 埋め込んだシェル連携スクリプトを `%APPDATA%\wimux\shell-integration\wimux.ps1` に書き出して、その場所を返す。
/// 内容が同じなら書き直さない（アップデートで中身が変わったときだけ上書き）
#[cfg_attr(not(windows), allow(dead_code))]
fn shell_integration_script() -> Option<PathBuf> {
    let path = crate::session::config_dir()
        .join("shell-integration")
        .join("wimux.ps1");
    let current = std::fs::read_to_string(&path).ok();
    if current.as_deref() != Some(SHELL_INTEGRATION) {
        if let Err(e) = crate::session::write_atomic(&path, SHELL_INTEGRATION.as_bytes()) {
            log::warn!("シェル連携スクリプトを書き出せませんでした: {e}");
            return None;
        }
    }
    Some(path)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn find_in_path(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(exe))
        .find(|p| p.is_file())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// バイト列中で最後に現れた `CSI ? 9001 h`（ON）/ `CSI ? 9001 l`（OFF）を返す。
fn last_win32_mode_switch(bytes: &[u8]) -> Option<bool> {
    const ON: &[u8] = b"\x1b[?9001h";
    const OFF: &[u8] = b"\x1b[?9001l";
    let on = bytes.windows(ON.len()).rposition(|w| w == ON);
    let off = bytes.windows(OFF.len()).rposition(|w| w == OFF);
    match (on, off) {
        (Some(a), Some(b)) => Some(a > b),
        (Some(_), None) => Some(true),
        (None, Some(_)) => Some(false),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_win32_input_mode() {
        assert_eq!(
            last_win32_mode_switch(b"abc\x1b[?9001h\x1b[?1004h"),
            Some(true)
        );
        assert_eq!(
            last_win32_mode_switch(b"\x1b[?9001h...\x1b[?9001l"),
            Some(false)
        );
        assert_eq!(last_win32_mode_switch(b"plain"), None);
    }
}
