use crate::catalog::small_button;
use crate::input::{InputEvent, TextInput};
use crate::store::{Store, SyncStatus};
use crate::{FAINT, MONO, MUTED, RULE, ACCENT_TEXT, heading, u};
use gpui::{
    AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, rgb,
};

const PAD: f32 = 40.;

pub struct Settings {
    store: Entity<Store>,
    /// Shows "Cleared" on the button until the next visit.
    covers_cleared: bool,
    pages_cleared: bool,
    sync_host: Entity<TextInput>,
    sync_key: Entity<TextInput>,
    solver: Entity<TextInput>,
}

impl Settings {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        let plain = |placeholder: &str, text: &str, cx: &mut Context<Self>| {
            let (placeholder, text) = (placeholder.to_string(), text.to_string());
            cx.new(|cx| {
                let mut t = TextInput::new(placeholder, cx);
                t.text = text;
                t.icon = false;
                t
            })
        };
        let field = |placeholder: &str, text: &str, cx: &mut Context<Self>| {
            let input = plain(placeholder, text, cx);
            cx.subscribe(&input, |this, _, _: &InputEvent, cx| this.sync_now(cx)).detach();
            input
        };
        let cfg = store.read(cx).sync_config().clone();
        let sync_host = field("Server, e.g. http://192.168.0.3:8282", &cfg.host, cx);
        let sync_key = field("API key", &cfg.key, cx);
        let url = store.read(cx).solver_url().to_string();
        let solver = plain("Optional, e.g. http://192.168.0.5:8191", &url, cx);
        // The field has no change event, so save it whenever it redraws.
        cx.observe(&solver, |this, input, cx| {
            let url = input.read(cx).text.clone();
            this.store.update(cx, |s, cx| s.set_solver_url(&url, cx));
        })
        .detach();
        Settings { store, covers_cleared: false, pages_cleared: false, sync_host, sync_key, solver }
    }

    fn sync_now(&mut self, cx: &mut Context<Self>) {
        let (host, key) = (self.sync_host.read(cx).text.clone(), self.sync_key.read(cx).text.clone());
        self.store.update(cx, |s, cx| s.set_sync_account(&host, &key, cx));
        crate::sync::run(self.store.clone(), cx);
    }
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
        let auto = self.store.read(cx).auto_download();
        let auto_chip = |id: &'static str, label: &'static str, value: bool, store: Entity<Store>| {
            small_button(id, label, auto == value).on_click(move |_, _, cx| store.update(cx, |s, cx| s.set_auto_download(value, cx)))
        };
        let folder = crate::downloads::dir().map(|d| d.display().to_string()).unwrap_or_default();
        let downloads = div()
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
                    .child(div().text_size(u(15.)).font_weight(FontWeight::SEMIBOLD).child("Download new chapters"))
                    .child(div().text_size(u(13.)).text_color(rgb(MUTED)).child(format!(
                        "When Nano finds new chapters for series in your library, it downloads them for reading offline. Saved in {folder}"
                    ))),
            )
            .child(div().flex_none().flex().gap(u(6.)).child(auto_chip("auto-off", "Off", false, self.store.clone())).child(auto_chip("auto-on", "On", true, self.store.clone())));
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
            .child(div().pt(u(24.)).pb(u(10.)).font_family(MONO).text_size(u(11.)).text_color(rgb(FAINT)).child("DOWNLOADS"))
            .child(downloads)
            .child(div().pt(u(24.)).pb(u(10.)).font_family(MONO).text_size(u(11.)).text_color(rgb(FAINT)).child("SYNC"))
            .child(sync)
            .child(div().pt(u(24.)).pb(u(10.)).font_family(MONO).text_size(u(11.)).text_color(rgb(FAINT)).child("CLOUDFLARE"))
            .child(
                div()
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
                            .child(div().text_size(u(15.)).font_weight(FontWeight::SEMIBOLD).child("Solver"))
                            .child(div().text_size(u(13.)).text_color(rgb(MUTED)).child(
                                "Nano passes Cloudflare checks on its own and only asks you to click when a site insists. If you run FlareSolverr or Byparr somewhere, add its address and Nano will try it first.",
                            )),
                    )
                    .child(div().flex_none().w(u(340.)).child(self.solver.clone())),
            )
            .child(div().pt(u(24.)).pb(u(10.)).font_family(MONO).text_size(u(11.)).text_color(rgb(FAINT)).child("STORAGE"))
            .child(storage)
            .child(pages)
            .child(div().pt(u(24.)).pb(u(10.)).font_family(MONO).text_size(u(11.)).text_color(rgb(FAINT)).child("MEMORY"))
            .child(heap)
    }
}
