//! An extension's own settings (content rating, image quality, mirrors...), drawn from what it declares.
//! Values live in the host under `source_<id>`, where Mihon keeps them too.

use crate::catalog::{chip, small_button};
use crate::input::{InputEvent, TextInput};
use crate::store::Store;
use crate::{ACCENT_TEXT, FAINT, LINE, MUTED, RULE, SURFACE, Source, TEXT, u};
use gpui::{
    AnyElement, AppContext, Context, Entity, EventEmitter, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, rgb,
};
use serde_json::{Value, json};
use std::collections::HashMap;

pub struct Close;

pub struct ExtSettings {
    /// One per language the extension offers. Each keeps its own settings.
    sources: Vec<Source>,
    picked: usize,
    store: Entity<Store>,
    pkg: String,
    shown: Vec<String>,
    prefs: Vec<Value>,
    status: SharedString,
    texts: HashMap<String, Entity<TextInput>>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<Close> for ExtSettings {}

impl ExtSettings {
    pub fn new(sources: Vec<Source>, shown: Vec<String>, pkg: String, store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let mut this = ExtSettings { sources, picked: 0, store, pkg, shown, prefs: vec![], status: "Loading…".into(), texts: HashMap::new(), _subs: vec![] };
        this.load(cx);
        this
    }

    fn source(&self) -> Source {
        self.sources[self.picked].clone()
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let source = self.source();
        cx.spawn(async move |this, cx| {
            let result = source.call(cx, "settings", json!({})).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Value::Array(list)) => {
                        this.status = if list.is_empty() { "This extension has no settings.".into() } else { "".into() };
                        this.set_prefs(list, cx);
                    }
                    Ok(_) => this.status = "This extension has no settings.".into(),
                    Err(e) => this.status = e.into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Text fields keep what's typed between redraws, so they're made once per key and saved on Enter.
    fn set_prefs(&mut self, list: Vec<Value>, cx: &mut Context<Self>) {
        self.texts.clear();
        self._subs.clear();
        for p in list.iter().filter(|p| p["type"] == "text") {
            let key = p["key"].as_str().unwrap_or_default().to_string();
            let value = p["value"].as_str().unwrap_or_default().to_string();
            let input = cx.new(|cx| {
                let mut i = TextInput::new("Press Enter to save", cx);
                i.text = value;
                i.icon = false;
                i
            });
            let k = key.clone();
            self._subs.push(cx.subscribe(&input, move |this, input, _: &InputEvent, cx| {
                let text = input.read(cx).text.clone();
                this.set(&k, json!(text), cx);
            }));
            self.texts.insert(key, input);
        }
        self.prefs = list;
    }

    /// Saves one value and updates it in place. Reloading right away can catch Kagane mid-refresh, and it drops values it can't see yet.
    fn set(&mut self, key: &str, value: Value, cx: &mut Context<Self>) {
        let source = self.source();
        let key = key.to_string();
        if let Some(p) = self.prefs.iter_mut().find(|p| p["key"] == key.as_str()) {
            p["value"] = value.clone();
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = source.call(cx, "setSetting", json!({ "key": key, "value": value })).await;
            let _ = this.update(cx, |this, cx| {
                this.status = match result {
                    Ok(Value::Bool(false)) => "The extension didn't accept that value.".into(),
                    Ok(_) => "".into(),
                    Err(e) => e.into(),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn row(&self, i: usize, p: &Value, cx: &mut Context<Self>) -> AnyElement {
        let key = p["key"].as_str().unwrap_or_default().to_string();
        let title = p["title"].as_str().unwrap_or_default().to_string();
        let kind = p["type"].as_str().unwrap_or_default();
        let enabled = p["enabled"].as_bool().unwrap_or(true);
        let strings = |k: &str| p[k].as_array().map(|a| a.iter().map(|v| v.as_str().unwrap_or_default().to_string()).collect::<Vec<_>>()).unwrap_or_default();
        let mut summary = p["summary"].as_str().unwrap_or_default().to_string();
        if kind == "switch" {
            let alt = if p["value"].as_bool() == Some(true) { "summaryOn" } else { "summaryOff" };
            if let Some(s) = p[alt].as_str() {
                summary = s.to_string();
            }
        }
        // Android fills "%s" with the chosen entry's name.
        if kind == "list" {
            let (entries, values) = (strings("entries"), strings("values"));
            let chosen = values.iter().position(|v| Some(v.as_str()) == p["value"].as_str()).and_then(|i| entries.get(i)).cloned().unwrap_or_default();
            summary = summary.replace("%s", &chosen);
        }
        let control: AnyElement = match kind {
            "switch" => {
                let on = p["value"].as_bool().unwrap_or(false);
                let k = key.clone();
                div()
                    .flex()
                    .gap(u(6.))
                    .child(small_button(("off", i), "Off", !on).on_click(cx.listener({
                        let k = k.clone();
                        move |this, _, _, cx| this.set(&k, json!(false), cx)
                    })))
                    .child(small_button(("on", i), "On", on).on_click(cx.listener(move |this, _, _, cx| this.set(&k, json!(true), cx))))
                    .into_any_element()
            }
            "list" | "multi" => {
                let (entries, values) = (strings("entries"), strings("values"));
                let chosen: Vec<String> = match &p["value"] {
                    Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
                    v => v.as_str().map(|s| vec![s.to_string()]).unwrap_or_default(),
                };
                let multi = kind == "multi";
                div()
                    .flex()
                    .flex_wrap()
                    .gap(u(6.))
                    .children(entries.into_iter().zip(values).enumerate().map(|(j, (name, value))| {
                        let on = chosen.contains(&value);
                        let (k, chosen) = (key.clone(), chosen.clone());
                        chip(SharedString::from(format!("opt-{i}-{j}")), name, on).on_click(cx.listener(move |this, _, _, cx| {
                            if multi {
                                let mut set = chosen.clone();
                                if on {
                                    set.retain(|v| *v != value);
                                } else {
                                    set.push(value.clone());
                                }
                                this.set(&k, json!(set), cx)
                            } else if !on {
                                this.set(&k, json!(value), cx)
                            }
                        }))
                    }))
                    .into_any_element()
            }
            "text" => match self.texts.get(&key) {
                Some(input) => input.clone().into_any_element(),
                None => div().into_any_element(),
            },
            _ => div().into_any_element(),
        };
        div()
            .py(u(14.))
            .flex()
            .flex_col()
            .gap(u(8.))
            .border_b_1()
            .border_color(rgb(RULE))
            .when(!enabled, |d| d.opacity(0.5))
            .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
            .when(!summary.is_empty(), |d| d.child(div().text_size(u(13.)).text_color(rgb(MUTED)).child(summary)))
            .when_some(p["message"].as_str().filter(|_| kind == "text"), |d, m| d.child(div().text_size(u(12.)).text_color(rgb(FAINT)).child(m.to_string())))
            .when(!enabled, |d| d.child(div().text_size(u(12.)).text_color(rgb(FAINT)).child("The extension hasn't loaded these options yet. Browse the source once, then come back.")))
            .when(enabled, |d| d.child(control))
            .into_any_element()
    }
}

impl Render for ExtSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let prefs = self.prefs.clone();
        let rows: Vec<AnyElement> = prefs.iter().enumerate().map(|(i, p)| self.row(i, p, cx)).collect();
        let name = self.sources.first().map(|s| s.name.clone()).unwrap_or_default();
        let languages = (self.sources.len() > 1).then(|| {
            div().flex().flex_wrap().gap(u(6.)).pb(u(8.)).children(self.sources.iter().enumerate().map(|(i, s)| {
                chip(SharedString::from(format!("ext-lang-{i}")), s.lang.clone(), i == self.picked).on_click(cx.listener(move |this, _, _, cx| {
                    if this.picked != i {
                        this.picked = i;
                        this.prefs.clear();
                        this.status = "Loading…".into();
                        this.load(cx);
                        cx.notify();
                    }
                }))
            }))
        });
        let in_browse = (self.sources.len() > 1).then(|| {
            let lang = self.source().lang;
            let on = self.shown.contains(&lang);
            div()
                .flex()
                .items_center()
                .gap_3()
                .pb(u(8.))
                .child(div().text_size(u(13.)).text_color(rgb(MUTED)).child(format!("Show {lang} in Browse")))
                .child(chip("in-browse", if on { "on" } else { "off" }, on).on_click(cx.listener(move |this, _, _, cx| {
                    let mut langs = this.shown.clone();
                    if on {
                        langs.retain(|x| *x != lang);
                    } else {
                        langs.push(lang.clone());
                    }
                    // Keep at least one language or the extension disappears from Browse.
                    if !langs.is_empty() {
                        this.shown = langs.clone();
                        let pkg = this.pkg.clone();
                        this.store.update(cx, |s, cx| s.set_languages(&pkg, langs, cx));
                        cx.notify();
                    }
                })))
        });
        div()
            .id("ext-settings")
            .occlude()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .w(u(460.))
            .bg(rgb(SURFACE))
            .border_l_1()
            .border_color(rgb(LINE))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(u(24.))
                    .pt(u(24.))
                    .pb(u(12.))
                    .flex()
                    .items_center()
                    .child(div().flex_1().min_w_0().truncate().text_size(u(17.)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(TEXT)).child(format!("{name} settings")))
                    .child(small_button("ext-settings-close", "Close", false).on_click(cx.listener(|_, _, _, cx| cx.emit(Close)))),
            )
            .child(
                div()
                    .id("ext-settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(u(24.))
                    .pb(u(24.))
                    .children(languages)
                    .children(in_browse)
                    .when(!self.status.is_empty(), |d| d.child(div().py(u(10.)).text_size(u(13.)).text_color(rgb(ACCENT_TEXT)).child(self.status.clone())))
                    .children(rows),
            )
    }
}
