//! A one-line text field, since GPUI doesn't come with one.
// No partial selection or IME yet.

use crate::u;
use crate::{FAINT, LINE, TEXT, ACCENT};
use gpui::{
    Animation, AnimationExt, ClickEvent, Context, EventEmitter, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Render, ShapedLine, SharedString, StatefulInteractiveElement, Styled, TextRun, Window, canvas, div, rgb,
};
use std::{cell::RefCell, rc::Rc, time::Duration};

pub enum InputEvent {
    Submit,
}

pub struct TextInput {
    focus: FocusHandle,
    pub text: String,
    placeholder: SharedString,
    pub icon: bool,
    pub numeric: bool,
    pub compact: bool,
    back: usize,
    pulse: usize,
    shaped: Rc<RefCell<Option<(Pixels, ShapedLine)>>>,
}

impl EventEmitter<InputEvent> for TextInput {}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        TextInput { focus: cx.focus_handle(), text: String::new(), placeholder: placeholder.into(), icon: true, numeric: false, compact: false, back: 0, pulse: 0, shaped: Rc::default() }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
    }

    fn caret(&self) -> usize {
        let n = self.text.chars().count();
        self.text.char_indices().nth(n - self.back.min(n)).map_or(self.text.len(), |(i, _)| i)
    }

    fn insert(&mut self, s: &str) {
        let at = self.caret();
        self.text.insert_str(at, s);
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let len = self.text.chars().count();
        self.back = self.back.min(len);
        match (k.key.as_str(), k.modifiers.platform || k.modifiers.control) {
            ("backspace", true) | ("escape", _) => {
                self.text.clear();
                self.back = 0;
            }
            ("backspace", false) => {
                let at = self.caret();
                if let Some(c) = self.text[..at].chars().next_back() {
                    self.text.remove(at - c.len_utf8());
                }
            }
            ("delete", _) => {
                let at = self.caret();
                if at < self.text.len() {
                    self.text.remove(at);
                    self.back -= 1;
                }
            }
            ("left", false) => self.back = (self.back + 1).min(len),
            ("right", false) => self.back = self.back.saturating_sub(1),
            ("left", true) | ("home", _) => self.back = len,
            ("right", true) | ("end", _) => self.back = 0,
            ("enter", _) => cx.emit(InputEvent::Submit),
            ("v", true) => {
                if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    let numeric = self.numeric;
                    let line: String = t.lines().next().unwrap_or("").chars().filter(|c| !numeric || c.is_ascii_digit()).collect();
                    self.insert(&line);
                }
            }
            (_, false) => match &k.key_char {
                Some(ch) if !ch.chars().any(char::is_control) && (!self.numeric || ch.chars().all(|c| c.is_ascii_digit())) => {
                    let flip = cfg!(target_os = "macos") && window.capslock().on && ch.chars().count() == 1 && ch.chars().all(char::is_alphabetic);
                    if flip {
                        let flipped: String = ch.chars().flat_map(|c| if c.is_uppercase() { c.to_lowercase().collect::<Vec<_>>() } else { c.to_uppercase().collect() }).collect();
                        self.insert(&flipped);
                    } else {
                        self.insert(ch);
                    }
                }
                _ => return,
            },
            _ => return,
        }
        self.pulse += 1;
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let (before, after) = self.text.split_at(self.caret());
        let (before, after) = (SharedString::from(before.to_string()), SharedString::from(after.to_string()));
        div()
            .id("input")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                let at = this.shaped.borrow().as_ref().map(|(left, line)| line.closest_index_for_x(ev.position.x - *left));
                this.back = at.filter(|&i| this.text.is_char_boundary(i)).map_or(0, |i| this.text[i..].chars().count());
                this.pulse += 1;
                cx.notify();
            }))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.focus(window, cx)))
            .h(u(if self.compact { 30. } else { 40. }))
            .px(u(if self.compact { 12. } else { 16. }))
            .rounded(u(if self.compact { 15. } else { 20. }))
            .border_1()
            .border_color(rgb(if focused { 0x5A544B } else { LINE }))
            .bg(rgb(0x141210))
            .flex()
            .items_center()
            .gap_2()
            .cursor_text()
            .children(self.icon.then(crate::search_icon))
            .child({
                let pulse = self.pulse;
                let caret = move || {
                    div().flex_none().w(u(2.)).h(u(18.)).bg(rgb(ACCENT)).with_animation(("caret", pulse), Animation::new(Duration::from_millis(1000)).repeat(), |d, t| d.opacity(if t < 0.5 { 1. } else { 0. }))
                };
                let empty = self.text.is_empty();
                let (shaped, text) = (self.shaped.clone(), SharedString::from(self.text.clone()));
                let measure = canvas(|_, _, _| (), move |bounds, _, window, _| {
                    if text.is_empty() {
                        *shaped.borrow_mut() = None;
                        return;
                    }
                    let style = window.text_style();
                    let run = TextRun { len: text.len(), font: style.font(), color: style.color, background_color: None, underline: None, strikethrough: None };
                    let line = window.text_system().shape_line(text, style.font_size.to_pixels(window.rem_size()), &[run], None);
                    *shaped.borrow_mut() = Some((bounds.origin.x, line));
                })
                .absolute()
                .size_full();
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .child(measure)
                    .children((focused && empty).then(caret))
                    .children(empty.then(|| div().flex_none().text_color(rgb(FAINT)).child(self.placeholder.clone())))
                    .children((!empty).then(|| div().flex_none().text_color(rgb(TEXT)).child(before)))
                    .children((focused && !empty).then(caret))
                    .children((!empty).then(|| div().flex_none().text_color(rgb(TEXT)).child(after)))
            })
    }
}
