//! ファイルの表示（ファイル一覧でクリックしたファイルを、区画のタブで開く）。
//!
//! - 読み込みと色付け（シンタックスハイライト）は UI スレッドの外で行う（`load`）
//! - 色付けは syntect（Sublime Text の言語定義を同梱）。テーマの色は使わず、スコープ（comment・string…）を
//!   Sondev のテーマの 16 色に割り当てる（テーマを変えると色も変わる）
//! - 表示だけ（書き換えはしない）。文字の選択とコピー、［エディタで開く］

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;

use gpui_kit::*;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

use crate::theme::{self, *};

/// これより大きいファイルは最初の部分だけ表示する
const MAX_BYTES: usize = 8 * 1024 * 1024;
/// これより大きいファイルは色を付けない（遅くなるので）
const MAX_HIGHLIGHT_BYTES: usize = 2 * 1024 * 1024;
/// これより長い行から先は色を付けない（1 行が巨大な minify された JS など）
const MAX_HIGHLIGHT_LINE: usize = 5000;
const TAB_WIDTH: usize = 4;

// ---------------------------------------------------------------------------
// 読み込みと色付け（UI に依存しない）
// ---------------------------------------------------------------------------

/// 色の種類（スコープから決める）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Comment,
    String,
    Escape,
    Number,
    Constant,
    Keyword,
    Type,
    Function,
    Tag,
    Attribute,
    Heading,
    Link,
    Inserted,
    Deleted,
    Invalid,
}

/// 読み込んだファイル
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    /// 1 行ずつ（タブは空白に展開済み・改行は含まない）
    pub lines: Vec<String>,
    /// 行ごとの色の範囲（バイト位置。色を付けていない行は空）
    pub spans: Vec<Vec<(Range<usize>, Kind)>>,
    /// 言語の名前（色を付けたとき）
    pub language: Option<String>,
    /// 大きすぎて途中までしか読んでいない
    pub truncated: bool,
    /// いちばん長い行（横スクロールの幅に使う）
    pub longest: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    Doc(Doc),
    /// 文字のファイルではない（画像など）
    Binary,
    Error(String),
}

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// ファイルを読み、色を付ける（バックグラウンドで呼ぶ）
pub fn load(path: &Path) -> Loaded {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return Loaded::Error(e.to_string()),
    };
    let truncated = bytes.len() > MAX_BYTES;
    let bytes = &bytes[..bytes.len().min(MAX_BYTES)];
    // 先頭に NUL があれば文字のファイルではない
    if bytes[..bytes.len().min(8000)].contains(&0) {
        return Loaded::Binary;
    }
    let text = String::from_utf8_lossy(bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let lines: Vec<String> = text
        .split('\n')
        .map(|l| expand_tabs(l.strip_suffix('\r').unwrap_or(l)))
        .collect();
    let syntax = (bytes.len() <= MAX_HIGHLIGHT_BYTES)
        .then(|| syntax_for(path, lines.first().map(String::as_str).unwrap_or("")))
        .flatten();
    let spans = match syntax {
        Some(s) => highlight(&lines, s),
        None => vec![Vec::new(); lines.len()],
    };
    let longest = lines
        .iter()
        .enumerate()
        .max_by_key(|(_, l)| l.chars().count())
        .map_or(0, |(i, _)| i);
    Loaded::Doc(Doc {
        longest,
        language: syntax.map(|s| s.name.clone()),
        lines,
        spans,
        truncated,
    })
}

fn expand_tabs(line: &str) -> String {
    if !line.contains('\t') {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len() + 8);
    let mut col = 0;
    for ch in line.chars() {
        if ch == '\t' {
            let n = TAB_WIDTH - col % TAB_WIDTH;
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        } else {
            out.push(ch);
            col += 1;
        }
    }
    out
}

/// ファイルの言語。同梱の言語定義に無いものは近い言語で代用する
fn syntax_for(path: &Path, first_line: &str) -> Option<&'static SyntaxReference> {
    let ss = syntaxes();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let by_ext = |e: &str| ss.find_syntax_by_extension(e);
    let alias = match ext.as_str() {
        "ts" | "tsx" | "mts" | "cts" | "jsx" | "mjs" | "cjs" => Some("js"),
        "jsonc" | "json5" | "ipynb" => Some("json"),
        "vue" | "svelte" | "astro" => Some("html"),
        "scss" | "sass" | "less" => Some("css"),
        "toml" | "ini" | "cfg" | "conf" | "env" | "editorconfig" => Some("properties"),
        "kt" | "kts" | "swift" | "dart" => Some("java"),
        "ps1" | "psm1" | "psd1" => Some("sh"),
        "zig" | "v" => Some("rs"),
        _ => None,
    };
    let special = match name.as_str() {
        "dockerfile" | "containerfile" | ".gitignore" | ".dockerignore" | ".env" => Some("sh"),
        "cargo.lock" => Some("properties"),
        _ => None,
    };
    by_ext(&ext)
        .or_else(|| alias.and_then(by_ext))
        .or_else(|| special.and_then(by_ext))
        .or_else(|| by_ext(&name))
        .or_else(|| ss.find_syntax_by_first_line(first_line))
        .filter(|s| s.name != "Plain Text")
}

/// スコープ → 色の種類（上から順に見る。細かいものを先に）
fn selectors() -> &'static [(Scope, Kind)] {
    static SEL: OnceLock<Vec<(Scope, Kind)>> = OnceLock::new();
    SEL.get_or_init(|| {
        [
            ("comment", Kind::Comment),
            ("constant.character.escape", Kind::Escape),
            ("string", Kind::String),
            ("constant.numeric", Kind::Number),
            ("constant", Kind::Constant),
            ("support.constant", Kind::Constant),
            ("variable.language", Kind::Keyword),
            ("keyword", Kind::Keyword),
            ("storage", Kind::Keyword),
            ("entity.name.function", Kind::Function),
            ("support.function", Kind::Function),
            ("variable.function", Kind::Function),
            ("entity.name.tag", Kind::Tag),
            ("entity.other.attribute-name", Kind::Attribute),
            ("entity.name", Kind::Type),
            ("entity.other.inherited-class", Kind::Type),
            ("support.type", Kind::Type),
            ("support.class", Kind::Type),
            ("markup.heading", Kind::Heading),
            ("markup.underline.link", Kind::Link),
            ("markup.inserted", Kind::Inserted),
            ("markup.deleted", Kind::Deleted),
            ("invalid", Kind::Invalid),
        ]
        .into_iter()
        .filter_map(|(s, k)| Some((Scope::new(s).ok()?, k)))
        .collect()
    })
}

/// 内側のスコープから順に見て、最初に当てはまった色
fn classify(stack: &[Scope]) -> Option<Kind> {
    let sel = selectors();
    stack.iter().rev().find_map(|scope| {
        sel.iter()
            .find(|(s, _)| s.is_prefix_of(*scope))
            .map(|(_, k)| *k)
    })
}

fn highlight(lines: &[String], syntax: &SyntaxReference) -> Vec<Vec<(Range<usize>, Kind)>> {
    let ss = syntaxes();
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut out = Vec::with_capacity(lines.len());
    let mut stopped = false;
    for line in lines {
        if stopped || line.len() > MAX_HIGHLIGHT_LINE {
            // 巨大な行より先は、状態がずれるので色を付けない
            stopped = true;
            out.push(Vec::new());
            continue;
        }
        let with_nl = format!("{line}\n");
        let Ok(ops) = state.parse_line(&with_nl, ss) else {
            stopped = true;
            out.push(Vec::new());
            continue;
        };
        let mut spans: Vec<(Range<usize>, Kind)> = Vec::new();
        let mut push = |range: Range<usize>, kind: Option<Kind>| {
            let range = range.start.min(line.len())..range.end.min(line.len());
            let Some(kind) = kind.filter(|_| !range.is_empty()) else {
                return;
            };
            match spans.last_mut() {
                Some((r, k)) if *k == kind && r.end == range.start => r.end = range.end,
                _ => spans.push((range, kind)),
            }
        };
        let mut pos = 0;
        for (at, op) in ops {
            if at > pos {
                push(pos..at, classify(stack.as_slice()));
                pos = at;
            }
            let _ = stack.apply(&op);
        }
        push(pos..with_nl.len(), classify(stack.as_slice()));
        out.push(spans);
    }
    out
}

/// 選択の位置（行と、行の中のバイト位置）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

/// 行の中の区切り（色・選択中か）。色の範囲と選択の範囲を重ならないように分ける
fn segments(
    len: usize,
    spans: &[(Range<usize>, Kind)],
    selected: Option<Range<usize>>,
) -> Vec<(Range<usize>, Option<Kind>, bool)> {
    let mut cuts = vec![0, len];
    for (r, _) in spans {
        cuts.push(r.start.min(len));
        cuts.push(r.end.min(len));
    }
    if let Some(s) = &selected {
        cuts.push(s.start.min(len));
        cuts.push(s.end.min(len));
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .filter(|w| w[0] < w[1])
        .map(|w| {
            let r = w[0]..w[1];
            let kind = spans
                .iter()
                .find(|(s, _)| s.start <= r.start && r.end <= s.end)
                .map(|(_, k)| *k);
            let sel = selected
                .as_ref()
                .is_some_and(|s| s.start <= r.start && r.end <= s.end);
            (r, kind, sel)
        })
        .filter(|(_, k, s)| k.is_some() || *s)
        .collect()
}

/// 選択範囲の文字列（行の終わりは改行）
fn selected_text(lines: &[String], a: Pos, b: Pos) -> String {
    let (s, e) = if a <= b { (a, b) } else { (b, a) };
    let mut out = String::new();
    for i in s.line..=e.line.min(lines.len().saturating_sub(1)) {
        let line = &lines[i];
        let from = if i == s.line {
            s.col.min(line.len())
        } else {
            0
        };
        let to = if i == e.line {
            e.col.min(line.len())
        } else {
            line.len()
        };
        out.push_str(line.get(from..to.max(from)).unwrap_or(""));
        if i != e.line {
            out.push('\n');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 表示（GPUI）
// ---------------------------------------------------------------------------

pub enum FileViewEvent {
    /// クリックされた（その区画・タブをアクティブにする）
    Focused,
}

pub struct FileView {
    path: PathBuf,
    focus_handle: FocusHandle,
    loaded: Option<Rc<Loaded>>,
    scroll: UniformListScrollHandle,
    anchor: Option<Pos>,
    head: Option<Pos>,
    selecting: bool,
    font: Font,
    font_size: f32,
    editor: Option<String>,
}

impl EventEmitter<FileViewEvent> for FileView {}

impl Focusable for FileView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl FileView {
    pub fn new(
        path: PathBuf,
        font: Font,
        font_size: f32,
        editor: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut v = Self {
            path,
            focus_handle: cx.focus_handle(),
            loaded: None,
            scroll: UniformListScrollHandle::new(),
            anchor: None,
            head: None,
            selecting: false,
            font,
            font_size,
            editor,
        };
        v.reload(cx);
        v
    }

    pub fn set_font(&mut self, font: Font) {
        self.font = font;
    }

    pub fn set_font_size(&mut self, size: f32) {
        self.font_size = size;
    }

    /// 読み直す（バックグラウンドで）
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move { load(&path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loaded = Some(Rc::new(loaded));
                cx.notify();
            });
        })
        .detach();
    }

    fn doc(&self) -> Option<&Doc> {
        match self.loaded.as_deref()? {
            Loaded::Doc(d) => Some(d),
            _ => None,
        }
    }

    fn copy(&self, cx: &mut Context<Self>) {
        let (Some(doc), Some(a), Some(b)) = (self.doc(), self.anchor, self.head) else {
            return;
        };
        if a == b {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(selected_text(&doc.lines, a, b)));
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        let last = doc.lines.len().saturating_sub(1);
        let end = Pos {
            line: last,
            col: doc.lines.get(last).map_or(0, String::len),
        };
        self.anchor = Some(Pos { line: 0, col: 0 });
        self.head = Some(end);
        cx.notify();
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let ctrl = k.modifiers.control || k.modifiers.platform;
        match k.key.as_str() {
            "c" if ctrl => self.copy(cx),
            "a" if ctrl => self.select_all(cx),
            "escape" => {
                self.anchor = None;
                self.head = None;
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// 行の中の選択範囲（バイト位置）
    fn line_selection(&self, line: usize, len: usize) -> Option<Range<usize>> {
        let (a, b) = (self.anchor?, self.head?);
        let (s, e) = if a <= b { (a, b) } else { (b, a) };
        if s == e || line < s.line || line > e.line {
            return None;
        }
        let from = if line == s.line { s.col } else { 0 };
        let to = if line == e.line { e.col } else { len };
        Some(from.min(len)..to.min(len))
    }

    fn render_lines(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(loaded) = self.loaded.clone() else {
            return message("読み込み中…");
        };
        let Loaded::Doc(doc) = &*loaded else {
            return match &*loaded {
                Loaded::Binary => {
                    message("文字のファイルではないので表示できません（エディタで開いてください）")
                }
                Loaded::Error(e) => message(&format!("開けませんでした: {e}")),
                Loaded::Doc(_) => unreachable!(),
            };
        };
        let count = doc.lines.len();
        let longest = doc.longest;
        let digits = count.to_string().len().max(3);
        let font = self.font.clone();
        let font_size = self.font_size;
        let row_h = (font_size * theme::line_height()).round();
        let gutter = px(font_size * 0.62 * digits as f32 + 20.);
        let loaded2 = loaded.clone();
        uniform_list(
            "file-view-lines",
            count,
            cx.processor(move |this, range: Range<usize>, _window, cx| {
                let Loaded::Doc(doc) = &*loaded2 else {
                    return Vec::new();
                };
                let c = th();
                let p = crate::terminal::colors::current();
                let ansi = |i: usize| hsla_of(crate::theme::hex(p.ansi[i]));
                let color = |k: Kind| -> HighlightStyle {
                    let (col, bold) = match k {
                        Kind::Comment => (ansi(8), false),
                        Kind::String => (ansi(2), false),
                        Kind::Escape => (ansi(6), false),
                        Kind::Number | Kind::Constant => (ansi(3), false),
                        Kind::Keyword => (ansi(5), false),
                        Kind::Type => (ansi(3), false),
                        Kind::Function => (ansi(4), false),
                        Kind::Tag => (ansi(1), false),
                        Kind::Attribute => (ansi(3), false),
                        Kind::Heading => (ansi(4), true),
                        Kind::Link => (ansi(6), false),
                        Kind::Inserted => (ansi(2), false),
                        Kind::Deleted | Kind::Invalid => (ansi(1), false),
                    };
                    HighlightStyle {
                        color: Some(col),
                        font_weight: bold.then_some(FontWeight::BOLD),
                        ..Default::default()
                    }
                };
                let sel_bg = hsla_of(c.accent).opacity(0.35);
                range
                    .filter_map(|ix| {
                        let line = doc.lines.get(ix)?;
                        let spans = doc.spans.get(ix).map(Vec::as_slice).unwrap_or(&[]);
                        let sel = this.line_selection(ix, line.len());
                        let hl: Vec<(Range<usize>, HighlightStyle)> =
                            segments(line.len(), spans, sel)
                                .into_iter()
                                .map(|(r, k, s)| {
                                    let mut st = k.map(color).unwrap_or_default();
                                    if s {
                                        st.background_color = Some(sel_bg);
                                    }
                                    (r, st)
                                })
                                .collect();
                        let text =
                            StyledText::new(SharedString::from(line.clone())).with_highlights(hl);
                        let layout = text.layout().clone();
                        let layout2 = layout.clone();
                        let len = line.len();
                        let line_text = line.clone();
                        let hit = move |layout: &TextLayout, pos: Point<Pixels>| -> usize {
                            if len == 0 {
                                return 0;
                            }
                            let i = match layout.index_for_position(pos) {
                                Ok(i) | Err(i) => i.min(len),
                            };
                            // 文字の途中にならないように
                            (0..=i)
                                .rev()
                                .find(|j| line_text.is_char_boundary(*j))
                                .unwrap_or(0)
                        };
                        let hit2 = hit.clone();
                        Some(
                            div()
                                .id(("file-line", ix))
                                .h(px(row_h))
                                .w_full()
                                .flex()
                                .items_center()
                                .whitespace_nowrap()
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                                        window.focus(&this.focus_handle, cx);
                                        cx.emit(FileViewEvent::Focused);
                                        let pos = Pos {
                                            line: ix,
                                            col: hit(&layout, ev.position),
                                        };
                                        if ev.modifiers.shift && this.anchor.is_some() {
                                            this.head = Some(pos);
                                        } else if ev.click_count >= 3 {
                                            this.anchor = Some(Pos { line: ix, col: 0 });
                                            this.head = Some(Pos { line: ix, col: len });
                                        } else {
                                            this.anchor = Some(pos);
                                            this.head = Some(pos);
                                        }
                                        this.selecting = ev.click_count < 3;
                                        cx.notify();
                                    }),
                                )
                                .on_mouse_move(cx.listener(
                                    move |this, ev: &MouseMoveEvent, _, cx| {
                                        if this.selecting
                                            && ev.pressed_button == Some(MouseButton::Left)
                                        {
                                            let pos = Pos {
                                                line: ix,
                                                col: hit2(&layout2, ev.position),
                                            };
                                            if this.head != Some(pos) {
                                                this.head = Some(pos);
                                                cx.notify();
                                            }
                                        }
                                    },
                                ))
                                .child(
                                    div()
                                        .w(gutter)
                                        .flex_none()
                                        .pr(px(12.))
                                        .flex()
                                        .justify_end()
                                        .text_color(rgb(c.text_faint))
                                        .child((ix + 1).to_string()),
                                )
                                .child(div().flex_none().child(text))
                                .into_any_element(),
                        )
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        // 長い行は横にスクロールして読む（いちばん長い行の幅に合わせる）
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .with_width_from_item(Some(longest))
        .size_full()
        .font(font)
        .text_size(px(font_size))
        .text_color(hsla_of(crate::theme::hex(
            crate::terminal::colors::current().foreground,
        )))
        .into_any_element()
    }
}

fn hsla_of(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn message(text: &str) -> AnyElement {
    div()
        .p(px(16.))
        .text_size(px(UI_TEXT))
        .text_color(rgb(th().text_faint))
        .child(text.to_string())
        .into_any_element()
}

impl Render for FileView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = th();
        let info = match self.doc() {
            Some(d) => {
                let mut parts = vec![format!("{} 行", d.lines.len())];
                if let Some(l) = &d.language {
                    parts.push(l.clone());
                }
                if d.truncated {
                    parts.push("大きいので途中まで".into());
                }
                parts.join(" · ")
            }
            None => String::new(),
        };
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px(px(8.))
                .h(px(22.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .cursor_pointer()
                .text_size(px(UI_SMALL))
                .text_color(rgb(c.text_dim))
                .bg(rgb(c.hover_bg))
                .hover(|s| s.bg(rgb(c.hover_bg_strong)).text_color(rgb(c.text)))
                .child(label)
        };
        let path_label = crate::window::display_path(&self.path);
        div()
            .id("file-view")
            .key_context("FileView")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.selecting = false),
            )
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .h(px(30.))
                    .px(px(10.))
                    .border_b_1()
                    .border_color(rgb(c.panel_border))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(UI_SMALL))
                            .text_color(rgb(c.text_dim))
                            .child(path_label),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(rgb(c.text_faint))
                            .child(info),
                    )
                    .child(
                        button("file-view-reload", "↻ 読み直す").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.reload(cx);
                            },
                        )),
                    )
                    .child(
                        button("file-view-editor", "エディタで開く").on_click(cx.listener(
                            |this, _, _, _| {
                                crate::open::open_path(
                                    &this.path,
                                    None,
                                    None,
                                    this.editor.as_deref(),
                                );
                            },
                        )),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .py(px(4.))
                    .child(self.render_lines(cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, Loaded, Pos, expand_tabs, load, segments, selected_text};

    #[test]
    fn tabs() {
        assert_eq!(expand_tabs("\ta"), "    a");
        assert_eq!(expand_tabs("ab\tc"), "ab  c");
    }

    #[test]
    fn highlight_rust() {
        let dir = std::env::temp_dir().join(format!("wimux-view-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.rs");
        std::fs::write(&f, "// hi\nfn main() { let s = \"x\"; }\n").unwrap();
        let Loaded::Doc(d) = load(&f) else {
            panic!("not a doc")
        };
        assert_eq!(d.language.as_deref(), Some("Rust"));
        assert_eq!(d.lines[0], "// hi");
        assert_eq!(d.spans[0], vec![(0..5, Kind::Comment)]);
        assert!(
            d.spans[1]
                .iter()
                .any(|(r, k)| *k == Kind::Keyword && &d.lines[1][r.clone()] == "fn")
        );
        assert!(
            d.spans[1]
                .iter()
                .any(|(r, k)| *k == Kind::String && d.lines[1][r.clone()].contains('x'))
        );

        // TypeScript は JavaScript の定義で色を付ける
        let ts = dir.join("a.ts");
        std::fs::write(&ts, "const a = 1;\n").unwrap();
        let Loaded::Doc(d) = load(&ts) else { panic!() };
        assert_eq!(d.language.as_deref(), Some("JavaScript"));

        // 文字のファイルではない
        let bin = dir.join("a.bin");
        std::fs::write(&bin, [0u8, 1, 2, 3]).unwrap();
        assert_eq!(load(&bin), Loaded::Binary);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn segments_and_selection() {
        let spans = vec![(0..2, Kind::Keyword), (5..8, Kind::String)];
        let s = segments(10, &spans, Some(1..6));
        assert_eq!(
            s,
            vec![
                (0..1, Some(Kind::Keyword), false),
                (1..2, Some(Kind::Keyword), true),
                (2..5, None, true),
                (5..6, Some(Kind::String), true),
                (6..8, Some(Kind::String), false),
            ]
        );
        let lines = vec!["abc".to_string(), "defg".to_string(), "hi".to_string()];
        let t = selected_text(&lines, Pos { line: 2, col: 1 }, Pos { line: 0, col: 1 });
        assert_eq!(t, "bc\ndefg\nh");
    }
}
