//! ターミナルの描画・入力・IME・選択を担当する GPUI ビュー。
//!
//! 描画方針:
//! - 行ごとに「同じスタイルが続くセル」をまとめて 1 回で shape する
//! - `force_width` でグリフ幅をセル幅に固定し、フォントによるズレを防ぐ
//! - 全角文字は別バッチにして幅 = セル幅 × 2 に固定する

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::colors::{resolve, to_hsla};
use super::osc::ShellEvent;
use super::{TermMsg, TermSize, Terminal, keys};

/// 親（ウィンドウ）へ通知するイベント。
#[derive(Debug, Clone)]
pub enum TerminalEvent {
    TitleChanged(String),
    Shell(ShellEvent),
    Bell,
    Exited,
    /// このペインにフォーカスが移った
    Focused,
}

#[derive(Debug, Clone, Copy)]
struct CellMetrics {
    width: Pixels,
    height: Pixels,
}

pub struct TerminalView {
    terminal: Terminal,
    focus_handle: FocusHandle,
    font: Font,
    font_size: Pixels,
    /// IME で変換中の文字列（未確定）
    marked_text: Option<String>,
    /// IME 候補ウィンドウを出す位置（= カーソルのセル）
    ime_anchor: Rc<Cell<Option<Bounds<Pixels>>>>,
    metrics: Rc<Cell<Option<CellMetrics>>>,
    /// 端末グリッドを描いている領域（マウス座標 → セル位置の変換に使う）
    content_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    scroll_accum: f32,
    selecting: bool,
    exited: bool,
    /// プロンプトが出るたびに 1 つずつ送るコマンド（復元時の conda activate / claude --continue など）
    pending_commands: VecDeque<String>,
    /// Claude Code が動いているらしい（タイトルで判定。プロンプトが出たら解除）
    claude_ui: bool,
    _io_task: Task<()>,
    _focus_sub: Subscription,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TerminalView {
    pub fn new(terminal: Terminal, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let rx = terminal.rx.clone();
        let io_task = cx.spawn(async move |this, cx| {
            while let Ok(first) = rx.recv_async().await {
                let mut msgs = vec![first];
                msgs.extend(rx.try_iter());
                let alive = this
                    .update(cx, |this, cx| {
                        for msg in msgs {
                            this.handle_msg(msg, cx);
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
                // 大量出力時に毎バイト再描画しないよう、少しだけまとめる（~120fps 相当）
                cx.background_executor()
                    .timer(Duration::from_millis(8))
                    .await;
            }
        });

        let focus_handle = cx.focus_handle();
        let focus_sub = cx.on_focus(&focus_handle, window, |_, _, cx| {
            cx.emit(TerminalEvent::Focused);
        });

        Self {
            terminal,
            focus_handle,
            font: pick_font(cx, None),
            font_size: px(crate::theme::DEFAULT_FONT_SIZE),
            marked_text: None,
            ime_anchor: Rc::new(Cell::new(None)),
            metrics: Rc::new(Cell::new(None)),
            content_bounds: Rc::new(Cell::new(None)),
            scroll_accum: 0.,
            selecting: false,
            exited: false,
            pending_commands: VecDeque::new(),
            claude_ui: false,
            _io_task: io_task,
            _focus_sub: focus_sub,
        }
    }

    /// 次のプロンプトから順に実行するコマンドを積む。
    pub fn queue_commands(&mut self, commands: impl IntoIterator<Item = String>) {
        self.pending_commands.extend(commands);
    }

    /// 設定（カーソルの形など）を反映する
    pub fn apply_config(&self) {
        self.terminal.apply_config();
    }

    /// フォントを変える（設定画面）。None なら自動
    pub fn set_font_family(&mut self, family: Option<&str>, cx: &App) {
        self.font = pick_font(cx, family);
    }

    /// 文字サイズを変える（Ctrl+= / Ctrl+-）
    pub fn set_font_size(&mut self, size: f32) {
        self.font_size = px(size);
    }

    /// シェルに文字列を打ち込む（cdi で選んだフォルダへの移動など）
    pub fn send_text(&mut self, text: &str) {
        self.write(text.as_bytes());
    }

    fn handle_msg(&mut self, msg: TermMsg, cx: &mut Context<Self>) {
        match msg {
            TermMsg::Wakeup => {}
            TermMsg::Title(t) => {
                let first = t.chars().next().unwrap_or(' ');
                self.claude_ui = t.contains("Claude")
                    || first == '\u{2733}'
                    || ('\u{2800}'..='\u{28FF}').contains(&first);
                cx.emit(TerminalEvent::TitleChanged(t))
            }
            TermMsg::Bell => cx.emit(TerminalEvent::Bell),
            TermMsg::Shell(ev) => {
                if matches!(ev, ShellEvent::Cwd(_)) {
                    self.claude_ui = false;
                }
                // cwd の通知 = プロンプトが表示された合図。積んであるコマンドを 1 つ流す
                if matches!(ev, ShellEvent::Cwd(_))
                    && let Some(cmd) = self.pending_commands.pop_front()
                {
                    let mut line = cmd.into_bytes();
                    line.push(b'\r');
                    self.terminal.shared.write(&line);
                }
                cx.emit(TerminalEvent::Shell(ev));
            }
            TermMsg::Exited => {
                if !self.exited {
                    self.exited = true;
                    cx.emit(TerminalEvent::Exited);
                }
            }
        }
    }

    /// 入力を PTY に送る。スクロールバック表示中なら最下部に戻し、選択は解除する。
    fn write(&mut self, bytes: &[u8]) {
        if self.exited {
            return;
        }
        {
            let mut term = self.terminal.shared.term.lock();
            if term.grid().display_offset() != 0 {
                term.scroll_display(Scroll::Bottom);
            }
            term.selection = None;
        }
        self.terminal.shared.write(bytes);
    }

    fn mode(&self) -> TermMode {
        *self.terminal.shared.term.lock().mode()
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        // 改行は CR に揃える（Windows の CRLF も 1 回の Enter にする）
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.mode().contains(TermMode::BRACKETED_PASTE) {
            let mut bytes = b"\x1b[200~".to_vec();
            bytes.extend_from_slice(text.replace('\x1b', "").as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
            self.write(&bytes);
        } else {
            self.write(text.as_bytes());
        }
    }

    /// 選択中の文字列をクリップボードへ。コピーしたら true。
    fn copy_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let text = {
            let mut term = self.terminal.shared.term.lock();
            let text = term.selection_to_string().filter(|s| !s.is_empty());
            term.selection = None;
            text
        };
        match text {
            Some(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                cx.notify();
                true
            }
            None => false,
        }
    }

    fn has_selection(&self) -> bool {
        self.terminal
            .shared
            .term
            .lock()
            .selection
            .as_ref()
            .is_some_and(|s| !s.is_empty())
    }

    /// ウィンドウ座標 → グリッド上の位置
    fn grid_point(&self, pos: Point<Pixels>) -> Option<(GridPoint, Side)> {
        let bounds = self.content_bounds.get()?;
        let metrics = self.metrics.get()?;
        let term = self.terminal.shared.term.lock();
        let cols = term.columns();
        let rows = term.screen_lines();
        let offset = term.grid().display_offset() as i32;

        let x = f32::from(pos.x - bounds.origin.x) / f32::from(metrics.width);
        let y = f32::from(pos.y - bounds.origin.y) / f32::from(metrics.height);
        let col = (x.floor().max(0.) as usize).min(cols.saturating_sub(1));
        let row = (y.floor().max(0.) as i32).min(rows as i32 - 1);
        let side = if x - x.floor() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        Some((GridPoint::new(Line(row - offset), Column(col)), side))
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        // IME 変換中のキーは IME に任せる
        if self.marked_text.is_some() {
            return;
        }
        let ks = &ev.keystroke;
        let m = ks.modifiers;
        // Ctrl+C: 文字を選択しているときはコピー、していなければ通常どおり ^C（中断）を送る
        if m.control && !m.shift && !m.alt && ks.key == "c" && self.copy_selection(cx) {
            cx.stop_propagation();
            return;
        }
        // Ctrl+V: クリップボードに文字があれば貼り付け（Windows Terminal と同じ）。
        // 画像だけのときはキーをそのまま送る（アプリ側の画像貼り付け用）
        if m.control && !m.shift && !m.alt && ks.key == "v" {
            let has_text = cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .is_some_and(|t| !t.is_empty());
            if has_text {
                self.paste(cx);
                cx.stop_propagation();
                return;
            }
            // 文字がない（スクショなど画像だけ）とき、Claude Code には画像貼り付けのキー
            // Alt+V を送る（Windows の Claude Code は Alt+V で画像を読み込む）
            if self.claude_ui
                && let Ok(alt_v) = Keystroke::parse("alt-v")
            {
                let bytes = if self.terminal.shared.win32_input() {
                    keys::to_win32_input(&alt_v)
                } else {
                    keys::to_esc(&alt_v, false)
                };
                if let Some(bytes) = bytes {
                    self.write(&bytes);
                    cx.stop_propagation();
                    return;
                }
            }
        }
        if m.control && m.shift && !m.alt {
            match ks.key.as_str() {
                "v" => {
                    self.paste(cx);
                    cx.stop_propagation();
                    return;
                }
                "c" => {
                    self.copy_selection(cx);
                    cx.stop_propagation();
                    return;
                }
                _ => {}
            }
        }
        // AltGr などで文字入力を優先すべきキーは IME 経由にする
        if ev.prefer_character_input {
            return;
        }
        let app_cursor = self.mode().contains(TermMode::APP_CURSOR);
        if let Some(bytes) = keys::to_esc(ks, app_cursor) {
            // ConPTY が win32-input-mode を要求していれば、修飾キー情報ごと送る
            let bytes = if self.terminal.shared.win32_input() {
                keys::to_win32_input(ks).unwrap_or(bytes)
            } else {
                bytes
            };
            self.write(&bytes);
            cx.stop_propagation();
        }
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(metrics) = self.metrics.get() else {
            return;
        };
        let dy = ev.delta.pixel_delta(metrics.height).y;
        self.scroll_accum += f32::from(dy) / f32::from(metrics.height);
        let lines = self.scroll_accum.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.scroll_accum -= lines as f32;

        let mode = self.mode();
        if mode.intersects(TermMode::MOUSE_MODE) && mode.contains(TermMode::SGR_MOUSE) {
            // アプリがマウスを使っている（Claude Code の全画面表示など）→ ホイールをそのまま送る
            let (col, row) = self
                .grid_point(ev.position)
                .map(|(p, _)| {
                    let offset = self.terminal.shared.term.lock().grid().display_offset() as i32;
                    (p.column.0 + 1, (p.line.0 + offset + 1).max(1) as usize)
                })
                .unwrap_or((1, 1));
            let button = if lines > 0 { 64 } else { 65 };
            let seq = format!("\x1b[<{button};{col};{row}M");
            let bytes = seq.repeat(lines.unsigned_abs() as usize);
            self.terminal.shared.write(bytes.as_bytes());
        } else if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
            // vim / less などの全画面アプリには矢印キーとして送る
            let key: &[u8] = if lines > 0 { b"\x1b[A" } else { b"\x1b[B" };
            let bytes = key.repeat(lines.unsigned_abs() as usize);
            self.terminal.shared.write(&bytes);
        } else {
            self.terminal
                .shared
                .term
                .lock()
                .scroll_display(Scroll::Delta(lines));
        }
        cx.notify();
    }

    fn on_left_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        let Some((point, side)) = self.grid_point(ev.position) else {
            return;
        };
        let ty = match ev.click_count {
            2 => SelectionType::Semantic,
            3 => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        self.terminal.shared.term.lock().selection = Some(Selection::new(ty, point, side));
        self.selecting = true;
        cx.notify();
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || ev.pressed_button != Some(MouseButton::Left) {
            return;
        }
        if let Some((point, side)) = self.grid_point(ev.position) {
            if let Some(sel) = self.terminal.shared.term.lock().selection.as_mut() {
                sel.update(point, side);
            }
            cx.notify();
        }
    }

    fn on_left_up(&mut self, _ev: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        self.selecting = false;
        let mut term = self.terminal.shared.term.lock();
        if term.selection.as_ref().is_some_and(|s| s.is_empty()) {
            term.selection = None;
        }
        cx.notify();
    }

    /// Windows Terminal と同じ: 選択があればコピー、なければ貼り付け
    fn on_right_down(&mut self, _ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        if self.has_selection() {
            self.copy_selection(cx);
        } else {
            self.paste(cx);
        }
    }
}

/// 等幅フォントの候補（自動選択では上から順に、インストールされている最初のものを使う）。
/// 等幅でないフォントだとセル位置がずれるので、等幅フォントだけを並べる。
/// 設定画面のフォント一覧にも、このうちインストールされているものが出る。
pub const FONT_CANDIDATES: &[&str] = &[
    "JetBrains Mono", // Ghostty / cmux の既定
    "Cascadia Mono",
    "Cascadia Code",
    "Consolas",
    "HackGen Console NF",
    "HackGen Console",
    "PlemolJP Console",
    "UDEV Gothic",
    "UDEV Gothic NF",
    "Firge",
    "Source Han Code JP",
    "BIZ UDGothic",
    "MS Gothic",
    "Fira Code",
    "Hack",
    "Source Code Pro",
    "DejaVu Sans Mono", // Linux での開発確認用
    "Noto Sans Mono",
];

/// 日本語などメインフォントにない文字のフォールバック先。
const FALLBACK_FONTS: &[&str] = &[
    "BIZ UDGothic",
    "MS Gothic",
    "Yu Gothic",
    "Noto Sans CJK JP",
    "Segoe UI Emoji",
];

/// 候補のうちインストールされているフォント
pub fn installed_fonts(cx: &App) -> Vec<&'static str> {
    let installed = cx.text_system().all_font_names();
    FONT_CANDIDATES
        .iter()
        .copied()
        .filter(|name| installed.iter().any(|f| f == name))
        .collect()
}

/// `preferred` がインストールされていればそれ、なければ候補の先頭から自動で選ぶ
fn pick_font(cx: &App, preferred: Option<&str>) -> Font {
    let installed = cx.text_system().all_font_names();
    let is_installed = |name: &str| installed.iter().any(|f| f == name);
    let family: String = preferred
        .filter(|p| is_installed(p))
        .map(str::to_string)
        .or_else(|| {
            FONT_CANDIDATES
                .iter()
                .find(|name| is_installed(name))
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "Consolas".into());
    let mut f = font(family.clone());
    f.fallbacks = Some(FontFallbacks::from_fonts(
        FALLBACK_FONTS
            .iter()
            .filter(|n| **n != family)
            .map(|n| n.to_string())
            .collect(),
    ));
    f
}

// ---------------------------------------------------------------------------
// 描画
// ---------------------------------------------------------------------------

struct TextBatch {
    origin: Point<Pixels>,
    line: ShapedLine,
}

struct Prepared {
    metrics: CellMetrics,
    background: Hsla,
    rects: Vec<(Bounds<Pixels>, Hsla)>,
    texts: Vec<TextBatch>,
    cursor: Option<(Bounds<Pixels>, Hsla, CursorShape)>,
    marked: Option<(Point<Pixels>, ShapedLine)>,
}

/// 1 行分の描画バッチを組み立てるための作業用構造体。
struct BatchBuilder {
    start_col: usize,
    text: String,
    fg: Hsla,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    wide: bool,
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shared = self.terminal.shared.clone();
        let font = self.font.clone();
        let font_size = self.font_size;
        let marked_text = self.marked_text.clone();
        let metrics_cell = self.metrics.clone();
        let content_bounds = self.content_bounds.clone();
        let ime_anchor = self.ime_anchor.clone();
        let focus_handle = self.focus_handle.clone();
        let entity = cx.entity();
        let focused = self.focus_handle.is_focused(window);
        let palette = super::colors::current();
        let selection_color = crate::theme::th().selection;
        let background = to_hsla(palette.background);
        // 透過中は背景を塗らない（区画の枠が透明度つきで塗る）
        let paint_bg = !crate::theme::translucent();

        let terminal_canvas = canvas(
            move |bounds, window, _cx| {
                content_bounds.set(Some(bounds));
                let text_system = window.text_system().clone();
                let font_id = text_system.resolve_font(&font);
                let cell_width = text_system
                    .advance(font_id, font_size, 'm')
                    .map(|s| s.width)
                    .unwrap_or(font_size * 0.6);
                let cell_height = (font_size * crate::theme::line_height()).round();
                let metrics = CellMetrics {
                    width: cell_width,
                    height: cell_height,
                };
                metrics_cell.set(Some(metrics));

                // 領域サイズからグリッドサイズを決めてリサイズ
                let cols = (f32::from(bounds.size.width) / f32::from(cell_width))
                    .floor()
                    .max(2.) as u16;
                let rows = (f32::from(bounds.size.height) / f32::from(cell_height))
                    .floor()
                    .max(1.) as u16;
                shared.resize(
                    TermSize { cols, rows },
                    (
                        f32::from(cell_width).round() as u16,
                        f32::from(cell_height).round() as u16,
                    ),
                );

                let term = shared.term.lock();
                let content = term.renderable_content();
                let overrides = content.colors;
                let display_offset = content.display_offset as i32;
                let selection = content.selection;
                let default_bg: Rgb =
                    resolve(Color::Named(NamedColor::Background), overrides, &palette);
                let selection_bg = to_hsla(Rgb {
                    r: (selection_color >> 16) as u8,
                    g: (selection_color >> 8) as u8,
                    b: selection_color as u8,
                });

                let mut rects: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
                let mut texts: Vec<TextBatch> = Vec::new();
                let mut batch: Option<BatchBuilder> = None;
                let mut current_row: i32 = i32::MIN;

                let cell_origin = |row: i32, col: usize| -> Point<Pixels> {
                    point(
                        bounds.origin.x + cell_width * col as f32,
                        bounds.origin.y + cell_height * row as f32,
                    )
                };

                let flush =
                    |batch: &mut Option<BatchBuilder>, row: i32, texts: &mut Vec<TextBatch>| {
                        let Some(b) = batch.take() else { return };
                        let trimmed = b.text.trim_end();
                        if trimmed.is_empty() {
                            return;
                        }
                        let mut f = font.clone();
                        if b.bold {
                            f.weight = FontWeight::BOLD;
                        }
                        if b.italic {
                            f.style = FontStyle::Italic;
                        }
                        let run = TextRun {
                            len: trimmed.len(),
                            font: f,
                            color: b.fg,
                            background_color: None,
                            underline: b.underline.then_some(UnderlineStyle {
                                thickness: px(1.),
                                color: Some(b.fg),
                                wavy: false,
                            }),
                            strikethrough: b.strike.then_some(StrikethroughStyle {
                                thickness: px(1.),
                                color: Some(b.fg),
                            }),
                        };
                        let force = if b.wide { cell_width * 2. } else { cell_width };
                        let line = text_system.shape_line(
                            SharedString::from(trimmed.to_string()),
                            font_size,
                            &[run],
                            Some(force),
                        );
                        texts.push(TextBatch {
                            origin: cell_origin(row, b.start_col),
                            line,
                        });
                    };

                for indexed in content.display_iter {
                    let row = indexed.point.line.0 + display_offset;
                    let col = indexed.point.column.0;
                    let cell = indexed.cell;

                    if row != current_row {
                        flush(&mut batch, current_row, &mut texts);
                        current_row = row;
                    }
                    if cell
                        .flags
                        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                    {
                        continue;
                    }

                    let mut fg = resolve(cell.fg, overrides, &palette);
                    let mut bg = resolve(cell.bg, overrides, &palette);
                    if cell.flags.contains(Flags::INVERSE) {
                        std::mem::swap(&mut fg, &mut bg);
                    }
                    let wide = cell.flags.contains(Flags::WIDE_CHAR);
                    let width_cells = if wide { 2 } else { 1 };
                    let selected = selection.is_some_and(|s| s.contains(indexed.point));

                    // 背景（既定色以外 or 選択中）。同じ色が続けば矩形を伸ばす
                    let bg_color = if selected {
                        Some(selection_bg)
                    } else if bg != default_bg {
                        Some(to_hsla(bg))
                    } else {
                        None
                    };
                    if let Some(color) = bg_color {
                        let origin = cell_origin(row, col);
                        let w = cell_width * width_cells as f32;
                        if let Some((last, last_color)) = rects.last_mut()
                            && *last_color == color
                            && last.origin.y == origin.y
                            && (last.origin.x + last.size.width - origin.x).abs() < px(0.5)
                        {
                            last.size.width += w;
                        } else {
                            rects.push((Bounds::new(origin, size(w, cell_height)), color));
                        }
                    }

                    let ch = if cell.flags.contains(Flags::HIDDEN) {
                        ' '
                    } else {
                        cell.c
                    };
                    let fg_hsla = to_hsla(fg);
                    let bold = cell.flags.contains(Flags::BOLD);
                    let italic = cell.flags.contains(Flags::ITALIC);
                    let underline = cell.flags.intersects(Flags::ALL_UNDERLINES);
                    let strike = cell.flags.contains(Flags::STRIKEOUT);

                    let same_style = batch.as_ref().is_some_and(|b| {
                        b.fg == fg_hsla
                            && b.bold == bold
                            && b.italic == italic
                            && b.underline == underline
                            && b.strike == strike
                            && b.wide == wide
                            && !wide // 全角は 1 文字ずつ（位置ズレ防止）
                    });
                    if !same_style {
                        flush(&mut batch, row, &mut texts);
                        if ch == ' ' && !underline {
                            continue;
                        }
                        batch = Some(BatchBuilder {
                            start_col: col,
                            text: String::new(),
                            fg: fg_hsla,
                            bold,
                            italic,
                            underline,
                            strike,
                            wide,
                        });
                    }
                    if let Some(b) = batch.as_mut() {
                        b.text.push(ch);
                        if let Some(zw) = cell.zerowidth() {
                            b.text.extend(zw.iter());
                        }
                    }
                }
                flush(&mut batch, current_row, &mut texts);

                // カーソル
                let cursor_point = content.cursor.point;
                let cursor_row = cursor_point.line.0 + display_offset;
                let cursor_col = cursor_point.column.0;
                let cursor_wide = term.grid()[cursor_point].flags.contains(Flags::WIDE_CHAR);
                let cursor_bounds = Bounds::new(
                    cell_origin(cursor_row, cursor_col),
                    size(cell_width * if cursor_wide { 2. } else { 1. }, cell_height),
                );
                ime_anchor.set(Some(cursor_bounds));
                let cursor_visible = cursor_row >= 0
                    && (cursor_row as u16) < rows
                    && content.cursor.shape != CursorShape::Hidden;
                let cursor_color = to_hsla(resolve(
                    Color::Named(NamedColor::Cursor),
                    overrides,
                    &palette,
                ));
                let cursor =
                    cursor_visible.then_some((cursor_bounds, cursor_color, content.cursor.shape));
                drop(term);

                // IME の未確定文字列
                let marked = marked_text.filter(|t| !t.is_empty()).map(|t| {
                    let run = TextRun {
                        len: t.len(),
                        font: font.clone(),
                        color: to_hsla(palette.foreground),
                        background_color: None,
                        underline: Some(UnderlineStyle {
                            thickness: px(1.),
                            color: Some(to_hsla(palette.foreground)),
                            wavy: false,
                        }),
                        strikethrough: None,
                    };
                    let line =
                        text_system.shape_line(SharedString::from(t), font_size, &[run], None);
                    (cursor_bounds.origin, line)
                });

                Prepared {
                    metrics,
                    background,
                    rects,
                    texts,
                    cursor,
                    marked,
                }
            },
            move |bounds, prepared, window, cx| {
                if paint_bg {
                    window.paint_quad(fill(bounds, prepared.background));
                }
                for (rect, color) in &prepared.rects {
                    window.paint_quad(fill(*rect, *color));
                }
                let line_height = prepared.metrics.height;
                for t in &prepared.texts {
                    let _ = t
                        .line
                        .paint(t.origin, line_height, TextAlign::Left, None, window, cx);
                }

                if let Some((origin, line)) = &prepared.marked {
                    // 変換中の文字列はカーソル位置に背景つきで重ねる
                    let w = line.width().max(prepared.metrics.width);
                    window.paint_quad(fill(
                        Bounds::new(*origin, size(w, line_height)),
                        prepared.background,
                    ));
                    let _ = line.paint(*origin, line_height, TextAlign::Left, None, window, cx);
                } else if let Some((cb, color, shape)) = prepared.cursor {
                    let cursor_quad = match (focused, shape) {
                        (true, CursorShape::Beam) => {
                            fill(Bounds::new(cb.origin, size(px(2.), cb.size.height)), color)
                        }
                        (true, CursorShape::Underline) => fill(
                            Bounds::new(
                                point(cb.origin.x, cb.origin.y + cb.size.height - px(2.)),
                                size(cb.size.width, px(2.)),
                            ),
                            color,
                        ),
                        (true, _) => fill(cb, color.opacity(0.6)),
                        (false, _) => outline(cb, color, BorderStyle::Solid),
                    };
                    window.paint_quad(cursor_quad);
                }

                // IME（日本語入力）を受け付ける
                window.handle_input(
                    &focus_handle,
                    ElementInputHandler::new(bounds, entity.clone()),
                    cx,
                );
            },
        )
        .size_full();

        let exited = self.exited;
        div()
            .id("terminal")
            .key_context("Terminal")
            .track_focus(&self.focus_handle)
            .size_full()
            .pl(px(10.))
            .pr(px(6.))
            .pt(px(6.))
            .when(paint_bg, |el| el.bg(background))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_scroll(cx.listener(Self::on_scroll))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_left_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_left_up))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_down))
            .child(terminal_canvas)
            .when(exited, |el| {
                el.relative().child(
                    div()
                        .absolute()
                        .bottom_2()
                        .right_4()
                        .px_2()
                        .py_1()
                        .rounded(px(6.))
                        .bg(rgb(0x26262c))
                        .text_color(rgb(0xa0a0aa))
                        .text_size(px(11.5))
                        .child("[プロセスが終了しました]"),
                )
            })
    }
}

// ---------------------------------------------------------------------------
// IME（日本語入力）
// ---------------------------------------------------------------------------

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn utf16_slice(s: &str, range: std::ops::Range<usize>) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    let end = range.end.min(units.len());
    let start = range.start.min(end);
    String::from_utf16_lossy(&units[start..end])
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: std::ops::Range<usize>,
        adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.marked_text.clone().unwrap_or_default();
        let len = utf16_len(&text);
        let range = range.start.min(len)..range.end.min(len);
        *adjusted_range = Some(range.clone());
        Some(utf16_slice(&text, range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let len = self.marked_text.as_deref().map(utf16_len).unwrap_or(0);
        Some(UTF16Selection {
            range: len..len,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        self.marked_text.as_deref().map(|t| 0..utf16_len(t))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.marked_text = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 確定した文字列（英字の通常入力もここに来る）
        self.marked_text = None;
        if !text.is_empty() {
            self.write(text.as_bytes());
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = (!new_text.is_empty()).then(|| new_text.to_string());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.ime_anchor.get()
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
