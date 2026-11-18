//! The library screen: what you're reading, your saved series, and their new chapters.

use crate::u;
use crate::browse::OpenManga;
use crate::covers::{CARD_H, CARD_W, Covers, Fetch, cover, grid};
use crate::images::ImageCache;
use crate::registry::Registry;
use crate::store::{Series, Store, ago, now_ms};
use crate::{ACCENT, ACCENT_TEXT, DISPLAY, FAINT, LINE, MONO, MUTED, RULE, SURFACE, Source, TEXT, heading, pill};
use gpui::{
    AnyElement, Context, Entity, EventEmitter, FontWeight, InteractiveElement, IntoElement, ListAlignment, ListState,
    ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, Window, div, list,
    prelude::FluentBuilder, relative, rgb,
};
use serde_json::{Value, json};
use std::sync::Arc;

const PAD: f32 = 40.;
const GAP: f32 = 20.;
const UPDATES: usize = 10;

pub struct Resume {
    pub source: Source,
    pub manga: Value,
    pub chapters: Arc<Vec<Value>>,
    pub index: usize,
    pub page: usize,
}

pub struct GoBrowse;

#[derive(Clone, Copy)]
enum Item {
    Hero,
    Heading,
    Empty,
    Row(usize),
    UpdatesHeading,
    UpdatesHeader,
    Update(usize),
}

pub struct Library {
    store: Entity<Store>,
    registry: Entity<Registry>,
    cache: Entity<ImageCache>,
    covers: Covers,
    list: ListState,
    /// The list caches row heights, so re-measure after a zoom change.
    zoom_seen: f32,
    items: Vec<Item>,
    cols: usize,
    card_w: f32,
    /// This frame's library, plus the (series, chapter) pairs for Latest Updates.
    /// The library first, then the series being read if it isn't saved, for the hero.
    series: Vec<Series>,
    library_len: usize,
    updates: Vec<(usize, usize)>,
    checking: Option<(usize, usize)>,
    checked: bool,
    _subs: Vec<Subscription>,
}

impl EventEmitter<OpenManga> for Library {}
impl EventEmitter<Resume> for Library {}
impl EventEmitter<GoBrowse> for Library {}

impl Library {
    pub fn new(store: Entity<Store>, registry: Entity<Registry>, cache: Entity<ImageCache>, cx: &mut Context<Self>) -> Self {
        let subs = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.observe(&registry, |this, registry, cx| {
                if registry.read(cx).ready && !this.checked {
                    this.checked = true;
                    this.check_for_updates(cx);
                }
                cx.notify();
            }),
        ];
        Library {
            store,
            registry,
            cache,
            covers: Covers::default(),
            zoom_seen: 0.,
            list: ListState::new(0, ListAlignment::Top, u(800.)),
            items: vec![],
            cols: 1,
            card_w: CARD_W,
            series: vec![],
            library_len: 0,
            updates: vec![],
            checking: None,
            checked: false,
            _subs: subs,
        }
    }

    // Checks once per launch. Might need a timer if Nano stays open for days.
    fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        let targets: Vec<(Source, Value)> = {
            let registry = self.registry.read(cx);
            self.store.read(cx).library().iter().filter_map(|s| Some((registry.source(&s.source)?, s.manga.clone()))).collect()
        };
        if targets.is_empty() {
            return;
        }
        let total = targets.len();
        cx.spawn(async move |this, cx| {
            for (done, (source, manga)) in targets.into_iter().enumerate() {
                let _ = this.update(cx, |this, cx| {
                    this.checking = Some((done, total));
                    cx.notify();
                });
                let result = source.call(cx, "update", json!({ "manga": manga, "chapters": [], "needsDetails": false })).await;
                let Ok(v) = result else { continue };
                let chapters = v["chapters"].as_array().cloned().unwrap_or_default();
                let _ = this.update(cx, |this, cx| {
                    this.store.update(cx, |store, cx| {
                        if store.get(&source.id, manga["url"].as_str().unwrap_or_default()).is_some() {
                            store.update(&source.id, &source.name, &manga, cx, |s| s.chapters = chapters);
                        }
                    });
                });
            }
            let _ = this.update(cx, |this, cx| {
                this.checking = None;
                cx.notify();
            });
        })
        .detach();
    }

    fn resume(&mut self, ix: usize, chapter: Option<usize>, cx: &mut Context<Self>) {
        let s = &self.series[ix];
        let source = self.registry.read(cx).source_or_saved(&s.source, &s.source_name);
        let (index, page) = match (chapter, s.reading_index()) {
            (Some(c), _) => (c, 0),
            (None, Some(c)) => (c, s.progress.as_ref().map_or(0, |p| p.page)),
            (None, None) => (s.chapters.len().saturating_sub(1), 0),
        };
        // No extension means nothing to read, so open the series page where it can be migrated.
        if s.chapters.is_empty() || !source.installed {
            return cx.emit(OpenManga { source, manga: s.manga.clone() });
        }
        cx.emit(Resume { source, manga: s.manga.clone(), chapters: Arc::new(s.chapters.clone()), index, page });
    }

    fn open(&mut self, ix: usize, cx: &mut Context<Self>) {
        let s = &self.series[ix];
        let source = self.registry.read(cx).source_or_saved(&s.source, &s.source_name);
        cx.emit(OpenManga { source, manga: s.manga.clone() });
    }

    fn cover_of(&mut self, ix: usize, cx: &mut Context<Self>) -> Option<Arc<gpui::RenderImage>> {
        let s = &self.series[ix];
        let source = self.registry.read(cx).source(&s.source);
        let url = s.manga["thumbnailUrl"].as_str().map(str::to_string);
        self.covers.get(&self.cache, source.map(Fetch::Cover), url.as_deref(), |l: &mut Library| &mut l.covers, cx)
    }

    fn item(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        match self.items.get(ix).copied() {
            Some(Item::Hero) => self.hero(cx),
            Some(Item::Heading) => {
                let unread: usize = self.series[..self.library_len].iter().map(Series::unread).sum();
                let note = match self.checking {
                    Some((done, total)) => format!("{} SERIES · CHECKING {done}/{total}", self.library_len),
                    None => format!("{} SERIES · {unread} UNREAD", self.library_len),
                };
                div()
                    .w_full()
                    .px(u(PAD))
                    .pt(u(8.))
                    .flex()
                    .items_end()
                    .child(div().flex_1().child(heading("Library", note)))
                    .child(div().pb(u(5.)).text_size(u(13.)).text_color(rgb(MUTED)).child("Sorted by last read"))
                    .into_any_element()
            }
            Some(Item::Empty) => div()
                .px(u(PAD))
                .pt(u(24.))
                .flex()
                .flex_col()
                .items_start()
                .gap_4()
                .child(div().text_color(rgb(MUTED)).child("Nothing saved yet. Open a series in Browse and add it to your library."))
                .child(pill("go-browse", "Browse sources", true).on_click(cx.listener(|_, _, _, cx| cx.emit(GoBrowse))))
                .into_any_element(),
            Some(Item::Row(row)) => {
                let start = row * self.cols;
                let end = (start + self.cols).min(self.library_len);
                let cards: Vec<AnyElement> = (start..end).map(|i| self.card(i, cx)).collect();
                div().flex().gap(u(GAP)).px(u(PAD)).pt(u(24.)).children(cards).into_any_element()
            }
            Some(Item::UpdatesHeading) => {
                div().px(u(PAD)).pt(u(48.)).child(heading("Latest Updates", "")).into_any_element()
            }
            Some(Item::UpdatesHeader) => div()
                .pt(u(14.))
                .child(mono_row(
                    ["WHEN", "SERIES", "CHAPTER", "SOURCE", "STATUS"].map(|t| div().child(t).into_any_element()),
                    FAINT,
                    11.,
                    LINE,
                ))
                .into_any_element(),
            Some(Item::Update(row)) => self.update_row(row, cx),
            None => div().into_any_element(),
        }
    }

    fn hero(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let history = self.store.read(cx).history().first().map(|s| (*s).clone());
        let Some(s) = history else { return div().into_any_element() };
        let p = s.progress.clone().unwrap_or_else(|| unreachable!());
        let Some(ix) = self.series.iter().position(|x| x.source == s.source && x.url() == s.url()) else {
            return div().into_any_element();
        };
        let image = self.cover_of(ix, cx);
        let newer = s.unread();
        let sub = match newer {
            0 => format!("{} · up to date", s.source_name),
            1 => format!("{} · 1 newer chapter", s.source_name),
            n => format!("{} · {n} newer chapters", s.source_name),
        };
        div()
            .w_full()
            .flex()
            .gap(u(48.))
            .items_center()
            .px(u(PAD))
            .pt(u(44.))
            .pb(u(40.))
            .child(div().flex_none().shadow_lg().child(cover(image, s.title(), 200., 300., 0)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(u(14.))
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(u(12.))
                            .text_color(rgb(ACCENT_TEXT))
                            .truncate()
                            .child(format!("NOW READING · {} · P.{} / {}", p.chapter_name.to_uppercase(), p.page + 1, p.pages)),
                    )
                    .child(
                        div()
                            .font_family(DISPLAY)
                            .font_weight(FontWeight::BLACK)
                            .text_size(u(72.))
                            .line_height(relative(0.88))
                            .max_w(u(900.))
                            .line_clamp(2)
                            .text_ellipsis()
                            .child(s.title().to_uppercase()),
                    )
                    .child(progress_bar(p.page + 1, p.pages))
                    .child(div().text_size(u(13.)).text_color(rgb(MUTED)).child(sub))
                    .child(
                        div()
                            .flex()
                            .gap(u(10.))
                            .mt_2()
                            .child(
                                pill("resume", "Resume", true)
                                    .child(div().font_family(MONO).text_size(u(12.)).opacity(0.6).child("⏎"))
                                    .on_click(cx.listener(move |this, _, _, cx| this.resume(ix, None, cx))),
                            )
                            .child(pill("all-chapters", "All chapters", false).on_click(cx.listener(move |this, _, _, cx| this.open(ix, cx)))),
                    ),
            )
            .into_any_element()
    }

    fn card(&mut self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let image = self.cover_of(i, cx);
        let s = &self.series[i];
        let title = s.title().to_string();
        div()
            .id(("lib", i))
            .w(u(self.card_w))
            .flex()
            .flex_col()
            .gap_2()
            .cursor_pointer()
            .child(card_cover(cover(image, &title, self.card_w, self.card_w * CARD_H / CARD_W, s.unread()), s.read_fraction(), s.caught_up()))
            .child(div().text_size(u(13.5)).font_weight(FontWeight::MEDIUM).truncate().child(title))
            .on_click(cx.listener(move |this, _, _, cx| this.open(i, cx)))
            .into_any_element()
    }

    fn update_row(&mut self, row: usize, cx: &mut Context<Self>) -> AnyElement {
        let (si, ci) = self.updates[row];
        let s = &self.series[si];
        let ch = &s.chapters[ci];
        let read = s.reading_index().is_some_and(|r| ci >= r);
        let (status, color) = if read { ("read", MUTED) } else { ("new", ACCENT_TEXT) };
        let when = ago(ch["dateUpload"].as_i64().unwrap_or(0), now_ms());
        let title = s.title().to_string();
        let image = self.cover_of(si, cx);
        let s = &self.series[si];
        let ch = &s.chapters[ci];
        let cells = [
            div().font_family(MONO).text_size(u(12.5)).text_color(rgb(MUTED)).child(when),
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(cover(image, "", 30., 30. * CARD_H / CARD_W, 0))
                .child(div().min_w_0().font_weight(FontWeight::SEMIBOLD).truncate().child(title)),
            div().text_color(rgb(0xC9C1B3)).truncate().child(ch["name"].as_str().unwrap_or("Chapter").to_string()),
            div().text_color(rgb(MUTED)).truncate().child(s.source_name.clone()),
            div().font_family(MONO).text_size(u(12.)).text_color(rgb(color)).child(status),
        ]
        .map(IntoElement::into_any_element);
        div()
            .id(("update", row))
            .cursor_pointer()
            .hover(|d| d.bg(rgb(SURFACE)))
            .child(mono_row(cells, TEXT, 14., RULE).py(u(10.)))
            .on_click(cx.listener(move |this, _, _, cx| this.resume(si, Some(ci), cx)))
            .into_any_element()
    }
}

fn mono_row(cells: [AnyElement; 5], color: u32, size: f32, border: u32) -> gpui::Div {
    let [when, series, chapter, source, status] = cells;
    div()
        .mx(u(PAD))
        .py(u(8.))
        .flex()
        .items_center()
        .gap_4()
        .border_b_1()
        .border_color(rgb(border))
        .text_color(rgb(color))
        .text_size(u(size))
        .when(size < 12., |d| d.font_family(MONO))
        .child(div().w(u(110.)).flex_none().child(when))
        .child(div().flex_1().min_w_0().child(series))
        .child(div().flex_1().min_w_0().child(chapter))
        .child(div().w(u(160.)).flex_none().min_w_0().child(source))
        .child(div().w(u(80.)).flex_none().flex().justify_end().child(status))
}

/// A cover with a READING or CAUGHT UP label and a thin progress line along the bottom.
fn card_cover(cover: AnyElement, read: Option<f32>, caught_up: bool) -> impl IntoElement {
    div().relative().child(cover).when_some(read, |d, f| {
        let label = if caught_up { "CAUGHT UP" } else { "READING" };
        d.when(caught_up, |d| d.child(div().absolute().inset_0().rounded(u(4.)).bg(gpui::rgba(0x0D0C0B99))))
            .child(
                div()
                    .absolute()
                    .top(u(8.))
                    .left(u(8.))
                    .h(u(24.))
                    .px(u(8.))
                    .rounded(u(12.))
                    .flex()
                    .items_center()
                    .bg(rgb(if caught_up { 0x0D0C0B } else { ACCENT }))
                    .font_family(MONO)
                    .text_size(u(10.5))
                    .text_color(rgb(if caught_up { TEXT } else { 0x0D0C0B }))
                    .child(label),
            )
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(u(3.))
                    .rounded_b(u(4.))
                    .bg(gpui::rgba(0x0D0C0B80))
                    .child(div().h_full().w(relative(f)).bg(rgb(ACCENT))),
            )
    })
}

fn progress_bar(read: usize, total: usize) -> impl IntoElement {
    let ticks = total.clamp(1, 48);
    div().flex().gap(u(2.)).h(u(18.)).max_w(u(520.)).children((0..ticks).map(move |i| {
        let done = (i + 1) * total.max(1) <= read * ticks;
        div().flex_1().h_full().bg(rgb(if done { ACCENT } else { 0x3A352F }))
    }))
}

impl Render for Library {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.zoom_seen != crate::zoom() {
            self.zoom_seen = crate::zoom();
            self.list.remeasure();
        }
        let width = f32::from(window.viewport_size().width);
        (self.cols, self.card_w) = grid(width, PAD, GAP, CARD_W);
        let store = self.store.read(cx);
        self.series = store.library().into_iter().cloned().map(|mut s| {
            // Chapters after the scanlator filter. Not saved.
            s.chapters = s.visible();
            s.hidden_scanlators.clear();
            s
        }).collect();
        self.library_len = self.series.len();
        let reading = store.history().into_iter().take(1).into_iter().cloned().map(|mut s| {
            s.chapters = s.visible();
            s.hidden_scanlators.clear();
            s
        }).next();
        let has_hero = reading.is_some();

        let mut updates: Vec<(usize, usize, i64)> = self
            .series
            .iter()
            .enumerate()
            .flat_map(|(si, s)| {
                s.chapters.iter().take(UPDATES).enumerate().filter_map(move |(ci, c)| Some((si, ci, c["dateUpload"].as_i64().filter(|d| *d > 0)?)))
            })
            .collect();
        updates.sort_by_key(|u| -u.2);
        self.updates = updates.into_iter().take(UPDATES).map(|(s, c, _)| (s, c)).collect();
        // Read from Browse without saving. It's still the hero, so keep it.
        if let Some(r) = reading
            && !self.series.iter().any(|x| x.source == r.source && x.url() == r.url())
        {
            self.series.push(r);
        }

        let mut items = vec![];
        if has_hero {
            items.push(Item::Hero);
        }
        items.push(Item::Heading);
        if self.library_len == 0 {
            items.push(Item::Empty);
        }
        items.extend((0..self.library_len.div_ceil(self.cols)).map(Item::Row));
        if !self.updates.is_empty() {
            items.extend([Item::UpdatesHeading, Item::UpdatesHeader]);
            items.extend((0..self.updates.len()).map(Item::Update));
        }
        if items.len() != self.list.item_count() {
            self.list.reset(items.len());
        }
        self.items = items;

        let this = cx.entity();
        div().size_full().pb(u(24.)).child(
            list(self.list.clone(), move |ix, _window, cx| this.update(cx, |this, cx| this.item(ix, cx))).size_full(),
        )
    }
}
