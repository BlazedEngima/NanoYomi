//! One source's popular, latest or search results as a cover grid that only builds the rows on screen.

use crate::u;
use crate::catalog::small_button;
use crate::covers::{CARD_H, CARD_W, Covers, Fetch, cover, grid};
use crate::filters::{self, Path};
use crate::images::ImageCache;
use crate::input::{InputEvent, TextInput};
use crate::registry::Registry;
use crate::store::Store;
use crate::{BG, HOVER, LINE, MUTED, SURFACE, Source, TEXT, heading};
use gpui::{
    AppContext,
    Context, Entity, EventEmitter, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, rgb, uniform_list,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::ops::Range;

const GAP: f32 = 24.;
const PAD: f32 = 40.;

pub struct OpenManga {
    pub source: Source,
    pub manga: Value,
}

#[derive(Clone, Copy, PartialEq)]
enum Listing {
    Popular,
    Latest,
    Search,
}

pub struct Browse {
    registry: Entity<Registry>,
    store: Entity<Store>,
    cache: Entity<ImageCache>,
    pub search: Entity<TextInput>,
    source: Option<Source>,
    listing: Listing,
    query: String,
    mangas: Vec<Value>,
    covers: Covers,
    page: u32,
    has_next: bool,
    loading: bool,
    /// Bumped on a source or listing change so stale pages get dropped.
    generation: u64,
    status: SharedString,
    card_w: f32,
    /// The source's filters, their starting values, and the text fields among them.
    filters: Vec<Value>,
    filter_defaults: Vec<Value>,
    filter_inputs: HashMap<Path, Entity<TextInput>>,
    filters_open: bool,
    _subs: Vec<Subscription>,
}

impl EventEmitter<OpenManga> for Browse {}

impl Browse {
    pub fn new(registry: Entity<Registry>, store: Entity<Store>, cache: Entity<ImageCache>, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new("Search this source…", cx));
        let subs = vec![
            cx.subscribe(&search, |this, input, _: &InputEvent, cx| {
                this.query = input.read(cx).text.trim().to_string();
                let listing = if this.query.is_empty() { Listing::Popular } else { Listing::Search };
                this.show(listing, cx);
            }),
            // Pick a source once extensions load, or when the current one disappears.
            cx.observe(&registry, |this, _, cx| this.check_source(cx)),
            cx.observe(&store, |this, _, cx| this.check_source(cx)),
        ];
        Browse {
            registry,
            store,
            cache,
            search,
            source: None,
            listing: Listing::Popular,
            query: String::new(),
            mangas: vec![],
            covers: Covers::default(),
            page: 0,
            has_next: false,
            loading: false,
            generation: 0,
            status: "".into(),
            card_w: CARD_W,
            filters: vec![],
            filter_defaults: vec![],
            filter_inputs: HashMap::new(),
            filters_open: false,
            _subs: subs,
        }
    }

    fn check_source(&mut self, cx: &mut Context<Self>) {
        let r = self.registry.read(cx);
        let list = r.browsable(self.store.read(cx));
        let gone = self.source.as_ref().is_some_and(|s| !list.iter().any(|b| b.id == s.id));
        if gone || self.source.is_none() {
            // NANO_DEMO_SOURCE=<name> starts on that source (for testing).
            let wanted = std::env::var("NANO_DEMO_SOURCE").ok();
            let first = list
                .iter()
                .find(|s| wanted.as_deref() == Some(s.name.as_str()))
                .or(list.iter().find(|s| s.lang == "en"))
                .or(list.first())
                .and_then(|s| r.source(&s.id));
            if wanted.is_some() && first.as_ref().is_some_and(|f| Some(f.name.as_str()) != wanted.as_deref()) && !r.ready {
                return; // wait until every extension has loaded
            }
            if first.is_some() || gone {
                self.source = first;
                self.load_filters(cx);
                self.show(Listing::Popular, cx);
            }
        }
        cx.notify();
    }

    pub fn source_id(&self) -> Option<&str> {
        self.source.as_ref().map(|s| s.id.as_str())
    }

    pub fn select(&mut self, id: &str, cx: &mut Context<Self>) {
        self.source = self.registry.read(cx).source(id);
        self.load_filters(cx);
        let listing = if self.listing == Listing::Search && self.query.is_empty() { Listing::Popular } else { self.listing };
        self.show(listing, cx);
    }

    /// Searches before the filters arrive just go out unfiltered.
    fn load_filters(&mut self, cx: &mut Context<Self>) {
        self.filters.clear();
        self.filter_defaults.clear();
        self.filter_inputs.clear();
        self.filters_open = false;
        let Some(source) = self.source.clone() else { return };
        cx.spawn(async move |this, cx| {
            let result = source.call(cx, "filters", json!({})).await;
            let _ = this.update(cx, |this, cx| {
                if this.source.as_ref().map(|s| &s.id) != Some(&source.id) {
                    return; // switched source meanwhile
                }
                match result {
                    Ok(Value::Array(list)) => {
                        this.filter_inputs = filters::text_inputs(&list, cx);
                        this.filter_defaults = list.clone();
                        this.filters = list;
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("filters for {}: {e}", source.name),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn set_filter(&mut self, p: Path, value: usize, cx: &mut Context<Self>) {
        if let Some(f) = filters::get_mut(&mut self.filters, p) {
            f["state"] = filters::next_state(f, value);
            cx.notify();
        }
    }

    fn sync_filter_texts(&mut self, cx: &mut Context<Self>) {
        for (p, input) in &self.filter_inputs {
            let text = input.read(cx).text.clone();
            if let Some(f) = filters::get_mut(&mut self.filters, *p) {
                f["state"] = json!(text);
            }
        }
    }

    fn apply_filters(&mut self, cx: &mut Context<Self>) {
        self.filters_open = false;
        self.show(Listing::Search, cx);
    }

    fn reset_filters(&mut self, cx: &mut Context<Self>) {
        self.filters = self.filter_defaults.clone();
        self.filter_inputs = filters::text_inputs(&self.filters, cx);
        cx.notify();
    }

    fn show(&mut self, listing: Listing, cx: &mut Context<Self>) {
        if listing == Listing::Search {
            self.sync_filter_texts(cx);
        }
        self.listing = listing;
        self.generation += 1;
        self.mangas.clear();
        self.page = 0;
        self.has_next = false;
        self.loading = false;
        self.load_next_page(cx);
    }

    fn load_next_page(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else { return };
        if self.loading || (self.page > 0 && !self.has_next) {
            return;
        }
        self.loading = true;
        self.status = "loading…".into();
        cx.notify();
        let page = self.page + 1;
        let generation = self.generation;
        let (op, params) = match self.listing {
            Listing::Popular => ("popular", json!({ "page": page })),
            Listing::Latest => ("latest", json!({ "page": page })),
            Listing::Search if self.filters.is_empty() => ("search", json!({ "page": page, "query": self.query })),
            Listing::Search => ("search", json!({ "page": page, "query": self.query, "filters": self.filters })),
        };
        cx.spawn(async move |this, cx| {
            let result = source.call(cx, op, params).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(v) => {
                        this.page = page;
                        this.has_next = v["hasNextPage"].as_bool().unwrap_or(false);
                        this.mangas.extend(v["mangas"].as_array().into_iter().flatten().cloned());
                        this.status = if this.mangas.is_empty() { "nothing found".into() } else { "".into() };
                        if let (Some(n), Some(source)) = (crate::demo(), this.source.clone())
                            && page == 1
                            && let Some(manga) = this.mangas.get(n).cloned()
                        {
                            cx.emit(OpenManga { source, manga });
                        }
                    }
                    Err(e) => this.status = e.into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn card(&mut self, i: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let title = self.mangas[i]["title"].as_str().unwrap_or("Untitled").to_string();
        let url = self.mangas[i]["thumbnailUrl"].as_str().map(str::to_string);
        let image = self.covers.get(&self.cache, self.source.clone().map(Fetch::Cover), url.as_deref(), |b: &mut Browse| &mut b.covers, cx);
        div()
            .id(("card", i))
            .w(u(self.card_w))
            .flex()
            .flex_col()
            .gap_2()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(source) = this.source.clone() {
                    cx.emit(OpenManga { source, manga: this.mangas[i].clone() });
                }
            }))
            .child(cover(image, &title, self.card_w, self.card_w * CARD_H / CARD_W, 0))
            .child(div().text_size(u(13.5)).font_weight(FontWeight::MEDIUM).truncate().child(title))
    }
}

pub(crate) fn segment(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, on: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(u(32.))
        .px(u(14.))
        .rounded(u(16.))
        .flex()
        .items_center()
        .flex_none()
        .text_size(u(13.))
        .cursor_pointer()
        .when(on, |d| d.bg(rgb(TEXT)).text_color(rgb(BG)).font_weight(FontWeight::SEMIBOLD))
        .when(!on, |d| d.text_color(rgb(MUTED)).hover(|s| s.bg(rgb(HOVER))))
        .child(label.into())
}

impl Render for Browse {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = crate::content_width(window);
        let (cols, card_w) = grid(width, PAD, GAP, CARD_W);
        self.card_w = card_w;
        let rows = self.mangas.len().div_ceil(cols);
        let registry = self.registry.read(cx);
        let selected = self.source.as_ref().map(|s| s.id.clone());
        let title = self.source.as_ref().map_or("Browse".to_string(), |s| s.name.clone());
        let note: SharedString = if !registry.ready {
            registry.status.clone()
        } else if self.source.is_none() {
            "no sources installed — add some in Extensions".into()
        } else if !self.status.is_empty() {
            self.status.clone()
        } else {
            format!("{} series", self.mangas.len()).into()
        };
        let listing = self.listing;
        let supports_latest = registry.sources.iter().find(|s| Some(&s.id) == selected.as_ref()).is_none_or(|s| s.supports_latest);

        let drawer = self.filters_open.then(|| {
            let body = filters::list(&self.filters, &self.filter_inputs, cx);
            div()
                .id("filters-panel")
                .occlude()
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .w(u(400.))
                .bg(rgb(SURFACE))
                .border_l_1()
                .border_color(rgb(LINE))
                .flex()
                .flex_col()
                .child(div().px(u(24.)).pt(u(24.)).pb(u(12.)).flex().items_center().child(div().flex_1().text_size(u(17.)).font_weight(FontWeight::SEMIBOLD).child("Filters")).child(
                    small_button("filters-close", "Close", false).on_click(cx.listener(|this, _, _, cx| {
                        this.filters_open = false;
                        cx.notify();
                    })),
                ))
                .child(div().id("filters-scroll").flex_1().min_h_0().overflow_y_scroll().px(u(24.)).pb(u(24.)).child(body))
                .child(
                    div()
                        .px(u(24.))
                        .py(u(16.))
                        .border_t_1()
                        .border_color(rgb(LINE))
                        .flex()
                        .justify_end()
                        .gap(u(10.))
                        .child(small_button("filters-reset", "Reset", false).on_click(cx.listener(|this, _, _, cx| this.reset_filters(cx))))
                        .child(small_button("filters-apply", "Apply", true).on_click(cx.listener(|this, _, _, cx| this.apply_filters(cx)))),
                )
        });

        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(
                div()
                    .px(u(PAD))
                    .pt(u(28.))
                    .flex()
                    .flex_wrap()
                    .items_end()
                    .gap_4()
                    .child(div().flex_none().max_w_full().child(heading(&title, note)))
                    .child(
                        div()
                            .ml_auto()
                            .flex()
                            .gap_1()
                            .p_1()
                            .rounded(u(20.))
                            .border_1()
                            .border_color(rgb(LINE))
                            .child(segment("popular", "Popular", listing == Listing::Popular).on_click(cx.listener(|this, _, _, cx| this.show(Listing::Popular, cx))))
                            .when(supports_latest, |d| {
                                d.child(segment("latest", "Latest", listing == Listing::Latest).on_click(cx.listener(|this, _, _, cx| this.show(Listing::Latest, cx))))
                            })
                            .when(listing == Listing::Search, |d| d.child(segment("search", "Search", true)))
                            .when(!self.filters.is_empty(), |d| {
                                let n = filters::changed(&self.filters, &self.filter_defaults);
                                let label = if n > 0 { format!("Filters · {n}") } else { "Filters".into() };
                                d.child(segment("filters", label, self.filters_open).on_click(cx.listener(|this, _, _, cx| {
                                    this.filters_open = !this.filters_open;
                                    cx.notify();
                                })))
                            }),
                    )
                    .child(div().w(u(300.)).max_w_full().child(self.search.clone())),
            )
            .child(
                uniform_list(
                    "grid",
                    rows,
                    cx.processor(move |this, range: Range<usize>, window, cx| {
                        if range.end + 2 >= rows {
                            cx.defer_in(window, |this, _, cx| this.load_next_page(cx));
                        }
                        range
                            .map(|row| {
                                let start = row * cols;
                                let end = (start + cols).min(this.mangas.len());
                                let cards: Vec<_> = (start..end).map(|i| this.card(i, cx)).collect();
                                div().flex().gap(u(GAP)).px(u(PAD)).pt(u(GAP)).children(cards)
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1(),
            )
            .children(drawer)
    }
}
