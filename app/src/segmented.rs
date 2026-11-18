//! A row of options where the highlight slides to the one you pick. Its owner keeps it and draws it.

use crate::{BG, LINE, MUTED, TEXT, u};
use gpui::{
    Animation, AnimationExt, App, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px, rgb,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

pub struct Segmented {
    labels: Vec<SharedString>,
    pub selected: usize,
    /// Each option's left edge and width, measured after layout.
    spots: Rc<RefCell<Vec<(f32, f32)>>>,
    /// Where the highlight starts its slide.
    from: Option<(f32, f32)>,
    /// Bumped on every change so the slide starts over.
    moves: usize,
}

impl Segmented {
    pub fn new(labels: &[&str], selected: usize) -> Self {
        Segmented { labels: labels.iter().map(|l| SharedString::from(l.to_string())).collect(), selected, spots: Default::default(), from: None, moves: 0 }
    }

    /// For owners whose options or choice can change from elsewhere.
    pub fn set(&mut self, labels: &[&str], selected: usize) {
        if self.labels.len() != labels.len() || self.labels.iter().zip(labels).any(|(a, b)| a != b) {
            self.labels = labels.iter().map(|l| SharedString::from(l.to_string())).collect();
            self.spots.borrow_mut().clear();
            self.from = None;
        }
        self.pick(selected);
    }

    pub fn pick(&mut self, i: usize) {
        if i != self.selected {
            self.from = self.spots.borrow().get(self.selected).copied();
            self.selected = i;
            self.moves += 1;
        }
    }

    /// `on_pick` gets the option's index, and should call `pick` with it.
    pub fn render(&self, id: &'static str, on_pick: impl Fn(&usize, &mut Window, &mut App) + 'static) -> impl IntoElement {
        let on_pick = Rc::new(on_pick);
        let spots = self.spots.borrow().clone();
        let measured = spots.len() == self.labels.len();
        let indicator = measured.then(|| {
            let to = spots[self.selected.min(spots.len() - 1)];
            let from = self.from.unwrap_or(to);
            div().absolute().top_0().bottom_0().rounded(u(16.)).bg(rgb(TEXT)).with_animation(
                (id, self.moves),
                Animation::new(Duration::from_millis(200)).with_easing(gpui::ease_out_quint()),
                move |d, t| d.left(px(from.0 + (to.0 - from.0) * t)).w(px(from.1 + (to.1 - from.1) * t)),
            )
        });
        // Where the highlight is at time t, for colouring the labels it passes under.
        let slide = measured.then(|| {
            let to = spots[self.selected.min(spots.len() - 1)];
            let from = self.from.unwrap_or(to);
            move |t: f32| (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)
        });
        let items = self.labels.iter().enumerate().map(|(i, label)| {
            let on = i == self.selected;
            let on_pick = on_pick.clone();
            let item = div()
                .id((id, i))
                .h(u(32.))
                .px(u(14.))
                .rounded(u(16.))
                .flex()
                .items_center()
                .flex_none()
                .text_size(u(13.))
                .cursor_pointer()
                .when(on, |d| d.font_weight(FontWeight::SEMIBOLD))
                // Until the first layout is measured, the chosen option paints its own highlight.
                .when(on && !measured, |d| d.bg(rgb(TEXT)).text_color(rgb(BG)))
                .when(!on, |d| d.text_color(rgb(MUTED)).hover(|s| s.text_color(rgb(TEXT))))
                .child(label.clone())
                .when(!on, |d| d.on_click(move |_, window, cx| on_pick(&i, window, cx)));
            match (slide, spots.get(i).copied()) {
                (Some(slide), Some((x, w))) => item
                    .with_animation((id, i + 1000 * self.moves), Animation::new(Duration::from_millis(200)).with_easing(gpui::ease_out_quint()), move |d, t| {
                        let (left, width) = slide(t);
                        let middle = x + w / 2.;
                        if middle > left && middle < left + width { d.text_color(rgb(BG)) } else { d.text_color(rgb(if on { TEXT } else { MUTED })) }
                    })
                    .into_any_element(),
                _ => item.into_any_element(),
            }
        });
        let store = self.spots.clone();
        let has_indicator = indicator.is_some();
        div().flex_none().p_1().rounded(u(20.)).border_1().border_color(rgb(LINE)).child(
            div()
                .relative()
                .flex()
                .gap_1()
                .children(indicator)
                .children(items)
                .on_children_prepainted(move |bounds, window, _| {
                    let items = if has_indicator { &bounds[1..] } else { &bounds[..] };
                    let Some(first) = items.first() else { return };
                    let left = f32::from(first.origin.x);
                    let now: Vec<(f32, f32)> = items.iter().map(|b| (f32::from(b.origin.x) - left, f32::from(b.size.width))).collect();
                    // Redraw only when something moved, or this would loop.
                    if *store.borrow() != now {
                        *store.borrow_mut() = now;
                        window.refresh();
                    }
                }),
        )
    }
}
