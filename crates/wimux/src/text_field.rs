//! 1 行の入力欄（ブラウザの URL 欄など）。日本語入力・貼り付け対応。

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::*;

pub enum TextFieldEvent {
    /// Enter で確定
    Submit(String),
    /// Esc で取り消し
    Cancel,
}

pub struct TextField {
    focus_handle: FocusHandle,
    text: String,
    marked: Option<String>,
    placeholder: SharedString,
    /// フォーカスした直後は全選択（打つと全部置き換わる、ブラウザの URL 欄と同じ）
    all_selected: bool,
    /// ユーザーが書き換え中（確定・取り消しまで）。この間は外から set_text しない
    edited: bool,
}

impl EventEmitter<TextFieldEvent> for TextField {}

impl Focusable for TextField {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextField {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            text: String::new(),
            marked: None,
            placeholder: placeholder.into(),
            all_selected: false,
            edited: false,
        }
    }

    pub fn set_text(&mut self, text: &str) {
        self.edited = false;
        if self.text != text {
            self.text = text.to_string();
        }
    }

    /// ユーザーが書き換え中か（Enter / Esc で確定・取り消しするまで）
    pub fn is_edited(&self) -> bool {
        self.edited
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// フォーカスして全選択
    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.all_selected = !self.text.is_empty();
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn insert(&mut self, s: &str) {
        self.edited = true;
        if self.all_selected {
            self.text.clear();
            self.all_selected = false;
        }
        self.text
            .extend(s.chars().map(|c| if c.is_control() { ' ' } else { c }));
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.insert(text.trim());
            cx.notify();
        }
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.marked.is_some() {
            return;
        }
        let ks = &ev.keystroke;
        let ctrl = ks.modifiers.control;
        match ks.key.as_str() {
            "enter" => {
                self.edited = false;
                cx.emit(TextFieldEvent::Submit(self.text.trim().to_string()))
            }
            "escape" => {
                self.edited = false;
                cx.emit(TextFieldEvent::Cancel)
            }
            "backspace" => {
                self.edited = true;
                if self.all_selected || ctrl {
                    self.text.clear();
                    self.all_selected = false;
                } else {
                    self.text.pop();
                }
            }
            "a" if ctrl => self.all_selected = !self.text.is_empty(),
            "v" if ctrl => self.paste(cx),
            "insert" if ks.modifiers.shift => self.paste(cx),
            "left" | "right" | "home" | "end" => self.all_selected = false,
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = th();
        let focused = self.focus_handle.is_focused(window);
        // フォーカスが外れたら「書き換え中」は終わり（以後はページの URL に追随する）
        if !focused {
            self.edited = false;
        }
        let entity = cx.entity();
        let focus = self.focus_handle.clone();
        let empty = self.text.is_empty() && self.marked.is_none();
        let mut shown = self.text.clone();
        if let Some(m) = &self.marked {
            shown.push_str(m);
        }

        div()
            .id("text-field")
            .key_context("TextField")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if !this.focus_handle.is_focused(window) {
                        this.focus(window, cx);
                    }
                }),
            )
            .relative()
            .flex()
            .items_center()
            .size_full()
            .overflow_hidden()
            .cursor_text()
            .when(empty, |el| {
                el.child(
                    div()
                        .text_color(rgb(c.text_faint))
                        .child(self.placeholder.clone()),
                )
            })
            .when(!empty, |el| {
                el.child(
                    div()
                        .truncate()
                        .text_color(rgb(if focused { c.text } else { c.text_dim }))
                        .when(focused && self.all_selected, |el| el.bg(rgb(c.selection)))
                        .child(shown),
                )
            })
            .when(focused && !self.all_selected, |el| {
                el.child(div().w(px(1.5)).h(px(14.)).bg(rgb(c.text)))
            })
            .child(
                // 日本語入力（IME）を受け取る
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
            )
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        _range: std::ops::Range<usize>,
        adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let t = self.marked.clone().unwrap_or_default();
        *adjusted_range = Some(0..t.encode_utf16().count());
        Some(t)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let len = self
            .marked
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
        self.marked.as_deref().map(|t| 0..t.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = None;
        self.insert(text);
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
        self.edited = true;
        if self.all_selected && !new_text.is_empty() {
            self.text.clear();
            self.all_selected = false;
        }
        self.marked = (!new_text.is_empty()).then(|| new_text.to_string());
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
