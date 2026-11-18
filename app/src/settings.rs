use crate::byparr::{Service, Status};
use crate::catalog::small_button;
use crate::input::{InputEvent, TextInput};
use crate::store::{Store, SyncStatus};
use crate::{FAINT, LINE, MONO, MUTED, RULE, SURFACE, TEXT, ACCENT, ACCENT_TEXT, BG, heading, u};
use gpui::{
    AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, rgb,
};

const PAD: f32 = 40.;

pub struct Settings {
    store: Entity<Store>,
    byparr: Entity<Service>,
    /// Shows "Cleared" on the button until the next visit.
    covers_cleared: bool,
    pages_cleared: bool,
    sync_host: Entity<TextInput>,
    sync_key: Entity<TextInput>,
}

impl Settings {
    pub fn new(store: Entity<Store>, byparr: Entity<Service>, cx: &mut Context<Self>) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        cx.observe(&byparr, |_, _, cx| cx.notify()).detach();
        let field = |placeholder: &str, text: &str, cx: &mut Context<Self>| {
            let (placeholder, text) = (placeholder.to_string(), text.to_string());
            let input = cx.new(|cx| {
                let mut t = TextInput::new(placeholder, cx);
                t.text = text;
                t.icon = false;
                t
            });
            cx.subscribe(&input, |this, _, _: &InputEvent, cx| this.sync_now(cx)).detach();
            input
        };
        let cfg = store.read(cx).sync_config().clone();
        let sync_host = field("Server, e.g. http://192.168.0.3:8282", &cfg.host, cx);
        let sync_key = field("API key", &cfg.key, cx);
        Settings { store, byparr, covers_cleared: false, pages_cleared: false, sync_host, sync_key }
    }

    fn sync_now(&mut self, cx: &mut Context<Self>) {
        let (host, key) = (self.sync_host.read(cx).text.clone(), self.sync_key.read(cx).text.clone());
        self.store.update(cx, |s, cx| s.set_sync_account(&host, &key, cx));
        crate::sync::run(self.store.clone(), cx);
    }
}

struct Tip(&'static str);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(u(10.))
            .py(u(6.))
            .rounded(u(6.))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(LINE))
            .text_size(u(12.))
            .text_color(rgb(TEXT))
            .child(self.0)
    }
}

fn info(id: &'static str, tip: &'static str) -> impl IntoElement {
    div()
        .id(id)
        .size(u(16.))
        .rounded_full()
        .border_1()
        .border_color(rgb(LINE))
        .flex()
        .items_center()
        .justify_center()
        .font_family(MONO)
        .text_size(u(10.))
        .text_color(rgb(FAINT))
        .child("i")
        .tooltip(move |_, cx| cx.new(|_| Tip(tip)).into())
}

impl Render for Settings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let clear = small_button("covers-clear", if self.covers_cleared { "Cleared" } else { "Clear" }, false).on_click(cx.listener(|this, _, _, cx| {
            crate::covers::clear_cache();
            this.covers_cleared = true;
            cx.notify();
        }));
        let storage = div()
            .w_full()
            .py(u(14.))
            .border_t_1()
            .border_b_1()
            .border_color(rgb(RULE))
            .flex()
            .items_center()
            .gap_4()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(u(4.))
                    .child(div().text_size(u(15.)).font_weight(FontWeight::SEMIBOLD).child("Cover cache"))
                    .child(div().text_size(u(13.)).text_color(rgb(MUTED)).child(
                        "Covers are kept for a week, so sites aren’t asked (or verified) again. Clear to fetch every cover fresh.",
                    )),
            )
            .child(clear);
        let cap = self.store.read(cx).page_cache_mb();
        let caps = crate::page_cache::CAPS_MB.map(|mb| {
            let label = if mb >= 1024 { format!("{} GB", mb / 1024) } else { format!("{mb} MB") };
            let store = self.store.clone();
            small_button(("page-cap", mb as usize), "", mb == cap)
                .child(label)
                .on_click(move |_, _, cx| store.update(cx, |s, cx| s.set_page_cache_mb(mb, cx)))
        });
        let clear_pages = small_button("pages-clear", if self.pages_cleared { "Cleared" } else { "Clear" }, false).on_click(cx.listener(|this, _, _, cx| {
            crate::page_cache::clear();
            this.pages_cleared = true;
            cx.notify();
        }));
        let pages = div()
            .w_full()
            .py(u(14.))
            .border_b_1()
            .border_color(rgb(RULE))
            .flex()
            .items_center()
            .gap_4()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(u(4.))
                    .child(div().text_size(u(15.)).font_weight(FontWeight::SEMIBOLD).child("Page cache"))
                    .child(div().text_size(u(13.)).text_color(rgb(MUTED)).child(
                        "Chapter pages you’ve read stay on disk, so going back doesn’t download them again. Past the limit, the longest-unread pages go first.",
                    )),
            )
            .child(div().flex_none().flex().gap(u(6.)).children(caps))
            .child(clear_pages);
        let vertical = self.store.read(cx).vertical_reader();
        let mode_chip = |id: &'static str, label: &'static str, value: bool, store: Entity<Store>| {
            small_button(id, label, vertical == value).on_click(move |_, _, cx| store.update(cx, |s, cx| s.set_vertical_reader(value, cx)))
        };
        let reading = div()
            .w_full()
            .py(u(14.))
            .border_t_1()
            .border_b_1()
            .border_color(rgb(RULE))
            .flex()
            .items_center()
            .gap_4()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(u(4.))
                    .child(div().text_size(u(15.)).font_weight(FontWeight::SEMIBOLD).child("Reading mode"))
                    .child(div().text_size(u(13.)).text_color(rgb(MUTED)).child(
                        "How chapters open. Paged shows one page at a time; Vertical scrolls them as one strip, for webtoons. Pressing V in the reader changes this too.",
                    )),
            )
            .child(div().flex_none().flex().gap(u(6.)).child(mode_chip("mode-paged", "Paged", false, self.store.clone())).child(mode_chip("mode-vertical", "Vertical", true, self.store.clone())));
        let chosen = self.store.read(cx).host_heap_mb();
        let auto = crate::host::auto_heap_mb();
        let effective = chosen.unwrap_or(auto);
        let restart = crate::host::LAUNCH_HEAP_MB.get().is_some_and(|m| *m != effective);
        let store = self.store.clone();
        let heap_choice = move |id: (&'static str, usize), label: String, value: Option<u32>| {
            let store = store.clone();
            small_button(id, "", chosen == value).child(label).on_click(move |_, _, cx| store.update(cx, |s, cx| s.set_host_heap_mb(value, cx)))
        };
        let heap_chips: Vec<_> = std::iter::once(heap_choice(("heap", 0), format!("Auto · {auto} MB"), None))
            .chain(crate::host::HEAP_CHOICES_MB.map(|mb| heap_choice(("heap", mb as usize), format!("{mb} MB"), Some(mb))))
            .collect();
        let heap = div()
            .w_full()
            .py(u(14.))
            .border_t_1()
            .border_b_1()
            .border_color(rgb(RULE))
            .flex()
            .items_center()
            .gap_4()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(u(4.))
                    .child(div().text_size(u(15.)).font_weight(FontWeight::SEMIBOLD).child("Extension memory"))
                    .child(div().text_size(u(13.)).text_color(rgb(MUTED)).child(
                        "How much memory extensions may use. More helps sources with heavy pages; less keeps Nano lighter. Auto sizes it for this computer.",
                    ))
                    .when(restart, |d| d.child(div().text_size(u(13.)).text_color(rgb(ACCENT_TEXT)).child("Restart Nano to apply"))),
            )
            .child(div().flex_none().flex().gap(u(6.)).children(heap_chips));
        let status = self.byparr.read(cx).status.clone();
        let auto = self.store.read(cx).byparr_autostart();
        let (label, busy) = match &status {
            Status::Checking => ("Checking…".to_string(), true),
            Status::NoDocker => ("Needs Docker Desktop".into(), false),
            Status::Missing => ("Not downloaded".into(), false),
            Status::Downloading => ("Downloading… (about 1 GB)".into(), true),
            Status::Stopped => ("Stopped".into(), false),
            Status::Starting => ("Working…".into(), true),
            Status::Running => ("Running".into(), false),
            Status::Failed(e) => (e.clone(), false),
        };
        let byparr = self.byparr.clone();
        let action = match &status {
            Status::Missing => Some(small_button("byparr-dl", "Download", true).on_click(move |_, _, cx| byparr.update(cx, |b, cx| b.download(cx)))),
            Status::Stopped | Status::Failed(_) => Some(small_button("byparr-run", "Run", true).on_click(move |_, _, cx| byparr.update(cx, |b, cx| b.run(cx)))),
            Status::Running => Some(small_button("byparr-stop", "Stop", false).on_click(move |_, _, cx| byparr.update(cx, |b, cx| b.stop(cx)))),
            _ => None,
        };
        let store = self.store.clone();
        let checkbox = div()
            .id("byparr-auto")
            .flex()
            .items_center()
            .gap(u(8.))
            .cursor_pointer()
            .child(
                div()
                    .size(u(16.))
                    .rounded(u(4.))
                    .border_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(u(11.))
                    .when(auto, |d| d.bg(rgb(ACCENT)).border_color(rgb(ACCENT)).text_color(rgb(BG)).child("✓"))
                    .when(!auto, |d| d.border_color(rgb(LINE))),
            )
            .child(div().text_size(u(13.)).text_color(rgb(MUTED)).child("Always run on startup"))
            .on_click(move |_, _, cx| store.update(cx, |s, cx| s.set_byparr_autostart(!auto, cx)));

        let (sync_label, sync_failed) = match &self.store.read(cx).sync_status {
            SyncStatus::Idle => (if self.store.read(cx).sync_config().key.is_empty() { "Not set up".to_string() } else { "Not synced yet".into() }, false),
            SyncStatus::Syncing => ("Syncing…".into(), false),
            SyncStatus::Done(at) => (format!("Synced {}", crate::store::ago(*at, crate::store::now_ms())), false),
            SyncStatus::Failed(e) => (e.clone(), true),
        };
        let sync = div()
            .w_full()
            .py(u(14.))
            .border_t_1()
            .border_b_1()
            .border_color(rgb(RULE))
            .flex()
            .flex_col()
            .gap(u(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(u(4.))
                            .child(div().text_size(u(15.)).font_weight(FontWeight::SEMIBOLD).child("SyncYomi"))
                            .child(div().text_size(u(13.)).text_color(rgb(MUTED)).child(
                                "Keeps your library and reading progress in step with Mihon forks that sync with SyncYomi (TachiyomiSY, Komikku). Use the same server and API key as on your phone. Syncs when Nano starts and when you close a chapter.",
                            )),
                    )
                    .child(
                        div()
                            .flex_none()
                            .max_w(u(320.))
                            .truncate()
                            .font_family(MONO)
                            .text_size(u(12.))
                            .text_color(rgb(if sync_failed { ACCENT_TEXT } else { FAINT }))
                            .child(sync_label),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(u(10.))
                    .child(div().flex_1().min_w_0().child(self.sync_host.clone()))
                    .child(div().w(u(300.)).child(self.sync_key.clone()))
                    .child(small_button("sync-now", "Sync now", true).on_click(cx.listener(|this, _, _, cx| this.sync_now(cx)))),
            );

        div()
            .id("settings")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .px(u(PAD))
            .pt(u(28.))
            .pb(u(40.))
            .child(heading("Settings", ""))
            .child(div().pt(u(24.)).pb(u(10.)).font_family(MONO).text_size(u(11.)).text_color(rgb(FAINT)).child("READING"))
            .child(reading)
            .child(div().pt(u(24.)).pb(u(10.)).font_family(MONO).text_size(u(11.)).text_color(rgb(FAINT)).child("SYNC"))
            .child(sync)
            .child(div().pt(u(24.)).pb(u(10.)).font_family(MONO).text_size(u(11.)).text_color(rgb(FAINT)).child("UTILITIES"))
            .child(
                div()
                    .w_full()
                    .py(u(14.))
                    .border_t_1()
                    .border_b_1()
                    .border_color(rgb(RULE))
                    .flex()
                    .items_start()
                    .gap_4()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(u(4.))
                            .child(div().text_size(u(15.)).font_weight(FontWeight::SEMIBOLD).child("Byparr"))
                            .child(
                                div()
                                    .text_size(u(13.))
                                    .text_color(rgb(MUTED))
                                    .child("Solves Cloudflare's \u{201c}Just a moment\u{2026}\u{201d} checks without a window. Runs in Docker."),
                            )
                            .child(
                                div()
                                    .pt(u(6.))
                                    .flex()
                                    .items_center()
                                    .gap(u(6.))
                                    .child(checkbox)
                                    .child(info("byparr-auto-info", "Starts with Nano, not with your computer")),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(u(12.))
                            .child(
                                div()
                                    .max_w(u(320.))
                                    .truncate()
                                    .font_family(MONO)
                                    .text_size(u(12.))
                                    .text_color(rgb(match &status {
                                        Status::Running => ACCENT_TEXT,
                                        Status::Failed(_) => ACCENT_TEXT,
                                        _ => FAINT,
                                    }))
                                    .when(busy, |d| d.opacity(0.8))
                                    .child(label),
                            )
                            .children(action),
                    ),
            )
            .child(div().pt(u(24.)).pb(u(10.)).font_family(MONO).text_size(u(11.)).text_color(rgb(FAINT)).child("STORAGE"))
            .child(storage)
            .child(pages)
            .child(div().pt(u(24.)).pb(u(10.)).font_family(MONO).text_size(u(11.)).text_color(rgb(FAINT)).child("MEMORY"))
            .child(heap)
    }
}
