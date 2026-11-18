mod browse;
mod byparr;
mod catalog;
mod challenge;
mod covers;
mod extensions;
mod filters;
mod history;
mod host;
mod images;
mod input;
mod library;
mod manga;
mod migrate;
mod page_cache;
mod reader;
mod registry;
mod rich;
mod settings;
mod store;
mod webviews;

use browse::{Browse, OpenManga};
use catalog::Catalog;
use history::History;
use library::{GoBrowse, Library, Resume};
use registry::Registry;
use store::Store;
use gpui::{
    AnyElement, App, AppContext, AsyncApp, Bounds, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, RenderImage, SharedString, StatefulInteractiveElement, Styled, Subscription,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, prelude::FluentBuilder, px, rgb, size,
};
use host::{Host, HostError};
use images::ImageCache;
use manga::{MangaEvent, MangaView};
use reader::{Reader, ReaderEvent};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// mimalloc hands freed decode buffers back to macOS. The system allocator sat on 341 MB.
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub const BG: u32 = 0x0D0C0B;
pub const SURFACE: u32 = 0x1A1816;
pub const HOVER: u32 = 0x24211D;
pub const BORDER: u32 = 0x24211D;
pub const RULE: u32 = 0x1E1B18;
pub const LINE: u32 = 0x3A352F;
pub const TEXT: u32 = 0xEDE6D8;
pub const BODY: u32 = 0xC9C1B3;
pub const MUTED: u32 = 0xA39C90;
pub const FAINT: u32 = 0x857E73;
pub const ACCENT: u32 = 0xFF4A2B;
pub const ACCENT_TEXT: u32 = 0xFF6A4D;

/// UI zoom. Every UI size goes through `u`. Reader pages use real pixels and don't zoom.
static ZOOM: AtomicU32 = AtomicU32::new(1f32.to_bits());

pub fn zoom() -> f32 {
    f32::from_bits(ZOOM.load(Ordering::Relaxed))
}

fn set_zoom(z: f32) {
    ZOOM.store(((z * 10.).round() / 10.).clamp(0.7, 2.0).to_bits(), Ordering::Relaxed);
}

pub fn u(v: f32) -> gpui::Pixels {
    px(v * zoom())
}

struct Assets;

impl gpui::AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        Ok(match path {
            "icons/search.svg" => Some(include_bytes!("../assets/icons/search.svg").as_slice().into()),
            _ => None,
        })
    }
    fn list(&self, _: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(vec![])
    }
}

pub fn search_icon() -> impl IntoElement {
    gpui::svg().path("icons/search.svg").flex_none().size(u(16.)).text_color(rgb(FAINT))
}

pub const DISPLAY: &str = "Big Shoulders Display";
pub const SANS: &str = "IBM Plex Sans";
pub const MONO: &str = "IBM Plex Mono";
/// System font for the few Japanese labels, so no CJK font is bundled.
pub const CJK: &str = "Hiragino Sans";

fn load_fonts(cx: &App) {
    let fonts = [
        &include_bytes!("../fonts/BigShouldersDisplay-ExtraBold.ttf")[..],
        include_bytes!("../fonts/BigShouldersDisplay-Black.ttf"),
        include_bytes!("../fonts/IBMPlexSans-Regular.ttf"),
        include_bytes!("../fonts/IBMPlexSans-Medium.ttf"),
        include_bytes!("../fonts/IBMPlexSans-SemiBold.ttf"),
        include_bytes!("../fonts/IBMPlexMono-Regular.ttf"),
        include_bytes!("../fonts/IBMPlexMono-Medium.ttf"),
    ];
    if let Err(e) = cx.text_system().add_fonts(fonts.into_iter().map(std::borrow::Cow::Borrowed).collect()) {
        eprintln!("fonts: {e}");
    }
}

pub fn heading(title: &str, note: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex()
        .items_end()
        .gap_4()
        .min_w_0()
        .child(div().flex_none().font_family(DISPLAY).font_weight(FontWeight::EXTRA_BOLD).text_size(u(28.)).child(title.to_uppercase()))
        .child(div().min_w_0().truncate().pb(u(5.)).font_family(MONO).text_size(u(13.)).text_color(rgb(FAINT)).child(note.into()))
}

pub fn pill(id: &'static str, label: impl Into<SharedString>, primary: bool) -> gpui::Stateful<gpui::Div> {
    let d = div()
        .id(id)
        .h(u(44.))
        .px(u(20.))
        .rounded(u(22.))
        .flex()
        .items_center()
        .gap_3()
        .cursor_pointer()
        .child(label.into());
    if primary {
        d.bg(rgb(ACCENT)).text_color(rgb(BG)).font_weight(FontWeight::SEMIBOLD).hover(|s| s.bg(rgb(ACCENT_TEXT)))
    } else {
        d.border_1().border_color(rgb(LINE)).hover(|s| s.bg(rgb(HOVER)))
    }
}

#[derive(Clone)]
pub struct Source {
    pub host: Arc<Host>,
    pub id: String,
    pub name: String,
    /// False for a saved series whose extension was removed.
    pub installed: bool,
}

impl Source {
    pub async fn call(&self, cx: &mut AsyncApp, op: &str, mut params: Value) -> Result<Value, String> {
        params["source"] = self.id.clone().into();
        call(&self.host, cx, op, params).await
    }
}

/// One Cloudflare solve at a time. Requests that hit the same check wait and reuse its cookie.
// One lock for all sites. Per-host locks if two sites often challenge at once.
static SOLVING: async_lock::Mutex<()> = async_lock::Mutex::new(());

/// Calls the host, and on a Cloudflare check solves it (with Byparr if it's running) and retries once.
pub async fn call(host: &Arc<Host>, cx: &mut AsyncApp, op: &str, params: Value) -> Result<Value, String> {
    match host.call(op, params.clone()).await {
        Err(HostError::Challenge { .. }) => {
            let _solving = SOLVING.lock().await;
            // Another request may have solved it while we waited.
            let (url, user_agent) = match host.call(op, params.clone()).await {
                Err(HostError::Challenge { url, user_agent }) => (url, user_agent),
                other => return other.map_err(|e| e.to_string()),
            };
            let base = byparr::base_url();
            let attempt = {
                let (base, url) = (base.clone(), url.clone());
                cx.background_executor().spawn(async move { if byparr::running(&base) { Some(byparr::solve(&base, &url)) } else { None } }).await
            };
            match attempt {
                Some(Ok(solved)) => {
                    let _ = host.set_cookies(&url, solved.cookies).await;
                    let _ = host.call("setUserAgent", serde_json::json!({ "url": url, "userAgent": solved.user_agent })).await;
                    match host.call(op, params.clone()).await {
                        Err(HostError::Challenge { .. }) => eprintln!("Byparr's cookies didn't pass {url}; trying the webview"),
                        other => return other.map_err(|e| e.to_string()),
                    }
                }
                Some(Err(e)) => eprintln!("{e}; trying the webview"),
                None => {}
            }
            let solved = cx.update(|cx| challenge::solve(cx, host.clone(), url, challenge::browser_user_agent(user_agent)));
            if solved.await.unwrap_or(false) {
                host.call(op, params).await.map_err(|e| e.to_string())
            } else {
                Err("Verification was cancelled".into())
            }
        }
        other => other.map_err(|e| e.to_string()),
    }
}

/// NANO_DEMO=<n> opens the nth popular series and pages through it (memory check).
pub fn demo() -> Option<usize> {
    std::env::var("NANO_DEMO").ok().map(|v| v.parse().unwrap_or(0))
}

/// Frees the GPU copies. Pass the window if you're inside one, since GPUI can't reach it while it's borrowed.
pub fn drop_images(cx: &mut App, mut window: Option<&mut Window>, images: Vec<Arc<RenderImage>>) {
    for image in images {
        cx.drop_image(image, window.as_deref_mut());
    }
}

gpui::actions!(yomi, [Quit, ShowLibrary, ShowBrowse, ShowExtensions, ShowHistory, ShowSettings, ZoomIn, ZoomOut, ZoomReset]);

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Library,
    Browse,
    Extensions,
    History,
    Settings,
}

const TABS: [(Tab, &str, &str); 5] = [
    (Tab::Library, "書庫", "Library"),
    (Tab::Browse, "探す", "Browse"),
    (Tab::Extensions, "拡張", "Extensions"),
    (Tab::History, "履歴", "History"),
    (Tab::Settings, "設定", "Settings"),
];

struct Root {
    cache: Entity<ImageCache>,
    store: Entity<Store>,
    registry: Entity<Registry>,
    tab: Tab,
    library: Entity<Library>,
    browse: Entity<Browse>,
    catalog: Option<Entity<Catalog>>,
    history: Option<Entity<History>>,
    settings: Option<Entity<settings::Settings>>,
    byparr: Entity<byparr::Service>,
    manga: Option<(Entity<MangaView>, Subscription)>,
    migrate: Option<(Entity<migrate::Migrate>, Subscription)>,
    reader: Option<(Entity<Reader>, Subscription)>,
    focus: gpui::FocusHandle,
    _subs: Vec<Subscription>,
}

impl Root {
    fn new(host: Arc<Host>, data_dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let cache = cx.new(|_| ImageCache::new(images::BUDGET_BYTES));
        let store = cx.new(|_| Store::load(data_dir.join("library.json")));
        set_zoom(store.read(cx).zoom());
        page_cache::set_cap_mb(store.read(cx).page_cache_mb());
        let registry = cx.new(|cx| Registry::new(host, data_dir.clone(), cx));
        let library = cx.new(|cx| Library::new(store.clone(), registry.clone(), cache.clone(), cx));
        let browse = cx.new(|cx| Browse::new(registry.clone(), store.clone(), cache.clone(), cx));
        let autostart = store.read(cx).byparr_autostart();
        let byparr = cx.new(|cx| byparr::Service::new(&data_dir, autostart, cx));
        let subs = vec![
            cx.subscribe_in(&browse, window, |this, _, ev: &OpenManga, window, cx| this.open_manga(ev, window, cx)),
            cx.subscribe_in(&library, window, |this, _, ev: &OpenManga, window, cx| this.open_manga(ev, window, cx)),
            cx.subscribe_in(&library, window, |this, _, ev: &Resume, window, cx| this.resume(ev, window, cx)),
            cx.subscribe_in(&library, window, |this, _, _: &GoBrowse, window, cx| this.show(Tab::Browse, window, cx)),
        ];
        let tab = if demo().is_some() { Tab::Browse } else { Tab::Library };
        // NANO_TAB=settings|browse starts on that tab (for testing).
        let tab = match std::env::var("NANO_TAB").as_deref() { Ok("settings") => Tab::Settings, Ok("browse") => Tab::Browse, _ => tab };
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let settings = (tab == Tab::Settings).then(|| {
            let (store, byparr) = (store.clone(), byparr.clone());
            cx.new(|cx| settings::Settings::new(store, byparr, cx))
        });
        Root { cache, store, registry, tab, library, browse, catalog: None, history: None, settings, byparr, manga: None, migrate: None, reader: None, focus, _subs: subs }
    }

    fn zoom_by(&mut self, delta: f32, cx: &mut Context<Self>) {
        set_zoom(zoom() + delta);
        let z = zoom();
        self.store.update(cx, |s, cx| s.set_zoom(z, cx));
        cx.refresh_windows();
    }

    fn show(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.tab = tab;
        self.manga = None;
        self.migrate = None;
        match tab {
            Tab::Extensions if self.catalog.is_none() => {
                let registry = self.registry.clone();
                let (store, cache) = (self.store.clone(), self.cache.clone());
                self.catalog = Some(cx.new(|cx| Catalog::new(registry, store, cache, cx)));
            }
            Tab::History if self.history.is_none() => {
                let (store, registry, cache) = (self.store.clone(), self.registry.clone(), self.cache.clone());
                let history = cx.new(|cx| History::new(store, registry, cache, cx));
                self._subs.push(cx.subscribe_in(&history, window, |this, _, ev: &Resume, window, cx| this.resume(ev, window, cx)));
                self.history = Some(history);
            }
            Tab::Settings if self.settings.is_none() => {
                let (store, byparr) = (self.store.clone(), self.byparr.clone());
                self.settings = Some(cx.new(|cx| settings::Settings::new(store, byparr, cx)));
            }
            _ => {}
        }
        cx.notify();
    }

    fn open_manga(&mut self, ev: &OpenManga, window: &mut Window, cx: &mut Context<Self>) {
        let (source, manga, cache, store) = (ev.source.clone(), ev.manga.clone(), self.cache.clone(), self.store.clone());
        let view = cx.new(|cx| MangaView::new(source, cache, store, manga, cx));
        let sub = cx.subscribe_in(&view, window, |this, view, ev: &MangaEvent, window, cx| match ev {
            MangaEvent::Back => {
                this.manga = None;
                cx.notify();
            }
            MangaEvent::Read { chapters, index, page } => {
                let (source, manga) = { let v = view.read(cx); (v.source.clone(), v.manga.clone()) };
                this.open_reader(source, manga, chapters.clone(), *index, *page, window, cx);
            }
            MangaEvent::Migrate => {
                let saved = { let v = view.read(cx); this.store.read(cx).get(&v.source.id, v.manga["url"].as_str().unwrap_or_default()).cloned() };
                if let Some(series) = saved {
                    this.open_migrate(series, window, cx);
                }
            }
        });
        self.manga = Some((view, sub));
        cx.notify();
    }

    fn open_migrate(&mut self, series: store::Series, window: &mut Window, cx: &mut Context<Self>) {
        let (registry, store, cache) = (self.registry.clone(), self.store.clone(), self.cache.clone());
        let view = cx.new(|cx| migrate::Migrate::new(series, &registry, store, cache, cx));
        let sub = cx.subscribe_in(&view, window, |this, _, ev: &migrate::MigrateEvent, window, cx| {
            this.migrate = None;
            if let migrate::MigrateEvent::Done { source, manga } = ev {
                this.open_manga(&OpenManga { source: source.clone(), manga: manga.clone() }, window, cx);
            }
            cx.notify();
        });
        self.migrate = Some((view, sub));
        cx.notify();
    }

    fn resume(&mut self, ev: &Resume, window: &mut Window, cx: &mut Context<Self>) {
        self.open_reader(ev.source.clone(), ev.manga.clone(), ev.chapters.clone(), ev.index, ev.page, window, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn open_reader(
        &mut self,
        source: Source,
        manga: Value,
        chapters: Arc<Vec<Value>>,
        index: usize,
        page: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (cache, store) = (self.cache.clone(), self.store.clone());
        let reader = cx.new(|cx| Reader::new(source, cache, store, manga, chapters, index, page, window, cx));
        let sub = cx.subscribe_in(&reader, window, |this, reader, _: &ReaderEvent, window, cx| {
            reader.update(cx, |r, cx| r.release_all(Some(window), cx));
            this.reader = None;
            this.focus.focus(window, cx);
            cx.notify();
        });
        self.reader = Some((reader, sub));
        cx.notify();
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(u(32.))
            .px(u(40.))
            .pt(u(12.))
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .pb(u(8.))
                    .child(div().font_family(CJK).font_weight(FontWeight::BLACK).text_size(u(22.)).text_color(rgb(ACCENT)).child("読"))
                    .child(div().font_family(DISPLAY).font_weight(FontWeight::BLACK).text_size(u(24.)).child("NANO")),
            )
            .child(div().flex().gap_1().flex_1().children(TABS.map(|(tab, jp, en)| {
                let on = self.tab == tab;
                div()
                    .id(en)
                    .flex()
                    .flex_col()
                    .px(u(14.))
                    .pt(u(6.))
                    .pb(u(8.))
                    .border_b_2()
                    .border_color(if on { rgb(ACCENT) } else { gpui::transparent_black().into() })
                    .text_color(rgb(if on { TEXT } else { MUTED }))
                    .cursor_pointer()
                    .hover(|s| s.text_color(rgb(TEXT)))
                    .child(div().font_family(CJK).text_size(u(10.)).text_color(rgb(MUTED)).child(jp))
                    .child(div().when(on, |d| d.font_weight(FontWeight::SEMIBOLD)).child(en))
                    .on_click(cx.listener(move |this, _, window, cx| this.show(tab, window, cx)))
            })))
            .child(
                div()
                    .id("jump")
                    .mb(u(8.))
                    .h(u(36.))
                    .min_w(u(260.))
                    .px(u(14.))
                    .rounded(u(18.))
                    .border_1()
                    .border_color(rgb(LINE))
                    .bg(rgb(0x141210))
                    .flex()
                    .items_center()
                    .gap(u(10.))
                    .text_size(u(13.))
                    .text_color(rgb(FAINT))
                    .cursor_pointer()
                    .child(search_icon())
                    .child(div().flex_1().child("Search sources…"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.show(Tab::Browse, window, cx);
                        this.browse.update(cx, |b, cx| b.search.update(cx, |s, cx| s.focus(window, cx)));
                    })),
            )
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(u(16.));
        let root = div().size_full().flex().flex_col().bg(rgb(BG)).text_color(rgb(TEXT)).font_family(SANS).text_size(u(14.));
        if let Some((reader, _)) = &self.reader {
            return root.child(reader.clone());
        }
        let screen: AnyElement = match (&self.manga, self.tab) {
            _ if self.migrate.is_some() => self.migrate.as_ref().unwrap().0.clone().into_any_element(),
            (Some((manga, _)), _) => manga.clone().into_any_element(),
            (None, Tab::Library) => self.library.clone().into_any_element(),
            (None, Tab::Browse) => self.browse.clone().into_any_element(),
            (None, Tab::Extensions) => self.catalog.clone().map(IntoElement::into_any_element).unwrap_or_else(|| div().into_any_element()),
            (None, Tab::History) => self.history.clone().map(IntoElement::into_any_element).unwrap_or_else(|| div().into_any_element()),
            (None, Tab::Settings) => self.settings.clone().map(IntoElement::into_any_element).unwrap_or_else(|| div().into_any_element()),
        };
        root.track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ShowLibrary, window, cx| this.show(Tab::Library, window, cx)))
            .on_action(cx.listener(|this, _: &ShowBrowse, window, cx| this.show(Tab::Browse, window, cx)))
            .on_action(cx.listener(|this, _: &ShowExtensions, window, cx| this.show(Tab::Extensions, window, cx)))
            .on_action(cx.listener(|this, _: &ShowHistory, window, cx| this.show(Tab::History, window, cx)))
            .on_action(cx.listener(|this, _: &ShowSettings, window, cx| this.show(Tab::Settings, window, cx)))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom_by(0.1, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom_by(-0.1, cx)))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| this.zoom_by(1. - zoom(), cx)))
            .child(self.header(cx))
            .child(div().flex_1().min_h_0().child(screen))
    }
}

fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NANO_DATA") {
        return dir.into();
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let (dir, old) = if cfg!(target_os = "macos") {
        (home.join("Library/Application Support/Nano"), home.join("Library/Application Support/Yomi"))
    } else if cfg!(windows) {
        let appdata = PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default());
        (appdata.join("Nano"), appdata.join("Yomi"))
    } else {
        (home.join(".local/share/nano"), home.join(".local/share/yomi"))
    };
    // The app used to be called Yomi. Move its library over once.
    if !dir.exists() && old.exists() {
        let _ = std::fs::rename(&old, &dir);
    }
    dir
}

fn main() {
    let data_dir = data_dir();
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    covers::set_cache_dir(data_dir.join("covers"));
    page_cache::set_dir(data_dir.join("pages"));
    // Kagane serves JPEG XL, which `image` only decodes through this hook.
    jxl_oxide::integration::register_image_decoding_hook();
    let heap_mb = Store::load(data_dir.join("library.json")).host_heap_mb().unwrap_or_else(host::auto_heap_mb);
    let _ = host::LAUNCH_HEAP_MB.set(heap_mb);
    let host = Host::spawn(&host::host_bin(), &data_dir.join("host"), heap_mb)
        .unwrap_or_else(|e| panic!("start extension host {}: {e}", host::host_bin().display()));

    gpui_platform::application().with_assets(Assets).run(move |cx: &mut App| {
        load_fonts(cx);
        webviews::start(cx, host.clone());
        cx.bind_keys([
            gpui::KeyBinding::new("cmd-1", ShowLibrary, None),
            gpui::KeyBinding::new("cmd-2", ShowBrowse, None),
            gpui::KeyBinding::new("cmd-3", ShowExtensions, None),
            gpui::KeyBinding::new("cmd-4", ShowHistory, None),
            gpui::KeyBinding::new("cmd-5", ShowSettings, None),
            gpui::KeyBinding::new("cmd-,", ShowSettings, None),
            gpui::KeyBinding::new("cmd-=", ZoomIn, None),
            gpui::KeyBinding::new("cmd-+", ZoomIn, None),
            gpui::KeyBinding::new("cmd--", ZoomOut, None),
            gpui::KeyBinding::new("cmd-0", ZoomReset, None),
            gpui::KeyBinding::new(if cfg!(target_os = "macos") { "cmd-q" } else { "ctrl-q" }, Quit, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.set_menus(vec![gpui::Menu { name: "Nano".into(), items: vec![gpui::MenuItem::action("Quit Nano", Quit)], disabled: false }]);
        cx.on_window_closed(|cx, _| {
            // The hidden WebView window shouldn't keep the app open.
            if cx.windows().iter().all(|w| webviews::is_webview_window(cx, w.window_id())) {
                cx.quit();
            }
        })
        .detach();

        // NANO_CHALLENGE_TEST=<url> opens the challenge window for a manual check.
        if let Ok(url) = std::env::var("NANO_CHALLENGE_TEST") {
            let host = host.clone();
            cx.spawn(async move |cx| {
                let ua = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
                let solved = cx.update(|cx| challenge::solve(cx, host, url, ua.into())).await;
                eprintln!("challenge solved: {solved:?}");
            })
            .detach();
        }

        let bounds = Bounds::centered(None, size(px(1280.), px(860.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions { title: Some("Nano".into()), ..Default::default() }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Root::new(host, data_dir, window, cx)),
        )
        .expect("open window");
        cx.activate(true);
    });
}
