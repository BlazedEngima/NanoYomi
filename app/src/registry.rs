//! Installed extensions and their sources. Every jar is loaded into the host at startup.

use crate::extensions::{self, Ext};
use crate::host::Host;
use crate::store::Store;
use crate::{Source, call};
use gpui::{AsyncApp, Context, SharedString, WeakEntity};
use serde_json::json;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct SourceInfo {
    pub id: String,
    pub name: String,
    pub lang: String,
    pub pkg: String,
    pub supports_latest: bool,
}

pub struct Registry {
    host: Arc<Host>,
    data_dir: PathBuf,
    pub sources: Vec<SourceInfo>,
    pub installed: HashSet<String>,
    /// True once every installed extension has been tried.
    pub ready: bool,
    pub busy: HashSet<String>,
    pub errors: std::collections::HashMap<String, String>,
    pub index: Option<Arc<Vec<Ext>>>,
    pub status: SharedString,
    fetching_index: bool,
}

impl Registry {
    pub fn new(host: Arc<Host>, data_dir: PathBuf, cx: &mut Context<Self>) -> Self {
        let this = Registry {
            host,
            data_dir,
            sources: vec![],
            installed: HashSet::new(),
            ready: false,
            busy: HashSet::new(),
            errors: Default::default(),
            index: None,
            status: "Loading extensions…".into(),
            fetching_index: false,
        };
        cx.spawn(async move |this, cx| {
            let (host, dir) = this.read_with(cx, |r, _| (r.host.clone(), r.data_dir.clone())).ok()?;
            let mut jars = { let dir = dir.clone(); cx.background_executor().spawn(async move { extensions::installed(&dir) }).await };
            if jars.is_empty() {
                // First run starts with MangaDex so there's something to read.
                let _ = this.update(cx, |r, cx| {
                    r.status = "Installing MangaDex…".into();
                    cx.notify();
                });
                let dir = dir.clone();
                let installed = cx
                    .background_executor()
                    .spawn(async move {
                        let index = extensions::fetch_index(&dir)?;
                        let ext = index.iter().find(|e| e.pkg == extensions::DEFAULT_PACKAGE).ok_or("MangaDex is not in the repo")?;
                        extensions::install(ext, &dir).map(|jar| (ext.pkg.clone(), jar))
                    })
                    .await;
                match installed {
                    Ok(jar) => jars.push(jar),
                    Err(e) => {
                        let _ = this.update(cx, |r, cx| {
                            r.status = e.into();
                            cx.notify();
                        });
                    }
                }
            }
            for (pkg, jar) in jars {
                load(&this, &host, cx, pkg, jar).await;
            }
            this.update(cx, |r, cx| {
                r.ready = true;
                if r.status.starts_with("Loading") || r.status.starts_with("Installing") {
                    r.status = "".into();
                }
                cx.notify();
            })
            .ok()
        })
        .detach();
        this
    }

    pub fn source(&self, id: &str) -> Option<Source> {
        self.sources.iter().find(|s| s.id == id).map(|s| Source { host: self.host.clone(), id: s.id.clone(), name: s.name.clone(), installed: true })
    }

    /// Falls back to a stand-in that opens the series from what was saved.
    pub fn source_or_saved(&self, id: &str, name: &str) -> Source {
        self.source(id).unwrap_or_else(|| Source { host: self.host.clone(), id: id.into(), name: name.into(), installed: false })
    }

    pub fn languages(&self, pkg: &str) -> Vec<String> {
        let mut langs: Vec<String> = self.sources.iter().filter(|s| s.pkg == pkg).map(|s| s.lang.clone()).collect();
        langs.sort_by_key(|l| (l != "all", l.clone()));
        langs.dedup();
        langs
    }

    /// The chosen languages, else English, else everything.
    pub fn enabled_languages(&self, pkg: &str, store: &Store) -> Vec<String> {
        if let Some(chosen) = store.languages(pkg) {
            return chosen.clone();
        }
        let all = self.languages(pkg);
        if all.iter().any(|l| l == "en") { vec!["en".into()] } else { all }
    }

    /// Each extension's sources in its enabled languages.
    pub fn browsable(&self, store: &Store) -> Vec<SourceInfo> {
        self.sources.iter().filter(|s| self.enabled_languages(&s.pkg, store).contains(&s.lang)).cloned().collect()
    }

    pub fn ensure_index(&mut self, cx: &mut Context<Self>) {
        if self.index.is_some() || self.fetching_index {
            return;
        }
        self.fetching_index = true;
        let dir = self.data_dir.clone();
        cx.spawn(async move |this, cx| {
            let cached = { let dir = dir.clone(); cx.background_executor().spawn(async move { extensions::cached_index(&dir) }).await };
            if let Some(list) = cached {
                let _ = this.update(cx, |r, cx| {
                    r.index = Some(Arc::new(list));
                    cx.notify();
                });
            }
            let fresh = cx.background_executor().spawn(async move { extensions::fetch_index(&dir) }).await;
            let _ = this.update(cx, |r, cx| {
                r.fetching_index = false;
                match fresh {
                    Ok(list) => r.index = Some(Arc::new(list)),
                    Err(e) => r.status = e.into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn install(&mut self, ext: Ext, cx: &mut Context<Self>) {
        if !self.busy.insert(ext.pkg.clone()) {
            return;
        }
        cx.notify();
        let (host, dir) = (self.host.clone(), self.data_dir.clone());
        cx.spawn(async move |this, cx| {
            let pkg = ext.pkg.clone();
            let jar = cx.background_executor().spawn(async move { extensions::install(&ext, &dir) }).await;
            match jar {
                Ok(jar) => load(&this, &host, cx, pkg.clone(), jar).await,
                Err(e) => {
                    let _ = this.update(cx, |r, _| r.status = e.into());
                }
            }
            let _ = this.update(cx, |r, cx| {
                r.busy.remove(&pkg);
                cx.notify();
            });
        })
        .detach();
    }

    /// The host keeps the classes loaded until the next start.
    pub fn uninstall(&mut self, pkg: &str, cx: &mut Context<Self>) {
        extensions::uninstall(pkg, &self.data_dir);
        self.installed.remove(pkg);
        self.sources.retain(|s| s.pkg != pkg);
        cx.notify();
    }
}

async fn load(this: &WeakEntity<Registry>, host: &Arc<Host>, cx: &mut AsyncApp, pkg: String, jar: PathBuf) {
    let result = call(host, cx, "load", json!({ "jar": jar, "signingKey": extensions::SIGNING_KEY })).await;
    let _ = this.update(cx, |r, cx| {
        r.installed.insert(pkg.clone());
        match result {
            Ok(list) => {
                r.errors.remove(&pkg);
                r.sources.retain(|s| s.pkg != pkg);
                r.sources.extend(list.as_array().into_iter().flatten().filter_map(|s| {
                    Some(SourceInfo {
                        id: s["id"].as_str()?.to_string(),
                        name: s["name"].as_str()?.to_string(),
                        lang: s["lang"].as_str().unwrap_or("").to_string(),
                        pkg: pkg.clone(),
                        supports_latest: s["supportsLatest"].as_bool().unwrap_or(true),
                    })
                }));
                r.sources.sort_by_key(|s| s.name.to_lowercase());
            }
            Err(e) => {
                eprintln!("{pkg}: {e}");
                r.errors.insert(pkg.clone(), e);
            }
        }
        cx.notify();
    });
}
