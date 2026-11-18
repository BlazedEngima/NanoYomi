//! Cloudflare checks: pass them in a hidden system web view, then hand its cookies to the extension host.

use crate::host::Host;
use futures::channel::oneshot;
use gpui::{
    App, AppContext, Bounds, Context, IntoElement, Pixels, Render, Size, Styled, TitlebarOptions,
    Window, WindowBounds, WindowOptions, div, px, size,
};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

const PAGE_PASSED_JS: &str = r#"(() => {
  const challenge = /just a moment|attention required|un instant/i.test(document.title || '')
    || !!document.querySelector('#challenge-form, #challenge-running, #challenge-stage, #cf-challenge-running, [name="cf-turnstile-response"]');
  return document.readyState === 'complete' && !challenge && !!document.body && document.body.innerText.trim().length > 0;
})()"#;

const TIMEOUT: Duration = Duration::from_secs(120);
/// Most checks pass on their own by now. After this the window shows so you can click.
const SHOW_AFTER: Duration = Duration::from_secs(12);

struct Challenge {
    webview: wry::WebView,
    shown_size: Size<Pixels>,
}

/// WebKit has to say it's Safari, or Cloudflare rejects the tick. WebView2 can use the extension's own agent.
pub fn browser_user_agent(extension_agent: String) -> String {
    if cfg!(target_os = "macos") {
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/27.2 Safari/605.1.15".into()
    } else {
        extension_agent
    }
}

/// Resolves true once the clearance cookie reaches the host. Errors if the user closes the window.
pub fn solve(cx: &mut App, host: Arc<Host>, url: String, user_agent: String) -> oneshot::Receiver<bool> {
    let (tx, rx) = oneshot::channel();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(480.), px(680.)), cx))),
        titlebar: Some(TitlebarOptions { title: Some("Verify to continue".into()), ..Default::default() }),
        focus: false,
        ..Default::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        cx.new(|cx| {
            let viewport = window.viewport_size();
            let webview = wry::WebViewBuilder::new()
                .with_url(&url)
                .with_user_agent(&user_agent)
                .with_bounds(rect(viewport))
                .build_as_child(window)
                .expect("system webview unavailable");
            // Invisible but still drawn, so Cloudflare's script runs at full speed.
            crate::webviews::native::keep_running_hidden(&webview);
            crate::webviews::native::hide_window(&webview);

            cx.spawn_in(window, async move |this, cx| {
                // A stale clearance would look like an instant solve, so clear it and reload.
                let mut stale: Option<Vec<String>> = None;
                let started = std::time::Instant::now();
                // Two passing checks in a row count as solved.
                let passed = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let mut clear_ticks = 0;
                let mut shown = false;
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if !shown && started.elapsed() > SHOW_AFTER {
                        shown = true;
                        let _ = this.update_in(cx, |this: &mut Challenge, window, cx| {
                            crate::webviews::native::show_window(&this.webview);
                            window.activate_window();
                            cx.activate(true);
                        });
                    }
                    // Give up eventually, or every queued page waits behind it.
                    if started.elapsed() > TIMEOUT {
                        let _ = this.update_in(cx, |_, window, _| window.remove_window());
                        return;
                    }
                    // This errors once the window is gone, which means the user gave up.
                    let Ok(cookies) = this.update(cx, |this: &mut Challenge, _| this.webview.cookies_for_url(&url)) else {
                        return;
                    };
                    let cookies = cookies.unwrap_or_default();
                    let old = cookies.iter().filter(|c| c.name() == "cf_clearance").map(|c| c.value().to_string()).collect::<Vec<_>>();
                    let Some(stale) = &stale else {
                        if !old.is_empty() {
                            let _ = this.update(cx, |this: &mut Challenge, _| {
                                for c in cookies.iter().filter(|c| c.name() == "cf_clearance") {
                                    let _ = this.webview.delete_cookie(c);
                                }
                                let _ = this.webview.load_url(&url);
                            });
                        }
                        stale = Some(old);
                        continue;
                    };
                    // Solved when a new clearance shows up, or the page stops being a challenge.
                    clear_ticks = if passed.swap(false, Ordering::Relaxed) { clear_ticks + 1 } else { 0 };
                    let p = passed.clone();
                    let _ = this.update(cx, |this: &mut Challenge, _| {
                        this.webview.evaluate_script_with_callback(PAGE_PASSED_JS, move |r| {
                            if r == "true" {
                                p.store(true, Ordering::Relaxed);
                            }
                        })
                    });
                    if !old.iter().any(|v| !stale.contains(v)) && clear_ticks < 2 {
                        continue;
                    }
                    // The full Set-Cookie form, which the host parses.
                    let header_values = cookies.iter().map(|c| c.to_string()).collect();
                    let ok = host.set_cookies(&url, header_values).await.is_ok()
                        // The webview passed with this agent, so the host keeps using it for the site.
                        && host.call("setUserAgent", serde_json::json!({ "url": url, "userAgent": user_agent })).await.is_ok();
                    let _ = tx.send(ok);
                    let _ = this.update_in(cx, |_, window, _| window.remove_window());
                    return;
                }
            })
            .detach();

            Challenge { webview, shown_size: viewport }
        })
    });
    // If the window never opened, `tx` was dropped, which reads as a cancel.
    let _ = opened;
    rx
}

impl Render for Challenge {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        if viewport != self.shown_size {
            let _ = self.webview.set_bounds(rect(viewport));
            self.shown_size = viewport;
        }
        div().size_full()
    }
}

fn rect(s: Size<Pixels>) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::LogicalPosition::new(0.0, 0.0).into(),
        size: wry::dpi::LogicalSize::new(f64::from(f32::from(s.width)), f64::from(f32::from(s.height))).into(),
    }
}
