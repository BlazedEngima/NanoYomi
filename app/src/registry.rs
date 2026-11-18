//! Installed extensions and their sources. Every jar is loaded into the host at startup.

use crate::extensions::{self, Ext, Repo};
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
    /// Installed version per package, for extensions no added repo lists.
    pub versions: std::collections::HashMap<String, String>,
    /// True once every installed extension has been tried.
    pub ready: bool,
    pub busy: HashSet<String>,
    pub errors: std::collections::HashMap<String, String>,
    pub index: Option<Arc<Vec<Ext>>>,
    pub status: SharedString,
    pub index_errors: Vec<String>,
}

impl Registry {
    pub fn new(host: Arc<Host>, data_dir: PathBuf, cx: &mut Context<Self>) -> Self {
        let this = Registry {
            host,
            data_dir,
            sources: vec![],
            installed: HashSet::new(),
            versions: Default::default(),
            ready: false,
            busy: HashSet::new(),
            errors: Default::default(),
            index: None,
            status: "Loading extensions…".into(),
            index_errors: vec![],
        };
        cx.spawn(async move |this, cx| {
            let (host, dir) = this.read_with(cx, |r, _| (r.host.clone(), r.data_dir.clone())).ok()?;
            let jars = { let dir = dir.clone(); cx.background_executor().spawn(async move { extensions::installed(&dir) }).await };
            for (pkg, jar) in jars {
                load(&this, &host, cx, pkg, jar).await;
            }
            this.update(cx, |r, cx| {
                r.ready = true;
                if r.status.starts_with("Loading") {
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

    /// Shows the cached lists right away, then fetches fresh ones. Call again after the repos change.
    pub fn refresh_index(&mut self, repos: Vec<Repo>, cx: &mut Context<Self>) {
        let dir = self.data_dir.clone();
        cx.spawn(async move |this, cx| {
            let cached = { let (dir, repos) = (dir.clone(), repos.clone()); cx.background_executor().spawn(async move { extensions::cached_index(&repos, &dir) }).await };
            let _ = this.update(cx, |r, cx| {
                r.index = Some(Arc::new(cached));
                cx.notify();
            });
            let (fresh, errors) = cx.background_executor().spawn(async move { extensions::fetch_index(&repos, &dir) }).await;
            let _ = this.update(cx, |r, cx| {
                r.index = Some(Arc::new(fresh));
                r.index_errors = errors;
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
        self.versions.remove(pkg);
        self.sources.retain(|s| s.pkg != pkg);
        cx.notify();
    }
}

async fn load(this: &WeakEntity<Registry>, host: &Arc<Host>, cx: &mut AsyncApp, pkg: String, jar: PathBuf) {
    let key = extensions::key_for(&jar);
    let version = extensions::version_of(&jar);
    let result = call(host, cx, "load", json!({ "jar": jar, "signingKey": key })).await;
    let _ = this.update(cx, |r, cx| {
        r.installed.insert(pkg.clone());
        r.versions.insert(pkg.clone(), version);
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
