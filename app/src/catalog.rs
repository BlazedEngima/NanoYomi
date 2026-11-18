//! The Extensions screen: what's installed, then the keiyoushi repo, filterable by name and language.

use crate::u;
use crate::covers::{Covers, Fetch, grid};
use crate::extensions::Ext;
use crate::images::ImageCache;
use crate::input::{InputEvent, TextInput};
use crate::registry::Registry;
use crate::store::Store;
use crate::{ACCENT_TEXT, BG, FAINT, HOVER, LINE, MONO, MUTED, RULE, SURFACE, TEXT, heading};
use gpui::{
    AnyElement, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ListAlignment, ListState,
    ObjectFit, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, StyledImage, Subscription,
    Window, div, img, list, prelude::FluentBuilder, rgb,
};
use std::collections::HashMap;

const PAD: f32 = 40.;
const GAP: f32 = 16.;
const CARD_MIN: f32 = 300.;

#[derive(Clone, Copy)]
enum Item {
    Label(&'static str),
    Installed(usize),
    Row(usize),
    Empty,
}

pub struct Catalog {
    registry: Entity<Registry>,
    store: Entity<Store>,
    cache: Entity<ImageCache>,
    search: Entity<TextInput>,
    icons: Covers,
    list: ListState,
    /// The list caches row heights, so re-measure after a zoom change.
    zoom_seen: f32,
    items: Vec<Item>,
    installed: Vec<Ext>,
    available: Vec<Ext>,
    cols: usize,
    card_w: f32,
    lang: Option<String>,
    nsfw: bool,
    _subs: Vec<Subscription>,
}

impl Catalog {
    pub fn new(registry: Entity<Registry>, store: Entity<Store>, cache: Entity<ImageCache>, cx: &mut Context<Self>) -> Self {
        registry.update(cx, |r, cx| r.ensure_index(cx));
        let search = cx.new(|cx| TextInput::new("Search extensions…", cx));
        let subs = vec![
            cx.observe(&registry, |_, _, cx| cx.notify()),
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify()),
        ];
        Catalog {
            registry,
            store,
            cache,
            search,
            icons: Covers::default(),
            zoom_seen: 0.,
            list: ListState::new(0, ListAlignment::Top, u(600.)),
            items: vec![],
            installed: vec![],
            available: vec![],
            cols: 1,
            card_w: CARD_MIN,
            lang: None,
            nsfw: false,
            _subs: subs,
        }
    }

    fn icon(&mut self, e: &Ext, size: f32, cx: &mut Context<Self>) -> AnyElement {
        let url = Some(e.icon_url.as_str()).filter(|u| !u.is_empty());
        let image = self.icons.get(&self.cache, Some(Fetch::Icon), url, |c: &mut Catalog| &mut c.icons, cx);
        div()
            .size(u(size))
            .flex_none()
            .rounded(u(size / 4.))
            .overflow_hidden()
            .bg(rgb(SURFACE))
            .flex()
            .items_center()
            .justify_center()
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(MUTED))
            .map(|d| match image {
                Some(image) => d.child(img(image).size_full().object_fit(ObjectFit::Cover)),
                None => d.child(e.name.chars().next().unwrap_or('?').to_uppercase().to_string()),
            })
            .into_any_element()
    }

    fn installed_row(&mut self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let e = self.installed[i].clone();
        let icon = self.icon(&e, 44., cx);
        let registry = self.registry.read(cx);
        let langs = registry.languages(&e.pkg);
        let enabled = registry.enabled_languages(&e.pkg, self.store.read(cx));
        let multi = langs.len() > 1;
        // Drop the stack trace the host adds.
        let error = registry.errors.get(&e.pkg).map(|err| format!("Couldn't load: {}", err.lines().next().unwrap_or(err)));
        let pkg = e.pkg.clone();
        let chips: Vec<_> = langs
            .into_iter()
            .map(|l| {
                let on = enabled.contains(&l);
                let (pkg, lang, enabled) = (pkg.clone(), l.clone(), enabled.clone());
                chip(SharedString::from(format!("lang-{pkg}-{l}")), l, on).on_click(cx.listener(move |this, _, _, cx| {
                    let mut langs = enabled.clone();
                    if langs.contains(&lang) {
                        langs.retain(|x| *x != lang);
                    } else {
                        langs.push(lang.clone());
                    }
                    // Keep at least one language or the extension disappears from Browse.
                    if !langs.is_empty() {
                        let pkg = pkg.clone();
                        this.store.update(cx, |s, cx| s.set_languages(&pkg, langs, cx));
                    }
                }))
            })
            .collect();
        let remove = small_button(SharedString::from(format!("rm-{}", e.pkg)), "Remove", false).on_click(cx.listener(
            move |this, _, _, cx| {
                let pkg = pkg.clone();
                this.registry.update(cx, |r, cx| r.uninstall(&pkg, cx))
            },
        ));
        div()
            .w_full()
            .px(u(PAD))
            .child(
                div()
                    .w_full()
                    .py(u(14.))
                    .flex()
                    .gap_4()
                    .border_b_1()
                    .border_color(rgb(RULE))
                    .child(icon)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(u(8.))
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap_3()
                                    .child(div().font_weight(FontWeight::SEMIBOLD).child(e.name.clone()))
                                    .child(
                                        div()
                                            .font_family(MONO)
                                            .text_size(u(12.))
                                            .text_color(rgb(FAINT))
                                            .child(if multi { format!("v{} · languages shown in Browse:", e.version) } else { format!("{} · v{}", e.langs.join(" "), e.version) }),
                                    ),
                            )
                            .when(multi, |d| d.child(div().flex().flex_wrap().gap(u(6.)).children(chips)))
                            .children(error.map(|err| div().text_size(u(13.)).text_color(rgb(ACCENT_TEXT)).child(err))),
                    )
                    .child(remove),
            )
            .into_any_element()
    }

    fn card(&mut self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let e = self.available[i].clone();
        let icon = self.icon(&e, 40., cx);
        let busy = self.registry.read(cx).busy.contains(&e.pkg);
        let id = SharedString::from(format!("add-{}", e.pkg));
        let button = if busy {
            small_button(id, "Installing…", false).opacity(0.6)
        } else {
            let ext = e.clone();
            small_button(id, "Install", true).on_click(cx.listener(move |this, _, _, cx| this.registry.update(cx, |r, cx| r.install(ext.clone(), cx))))
        };
        div()
            .w(u(self.card_w))
            .p(u(12.))
            .flex()
            .items_center()
            .gap_3()
            .rounded(u(10.))
            .border_1()
            .border_color(rgb(RULE))
            .child(icon)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(u(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().min_w_0().truncate().font_weight(FontWeight::SEMIBOLD).child(e.name.clone()))
                            .when(e.nsfw, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .px(u(5.))
                                        .rounded(u(4.))
                                        .border_1()
                                        .border_color(rgb(LINE))
                                        .font_family(MONO)
                                        .text_size(u(10.))
                                        .text_color(rgb(MUTED))
                                        .child("18+"),
                                )
                            }),
                    )
                    .child(div().truncate().font_family(MONO).text_size(u(12.)).text_color(rgb(FAINT)).child(format!("{} · v{}", e.langs.join(" "), e.version))),
            )
            .child(button)
            .into_any_element()
    }

    fn item(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        match self.items.get(ix).copied() {
            Some(Item::Label(text)) => {
                let text = if text == "AVAILABLE" { format!("AVAILABLE · {}", self.available.len()) } else { text.to_string() };
                div().px(u(PAD)).pt(u(24.)).pb(u(10.)).font_family(MONO).text_size(u(11.)).text_color(rgb(FAINT)).child(text).into_any_element()
            }
            Some(Item::Installed(i)) => self.installed_row(i, cx),
            Some(Item::Row(r)) => {
                let cards: Vec<AnyElement> =
                    (r * self.cols..((r + 1) * self.cols).min(self.available.len())).map(|i| self.card(i, cx)).collect();
                div().flex().gap(u(GAP)).px(u(PAD)).pb(u(GAP)).children(cards).into_any_element()
            }
            Some(Item::Empty) => div().px(u(PAD)).py_4().text_color(rgb(MUTED)).child("No extensions match.").into_any_element(),
            None => div().into_any_element(),
        }
    }
}

pub fn chip(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, on: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .h(u(26.))
        .px(u(10.))
        .rounded(u(13.))
        .flex()
        .items_center()
        .border_1()
        .font_family(MONO)
        .text_size(u(12.))
        .cursor_pointer()
        .when(on, |d| d.bg(rgb(TEXT)).border_color(rgb(TEXT)).text_color(rgb(BG)))
        .when(!on, |d| d.border_color(rgb(LINE)).text_color(rgb(MUTED)).hover(|s| s.bg(rgb(HOVER))))
        .child(label.into())
}

pub fn small_button(id: impl Into<gpui::ElementId>, label: &'static str, primary: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .h(u(30.))
        .px(u(14.))
        .rounded(u(15.))
        .flex()
        .items_center()
        .text_size(u(13.))
        .cursor_pointer()
        .border_1()
        .border_color(rgb(LINE))
        .text_color(rgb(if primary { ACCENT_TEXT } else { MUTED }))
        .font_weight(if primary { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
        .hover(|s| s.bg(rgb(HOVER)))
        .child(label)
}

impl Render for Catalog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.zoom_seen != crate::zoom() {
            self.zoom_seen = crate::zoom();
            self.list.remeasure();
        }
        let query = self.search.read(cx).text.trim().to_lowercase();
        let registry = self.registry.read(cx);
        let index = registry.index.clone().unwrap_or_default();
        let (lang, nsfw) = (self.lang.clone(), self.nsfw);
        let matches = |e: &Ext| {
            (query.is_empty() || e.name.to_lowercase().contains(&query))
                && lang.as_ref().is_none_or(|l| e.langs.iter().any(|x| x == l || x == "all"))
                && (nsfw || !e.nsfw)
        };
        self.installed = index.iter().filter(|e| registry.installed.contains(&e.pkg)).cloned().collect();
        self.available = index.iter().filter(|e| !registry.installed.contains(&e.pkg) && matches(e)).cloned().collect();
        (self.cols, self.card_w) = grid(crate::content_width(window), PAD, GAP, CARD_MIN);

        let mut counts: HashMap<&str, usize> = HashMap::new();
        for l in index.iter().flat_map(|e| e.langs.iter()).filter(|l| *l != "all") {
            *counts.entry(l.as_str()).or_default() += 1;
        }
        let mut top: Vec<(&str, usize)> = counts.into_iter().collect();
        top.sort_by_key(|(l, n)| (std::cmp::Reverse(*n), *l));
        let top: Vec<String> = top.into_iter().take(12).map(|(l, _)| l.to_string()).collect();

        let note = if index.is_empty() {
            if registry.status.is_empty() { "loading the repo…".to_string() } else { registry.status.to_string() }
        } else {
            format!("{} installed · {} in keiyoushi", self.installed.len(), index.len())
        };

        let mut items = vec![];
        if !self.installed.is_empty() {
            items.push(Item::Label("INSTALLED"));
            items.extend((0..self.installed.len()).map(Item::Installed));
        }
        items.push(Item::Label("AVAILABLE"));
        if self.available.is_empty() {
            items.push(Item::Empty);
        }
        items.extend((0..self.available.len().div_ceil(self.cols)).map(Item::Row));
        if items.len() != self.list.item_count() {
            self.list.reset(items.len());
        }
        self.items = items;
        let this = cx.entity();

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .px(u(PAD))
                    .pt(u(28.))
                    .flex()
                    .items_end()
                    .gap_4()
                    .child(div().flex_1().child(heading("Extensions", note)))
                    .child(div().w(u(320.)).child(self.search.clone())),
            )
            .child(
                div()
                    .px(u(PAD))
                    .pt(u(16.))
                    .flex()
                    .flex_wrap()
                    .gap(u(6.))
                    .child(chip("lang-any", "all languages", lang.is_none()).on_click(cx.listener(|this, _, _, cx| {
                        this.lang = None;
                        cx.notify();
                    })))
                    .children(top.into_iter().map(|l| {
                        let on = lang.as_deref() == Some(l.as_str());
                        chip(SharedString::from(format!("filter-{l}")), l.clone(), on).on_click(cx.listener(move |this, _, _, cx| {
                            this.lang = if on { None } else { Some(l.clone()) };
                            cx.notify();
                        }))
                    }))
                    .child(div().w(u(12.)))
                    .child(chip("nsfw", "show 18+", nsfw).on_click(cx.listener(|this, _, _, cx| {
                        this.nsfw = !this.nsfw;
                        cx.notify();
                    }))),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(list(self.list.clone(), move |ix, _, cx| this.update(cx, |this, cx| this.item(ix, cx))).size_full()),
            )
    }
}
