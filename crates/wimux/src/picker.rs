//! 画面上部に出る入力ボックス付きのポップアップ。
//! - フォルダ履歴から選んで移動（cdi）
//! - ワークスペース名の変更
//! - コマンドパレット（Ctrl+Shift+P）
//!
//! 日本語入力（IME）にも対応している。

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::dirs::DirHistory;
use crate::theme::*;

const MAX_RESULTS: usize = 12;
/// コマンドパレットに一度に出す数
const MAX_COMMANDS: usize = 14;

#[derive(Debug, Clone)]
pub enum PickerKind {
    /// フォルダ履歴から選ぶ（旧 cdi。今の cdi はターミナル内で選ぶ）
    #[allow(dead_code)]
    Dirs,
    /// テキストを入力する（ワークスペース名など）
    Text { title: String },
    /// コマンドパレット（名前で絞り込んで、選んだコマンドの id を返す）
    Commands(Vec<CommandItem>),
}

#[derive(Debug, Clone)]
pub struct CommandItem {
    pub id: String,
    pub label: String,
    /// 表示用のキー（"Ctrl+T / Ctrl+Shift+T"）
    pub keys: String,
}

pub enum PickerEvent {
    /// 選ばれた（または入力された）文字列
    Confirmed(String),
    Dismissed,
    /// 入力が変わった（親が履歴を引き直して `refresh` を呼ぶ）
    QueryChanged,
}

pub struct Picker {
    kind: PickerKind,
    focus_handle: FocusHandle,
    query: String,
    marked_text: Option<String>,
    results: Vec<String>,
    /// コマンドパレットで絞り込んだ結果（`PickerKind::Commands` の何番目か）
    commands: Vec<usize>,
    selected: usize,
}

impl EventEmitter<PickerEvent> for Picker {}

impl Focusable for Picker {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Picker {
    pub fn new(
        kind: PickerKind,
        initial: String,
        history: Option<&DirHistory>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            kind,
            focus_handle: cx.focus_handle(),
            query: initial,
            marked_text: None,
            results: Vec::new(),
            commands: Vec::new(),
            selected: 0,
        };
        this.refresh(history);
        this
    }

    /// 結果を更新する（Dirs・Commands のとき）
    pub fn refresh(&mut self, history: Option<&DirHistory>) {
        match (&self.kind, history) {
            (PickerKind::Dirs, Some(h)) => {
                self.results = h.query(&self.query, MAX_RESULTS);
            }
            (PickerKind::Commands(items), _) => {
                let words: Vec<String> = self
                    .query
                    .to_lowercase()
                    .split_whitespace()
                    .map(str::to_string)
                    .collect();
                self.commands = items
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| {
                        let hay = format!("{} {} {}", c.label, c.id, c.keys).to_lowercase();
                        words.iter().all(|w| hay.contains(w.as_str()))
                    })
                    .map(|(i, _)| i)
                    .collect();
            }
            _ => {}
        }
        self.selected = self.selected.min(self.count().saturating_sub(1));
    }

    /// 選べる結果の数
    fn count(&self) -> usize {
        match self.kind {
            PickerKind::Commands(_) => self.commands.len(),
            _ => self.results.len(),
        }
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        let value = match &self.kind {
            PickerKind::Dirs => self.results.get(self.selected).cloned(),
            PickerKind::Text { .. } => Some(self.query.trim().to_string()),
            PickerKind::Commands(items) => self
                .commands
                .get(self.selected)
                .and_then(|i| items.get(*i))
                .map(|c| c.id.clone()),
        };
        match value {
            Some(v) => cx.emit(PickerEvent::Confirmed(v)),
            None => cx.emit(PickerEvent::Dismissed),
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        // 改行やタブは空白にして 1 行にする
        let text: String = text
            .trim()
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        self.query.push_str(&text);
        self.selected = 0;
        cx.emit(PickerEvent::QueryChanged);
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.marked_text.is_some() {
            return;
        }
        let ks = &ev.keystroke;
        let ctrl = ks.modifiers.control;
        match ks.key.as_str() {
            "escape" => cx.emit(PickerEvent::Dismissed),
            "enter" => self.confirm(cx),
            "up" => self.selected = self.selected.saturating_sub(1),
            "down" => {
                if self.selected + 1 < self.count() {
                    self.selected += 1;
                }
            }
            "p" if ctrl => self.selected = self.selected.saturating_sub(1),
            "n" if ctrl => {
                if self.selected + 1 < self.count() {
                    self.selected += 1;
                }
            }
            "backspace" => {
                self.query.pop();
                self.selected = 0;
                cx.emit(PickerEvent::QueryChanged);
            }
            "u" if ctrl => {
                self.query.clear();
                self.selected = 0;
                cx.emit(PickerEvent::QueryChanged);
            }
            // 貼り付け（Ctrl+V / Ctrl+Shift+V / Shift+Insert）
            "v" if ctrl => self.paste(cx),
            "insert" if ks.modifiers.shift => self.paste(cx),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for Picker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let focus = self.focus_handle.clone();
        let title = match &self.kind {
            PickerKind::Dirs => "cdi — フォルダへ移動".to_string(),
            PickerKind::Text { title } => title.clone(),
            PickerKind::Commands(_) => "コマンドパレット".to_string(),
        };
        let is_commands = matches!(self.kind, PickerKind::Commands(_));
        let placeholder = matches!(self.kind, PickerKind::Dirs | PickerKind::Commands(_))
            && self.query.is_empty();
        let mut display = self.query.clone();
        if let Some(m) = &self.marked_text {
            display.push_str(m);
        }

        let input = div()
            .relative()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(38.))
            .px(px(12.))
            .bg(rgb(th().term_bg))
            .border_b_1()
            .border_color(rgb(th().panel_border))
            .text_size(px(14.))
            .text_color(rgb(th().text))
            .when(
                matches!(self.kind, PickerKind::Dirs | PickerKind::Commands(_)),
                |el| el.child(div().text_color(rgb(th().text_faint)).child("›")),
            )
            .child(if placeholder {
                div()
                    .text_color(rgb(th().text_faint))
                    .child(if is_commands {
                        "コマンドを検索（例: 分割、タブ、ブラウザ）"
                    } else {
                        "フォルダを検索（スペース区切りで絞り込み）"
                    })
            } else {
                div().child(format!("{display}▏"))
            })
            .child(
                // IME の入力を受け取るための透明な領域
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                    },
                )
                .absolute()
                .size_full(),
            );

        let results = self.results.iter().enumerate().map(|(i, path)| {
            let selected = i == self.selected;
            let (name, parent) = split_path(path);
            div()
                .id(("picker-item", i))
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(30.))
                .px(px(10.))
                .rounded(px(6.))
                .cursor_pointer()
                .when(selected, |el| el.bg(rgb(th().selected_bg)))
                .when(!selected, |el| el.hover(|s| s.bg(rgb(th().hover_bg))))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected = i;
                    this.confirm(cx);
                }))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(UI_TEXT))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(if selected { 0xffffff } else { th().text }))
                        .child(name),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(UI_SMALL))
                        .text_color(rgb(if selected {
                            th().selected_meta
                        } else {
                            th().text_faint
                        }))
                        .child(parent),
                )
        });

        // コマンドパレットの結果（名前と、右にキー）
        let command_rows: Vec<AnyElement> = match &self.kind {
            PickerKind::Commands(items) => self
                .commands
                .iter()
                .enumerate()
                .filter_map(|(row, i)| Some((row, items.get(*i)?)))
                .take(MAX_COMMANDS)
                .map(|(row, c)| {
                    let selected = row == self.selected;
                    div()
                        .id(("command-item", row))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(10.))
                        .h(px(30.))
                        .px(px(10.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .when(selected, |el| el.bg(rgb(th().selected_bg)))
                        .when(!selected, |el| el.hover(|s| s.bg(rgb(th().hover_bg))))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected = row;
                            this.confirm(cx);
                        }))
                        .child(
                            div()
                                .text_size(px(UI_TEXT))
                                .text_color(rgb(if selected { 0xffffff } else { th().text }))
                                .child(c.label.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(UI_SMALL))
                                .text_color(rgb(if selected {
                                    th().selected_meta
                                } else {
                                    th().text_faint
                                }))
                                .child(c.keys.clone()),
                        )
                        .into_any_element()
                })
                .collect(),
            _ => Vec::new(),
        };
        let no_commands = is_commands && self.commands.is_empty();

        let is_dirs = matches!(self.kind, PickerKind::Dirs);
        let is_list = is_dirs || is_commands;
        div()
            .id("picker")
            .key_context("Picker")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .w(px(620.))
            .max_w(relative(0.9))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(10.))
            .bg(rgb(th().chrome_bg))
            .border_1()
            .border_color(rgb(th().panel_border))
            .shadow_lg()
            .when(!is_list, |el| {
                el.child(
                    div()
                        .px(px(12.))
                        .pt(px(10.))
                        .pb(px(6.))
                        .bg(rgb(th().term_bg))
                        .text_size(px(UI_SMALL))
                        .text_color(rgb(th().text_dim))
                        .child(title.clone()),
                )
            })
            .child(input)
            .when(is_dirs, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .p(px(6.))
                        .children(results)
                        .when(self.results.is_empty(), |el| {
                            el.child(
                                div()
                                    .px(px(10.))
                                    .py(px(8.))
                                    .text_size(px(UI_SMALL))
                                    .text_color(rgb(th().text_faint))
                                    .child("履歴がありません（cd で移動すると記録されます）"),
                            )
                        }),
                )
            })
            .when(is_commands, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .p(px(6.))
                        .children(command_rows)
                        .when(no_commands, |el| {
                            el.child(
                                div()
                                    .px(px(10.))
                                    .py(px(8.))
                                    .text_size(px(UI_SMALL))
                                    .text_color(rgb(th().text_faint))
                                    .child("見つかりません"),
                            )
                        }),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_between()
                    .px(px(12.))
                    .py(px(7.))
                    .border_t_1()
                    .border_color(rgb(th().panel_border))
                    .text_size(px(11.))
                    .text_color(rgb(th().text_faint))
                    .child(if is_list { title } else { String::new() })
                    .child(if is_dirs {
                        "↑↓ 選択   Enter 移動   Esc 閉じる"
                    } else if is_commands {
                        "↑↓ 選択   Enter 実行   Esc 閉じる"
                    } else {
                        "Enter 決定   Esc キャンセル"
                    }),
            )
    }
}

/// "C:\Users\kento\wimux" → ("wimux", "C:\Users\kento")
fn split_path(path: &str) -> (String, String) {
    let trimmed = path.trim_end_matches(['\\', '/']);
    match trimmed.rfind(['\\', '/']) {
        Some(i) if i + 1 < trimmed.len() => {
            (trimmed[i + 1..].to_string(), trimmed[..i].to_string())
        }
        _ => (path.to_string(), String::new()),
    }
}

impl EntityInputHandler for Picker {
    fn text_for_range(
        &mut self,
        _range: std::ops::Range<usize>,
        adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.marked_text.clone().unwrap_or_default();
        *adjusted_range = Some(0..text.encode_utf16().count());
        Some(text)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let len = self
            .marked_text
            .as_deref()
            .map(|t| t.encode_utf16().count())
            .unwrap_or(0);
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
        self.marked_text
            .as_deref()
            .map(|t| 0..t.encode_utf16().count())
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
        self.marked_text = None;
        // 改行などの制御文字は無視
        self.query.extend(text.chars().filter(|c| !c.is_control()));
        self.selected = 0;
        cx.emit(PickerEvent::QueryChanged);
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
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
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
