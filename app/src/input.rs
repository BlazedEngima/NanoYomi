//! A one-line text field, since GPUI doesn't come with one.
// No cursor movement, selection or IME yet. Just typing, backspace, paste and enter.

use crate::u;
use crate::{FAINT, LINE, TEXT, ACCENT};
use gpui::{
    ClickEvent, Context, EventEmitter, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, div, rgb,
};

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
}

impl EventEmitter<InputEvent> for TextInput {}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        TextInput { focus: cx.focus_handle(), text: String::new(), placeholder: placeholder.into(), icon: true, numeric: false, compact: false }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        match (k.key.as_str(), k.modifiers.platform || k.modifiers.control) {
            ("backspace", true) => self.text.clear(),
            ("backspace", false) => {
                self.text.pop();
            }
            ("enter", _) => cx.emit(InputEvent::Submit),
            ("escape", _) => self.text.clear(),
            ("v", true) => {
                if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    let numeric = self.numeric;
                    self.text.extend(t.lines().next().unwrap_or("").chars().filter(|c| !numeric || c.is_ascii_digit()));
                }
            }
            (_, false) => match &k.key_char {
                Some(ch) if !ch.chars().any(char::is_control) && (!self.numeric || ch.chars().all(|c| c.is_ascii_digit())) => {
                    let flip = cfg!(target_os = "macos") && window.capslock().on && ch.chars().count() == 1 && ch.chars().all(char::is_alphabetic);
                    if flip {
                        self.text.extend(ch.chars().flat_map(|c| if c.is_uppercase() { c.to_lowercase().collect::<Vec<_>>() } else { c.to_uppercase().collect() }));
                    } else {
                        self.text.push_str(ch);
                    }
                }
                _ => return,
            },
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let (text, color) = if self.text.is_empty() {
            (self.placeholder.clone(), FAINT)
        } else {
            (SharedString::from(self.text.clone()), TEXT)
        };
        div()
            .id("input")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
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
                let caret = || div().flex_none().w(u(2.)).h(u(18.)).bg(rgb(ACCENT));
                let empty = self.text.is_empty();
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .children((focused && empty).then(caret))
                    .child(div().flex_none().text_color(rgb(color)).child(text))
                    .children((focused && !empty).then(caret))
            })
    }
}
