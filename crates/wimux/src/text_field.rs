//! 1 行の入力欄（ブラウザの URL 欄など）。
//!
//! - カーソル移動（← → Home End、Ctrl で単語単位、Shift で選択）
//! - マウスでカーソル位置の指定・ドラッグで選択・ダブルクリックで単語選択
//! - コピー / 切り取り / 貼り付け（Ctrl+C / X / V、Ctrl+Insert / Shift+Delete / Shift+Insert）
//! - 日本語入力（IME）
//! - 長い文字列はカーソルが見える位置まで横にずらして表示
//!
//! 描画は GPUI の examples/input.rs（Apache-2.0）と同じ方式: 文字列を自前で整形して、
//! 選択範囲・文字・カーソルを直接描く。

use std::ops::Range;

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
    placeholder: SharedString,
    /// 選択範囲（バイト位置）。空ならカーソルだけ
    selected: Range<usize>,
    /// 選択の向き（true ならカーソルは start 側）
    reversed: bool,
    /// IME で変換中の範囲
    marked: Option<Range<usize>>,
    /// ユーザーが書き換え中（確定・取り消しまで）。この間は外から set_text しない
    edited: bool,
    /// マウスでドラッグ選択中
    dragging: bool,
    /// 横スクロール量（長い URL でカーソルを見えるようにする）
    scroll_x: Pixels,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
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
            placeholder: placeholder.into(),
            selected: 0..0,
            reversed: false,
            marked: None,
            edited: false,
            dragging: false,
            scroll_x: px(0.),
            last_layout: None,
            last_bounds: None,
        }
    }

    pub fn set_text(&mut self, text: &str) {
        self.edited = false;
        if self.text != text {
            self.text = text.to_string();
            self.selected = 0..0;
            self.reversed = false;
            self.marked = None;
            self.scroll_x = px(0.);
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// ユーザーが書き換え中か（Enter / Esc で確定・取り消しするまで）
    pub fn is_edited(&self) -> bool {
        self.edited
    }

    /// フォーカスして全選択（ブラウザの URL 欄と同じ）
    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.select_all();
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    // -----------------------------------------------------------------------
    // カーソル・選択
    // -----------------------------------------------------------------------

    fn cursor(&self) -> usize {
        if self.reversed {
            self.selected.start
        } else {
            self.selected.end
        }
    }

    fn move_to(&mut self, offset: usize) {
        self.selected = offset..offset;
        self.reversed = false;
    }

    fn select_to(&mut self, offset: usize) {
        if self.reversed {
            self.selected.start = offset;
        } else {
            self.selected.end = offset;
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
    }

    fn select_all(&mut self) {
        self.selected = 0..self.text.len();
        self.reversed = false;
    }

    fn prev_char(&self, offset: usize) -> usize {
        self.text[..offset]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_char(&self, offset: usize) -> usize {
        self.text[offset..]
            .chars()
            .next()
            .map_or(self.text.len(), |c| offset + c.len_utf8())
    }

    fn is_word(c: char) -> bool {
        c.is_alphanumeric() || c == '_' || c == '-'
    }

    /// Ctrl+← : 前の単語の先頭へ
    fn prev_word(&self, offset: usize) -> usize {
        let before: Vec<(usize, char)> = self.text[..offset].char_indices().collect();
        let mut i = before.len();
        while i > 0 && !Self::is_word(before[i - 1].1) {
            i -= 1;
        }
        while i > 0 && Self::is_word(before[i - 1].1) {
            i -= 1;
        }
        before.get(i).map_or(offset, |(b, _)| *b)
    }

    /// Ctrl+→ : 次の単語の終わりへ
    fn next_word(&self, offset: usize) -> usize {
        let mut it = self.text[offset..].char_indices().peekable();
        while it.peek().is_some_and(|(_, c)| !Self::is_word(*c)) {
            it.next();
        }
        while it.peek().is_some_and(|(_, c)| Self::is_word(*c)) {
            it.next();
        }
        it.peek().map_or(self.text.len(), |(i, _)| offset + i)
    }

    fn word_range(&self, offset: usize) -> Range<usize> {
        let start = self.text[..offset]
            .char_indices()
            .rev()
            .take_while(|(_, c)| Self::is_word(*c))
            .last()
            .map_or(offset, |(i, _)| i);
        let end = self.text[offset..]
            .char_indices()
            .find(|(_, c)| !Self::is_word(*c))
            .map_or(self.text.len(), |(i, _)| offset + i);
        start..end
    }

    // -----------------------------------------------------------------------
    // 編集
    // -----------------------------------------------------------------------

    fn replace_selection(&mut self, s: &str) {
        let clean: String = s
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let range = self.marked.take().unwrap_or(self.selected.clone());
        self.text.replace_range(range.clone(), &clean);
        self.move_to(range.start + clean.len());
        self.edited = true;
    }

    fn selected_text(&self) -> Option<String> {
        (!self.selected.is_empty()).then(|| self.text[self.selected.clone()].to_string())
    }

    fn copy(&self, cx: &mut Context<Self>) {
        if let Some(t) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(t));
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_selection(text.trim());
        }
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.marked.is_some() {
            return;
        }
        let ks = &ev.keystroke;
        let ctrl = ks.modifiers.control;
        let shift = ks.modifiers.shift;
        let len = self.text.len();
        let cur = self.cursor();
        // 移動先へ。Shift なら選択を広げる
        let go = |this: &mut Self, to: usize| {
            if shift {
                this.select_to(to);
            } else {
                this.move_to(to);
            }
        };
        match ks.key.as_str() {
            "enter" => {
                self.edited = false;
                cx.emit(TextFieldEvent::Submit(self.text.trim().to_string()));
            }
            "escape" => {
                self.edited = false;
                cx.emit(TextFieldEvent::Cancel);
            }
            "left" => {
                let to = if ctrl {
                    self.prev_word(cur)
                } else if !shift && !self.selected.is_empty() {
                    self.selected.start
                } else {
                    self.prev_char(cur)
                };
                go(self, to);
            }
            "right" => {
                let to = if ctrl {
                    self.next_word(cur)
                } else if !shift && !self.selected.is_empty() {
                    self.selected.end
                } else {
                    self.next_char(cur)
                };
                go(self, to);
            }
            "home" => go(self, 0),
            "end" => go(self, len),
            "backspace" => {
                if self.selected.is_empty() {
                    let to = if ctrl {
                        self.prev_word(cur)
                    } else {
                        self.prev_char(cur)
                    };
                    self.select_to(to);
                }
                self.replace_selection("");
            }
            "delete" if shift => {
                self.copy(cx);
                self.replace_selection("");
            }
            "delete" => {
                if self.selected.is_empty() {
                    let to = if ctrl {
                        self.next_word(cur)
                    } else {
                        self.next_char(cur)
                    };
                    self.select_to(to);
                }
                self.replace_selection("");
            }
            "a" if ctrl => self.select_all(),
            "c" if ctrl => self.copy(cx),
            "x" if ctrl => {
                self.copy(cx);
                self.replace_selection("");
            }
            "v" if ctrl => self.paste(cx),
            "insert" if ctrl => self.copy(cx),
            "insert" if shift => self.paste(cx),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    // -----------------------------------------------------------------------
    // マウス
    // -----------------------------------------------------------------------

    fn index_for_x(&self, x: Pixels) -> usize {
        let (Some(bounds), Some(line)) = (self.last_bounds, self.last_layout.as_ref()) else {
            return self.text.len();
        };
        if self.text.is_empty() {
            return 0;
        }
        line.closest_index_for_x(x - bounds.left() + self.scroll_x)
            .min(self.text.len())
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) {
            // フォーカスしていなければ全選択（ブラウザの URL 欄と同じ）
            self.focus(window, cx);
            return;
        }
        let at = self.index_for_x(ev.position.x);
        match ev.click_count {
            2 => {
                self.selected = self.word_range(at);
                self.reversed = false;
            }
            3.. => self.select_all(),
            _ => {
                if ev.modifiers.shift {
                    self.select_to(at);
                } else {
                    self.move_to(at);
                }
                self.dragging = true;
            }
        }
        cx.notify();
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.dragging && ev.pressed_button == Some(MouseButton::Left) {
            let at = self.index_for_x(ev.position.x);
            self.select_to(at);
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, _ev: &MouseUpEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        self.dragging = false;
    }

    // -----------------------------------------------------------------------
    // UTF-16 ⇔ バイト位置（IME とのやりとり用）
    // -----------------------------------------------------------------------

    fn from_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.text.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    fn to_utf16(&self, offset: usize) -> usize {
        self.text[..offset.min(self.text.len())]
            .chars()
            .map(char::len_utf16)
            .sum()
    }

    fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.from_utf16(r.start)..self.from_utf16(r.end)
    }

    fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.to_utf16(r.start)..self.to_utf16(r.end)
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        // フォーカスが外れたら「書き換え中」は終わり（以後はページの URL に追随する）
        if !focused {
            self.edited = false;
            self.dragging = false;
        }
        div()
            .id("text-field")
            .key_context("TextField")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .flex()
            .items_center()
            .size_full()
            .overflow_hidden()
            .cursor_text()
            .child(TextLine {
                field: cx.entity(),
                focused,
            })
    }
}

/// 文字列・選択範囲・カーソルを描く要素
struct TextLine {
    field: Entity<TextField>,
    focused: bool,
}

struct LinePaint {
    line: ShapedLine,
    scroll_x: Pixels,
    selection: Option<PaintQuad>,
    cursor: Option<PaintQuad>,
}

impl IntoElement for TextLine {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextLine {
    type RequestLayoutState = ();
    type PrepaintState = LinePaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let c = th();
        let field = self.field.read(cx);
        let style = window.text_style();
        let empty = field.text.is_empty();
        let (shown, color): (SharedString, u32) = if empty {
            (field.placeholder.clone(), c.text_faint)
        } else {
            (
                field.text.clone().into(),
                if self.focused { c.text } else { c.text_dim },
            )
        };
        let run = TextRun {
            len: shown.len(),
            font: style.font(),
            color: rgb(color).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        // IME で変換中の部分には下線
        let runs = match field.marked.clone() {
            Some(m) if !empty && m.end <= shown.len() => [
                TextRun {
                    len: m.start,
                    ..run.clone()
                },
                TextRun {
                    len: m.end - m.start,
                    underline: Some(UnderlineStyle {
                        color: Some(rgb(color).into()),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: shown.len() - m.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|r| r.len > 0)
            .collect::<Vec<_>>(),
            _ => vec![run],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(shown, font_size, &runs, None);

        let cursor = field.cursor();
        let selected = field.selected.clone();
        let mut scroll_x = field.scroll_x;
        if empty || !self.focused {
            scroll_x = px(0.);
        } else {
            // カーソルが見える位置まで横にずらす
            let at = line.x_for_index(cursor);
            let width = bounds.size.width - px(4.);
            if at - scroll_x > width {
                scroll_x = at - width;
            }
            if at < scroll_x {
                scroll_x = at;
            }
            scroll_x = scroll_x.min((line.width() - width).max(px(0.)));
            scroll_x = scroll_x.max(px(0.));
        }
        let x = |i: usize| bounds.left() + line.x_for_index(i) - scroll_x;
        let height = bounds.size.height;
        let selection = (self.focused && !empty && !selected.is_empty()).then(|| {
            fill(
                Bounds::from_corners(
                    point(x(selected.start), bounds.top()),
                    point(x(selected.end), bounds.top() + height),
                ),
                rgb(c.selection),
            )
        });
        let cursor = (self.focused && selected.is_empty()).then(|| {
            let at = if empty { bounds.left() } else { x(cursor) };
            fill(
                Bounds::new(
                    point(at, bounds.top() + px(1.)),
                    size(px(1.5), height - px(2.)),
                ),
                rgb(c.text),
            )
        });
        LinePaint {
            line,
            scroll_x,
            selection,
            cursor,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.field.read(cx).focus_handle.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.field.clone()),
            cx,
        );
        let scroll_x = prepaint.scroll_x;
        let line = prepaint.line.clone();
        let line_height = window.line_height();
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(sel) = prepaint.selection.take() {
                window.paint_quad(sel);
            }
            let _ = line.paint(
                point(bounds.left() - scroll_x, bounds.top()),
                line_height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
            if let Some(cur) = prepaint.cursor.take() {
                window.paint_quad(cur);
            }
        });
        let has_text = !self.field.read(cx).text.is_empty();
        self.field.update(cx, |f, _| {
            f.scroll_x = scroll_x;
            f.last_layout = has_text.then_some(line);
            f.last_bounds = Some(bounds);
        });
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let r = self.range_from_utf16(&range);
        *adjusted_range = Some(self.range_to_utf16(&r));
        Some(self.text[r].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected),
            reversed: self.reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(r) = range {
            self.selected = self.range_from_utf16(&r);
            self.marked = None;
        }
        self.replace_selection(text);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let r = range
            .map(|r| self.range_from_utf16(&r))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        self.text.replace_range(r.clone(), new_text);
        self.edited = true;
        self.marked = (!new_text.is_empty()).then(|| r.start..r.start + new_text.len());
        let start16 = self.to_utf16(r.start);
        let len16 = new_text.encode_utf16().count();
        self.selected = new_selected
            .map(|s| {
                self.range_from_utf16(&(start16 + s.start.min(len16)..start16 + s.end.min(len16)))
            })
            .unwrap_or(r.start + new_text.len()..r.start + new_text.len());
        self.reversed = false;
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let r = self.range_from_utf16(&range);
        let Some(line) = self.last_layout.as_ref() else {
            return Some(element_bounds);
        };
        let x = |i: usize| element_bounds.left() + line.x_for_index(i) - self.scroll_x;
        Some(Bounds::from_corners(
            point(x(r.start), element_bounds.top()),
            point(x(r.end), element_bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let i = self.index_for_x(point.x);
        Some(self.to_utf16(i))
    }
}

#[cfg(test)]
mod tests {
    use super::TextField;

    #[test]
    fn words() {
        assert!(TextField::is_word('a'));
        assert!(TextField::is_word('あ'));
        assert!(!TextField::is_word('/'));
        assert!(!TextField::is_word('.'));
    }
}
