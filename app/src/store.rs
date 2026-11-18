//! What Nano remembers between runs, in one JSON file saved a second after each change.

use gpui::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone)]
pub struct Series {
    pub source: String,
    pub source_name: String,
    pub manga: Value,
    pub in_library: bool,
    pub added_at: i64,
    pub chapters: Vec<Value>,
    pub progress: Option<Progress>,
    /// Hidden scanlators (or languages, on multi-language sources).
    #[serde(default)]
    pub hidden_scanlators: Vec<String>,
    /// Until the user picks, only the main scanlator shows.
    #[serde(default)]
    pub scanlators_chosen: bool,
    #[serde(default)]
    pub sync: Option<crate::sync::Meta>,
    /// Chapters marked read (true) or unread (false) by hand, by url. They win over what the progress implies.
    #[serde(default)]
    pub marks: std::collections::BTreeMap<String, bool>,
    /// Bookmarked chapters, by url.
    #[serde(default)]
    pub bookmarks: std::collections::BTreeSet<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Progress {
    pub chapter_url: String,
    pub chapter_name: String,
    pub page: usize,
    pub pages: usize,
    pub at: i64,
}

impl Series {
    pub fn new(source: String, source_name: String, manga: Value) -> Self {
        Series {
            source,
            source_name,
            manga,
            in_library: false,
            added_at: 0,
            chapters: vec![],
            progress: None,
            hidden_scanlators: vec![],
            scanlators_chosen: false,
            sync: None,
            marks: Default::default(),
            bookmarks: Default::default(),
        }
    }

    /// In the library, read, or with an unsynced change.
    fn kept(&self) -> bool {
        self.in_library || self.progress.is_some() || !self.marks.is_empty() || !self.bookmarks.is_empty() || crate::sync::unsent(self)
    }

    pub fn url(&self) -> &str {
        self.manga["url"].as_str().unwrap_or_default()
    }

    pub fn title(&self) -> &str {
        self.manga["title"].as_str().unwrap_or("Untitled")
    }

    pub fn visible(&self) -> Vec<Value> {
        visible_chapters(&self.chapters, &self.hidden())
    }

    pub fn hidden(&self) -> Vec<String> {
        if self.scanlators_chosen {
            self.hidden_scanlators.clone()
        } else {
            default_hidden(&self.chapters, self.progress.as_ref().map(|p| p.chapter_url.as_str()))
        }
    }

    pub fn reading_index(&self) -> Option<usize> {
        let p = self.progress.as_ref()?;
        self.visible().iter().position(|c| c["url"] == p.chapter_url.as_str())
    }

    /// How far through the series you are, from 0 to 1. The current chapter counts by page.
    pub fn read_fraction(&self) -> Option<f32> {
        let p = self.progress.as_ref()?;
        let total = self.visible().len();
        let i = self.reading_index()?;
        let page = if p.pages == 0 { 0. } else { ((p.page + 1) as f32 / p.pages as f32).min(1.) };
        Some(((total - i - 1) as f32 + page) / total as f32)
    }

    /// Whether each chapter in `list` (newest first) is read: older than the one being read, finished, or marked by hand.
    pub fn read_flags(&self, list: &[Value]) -> Vec<bool> {
        let p = self.progress.as_ref();
        let reading = p.and_then(|p| list.iter().position(|c| c["url"] == p.chapter_url.as_str()));
        let finished = p.is_some_and(|p| p.pages > 0 && p.page + 1 >= p.pages);
        list.iter()
            .enumerate()
            .map(|(i, c)| match self.marks.get(c["url"].as_str().unwrap_or_default()) {
                Some(read) => *read,
                None => reading.is_some_and(|r| i > r || (i == r && finished)),
            })
            .collect()
    }

    /// Whether each chapter counts as new: newer than the one being read, or marked unread.
    /// An unstarted series has nothing new, so its whole list isn't a backlog.
    pub fn new_flags(&self, list: &[Value]) -> Vec<bool> {
        let reading = self.progress.as_ref().and_then(|p| list.iter().position(|c| c["url"] == p.chapter_url.as_str()));
        list.iter()
            .enumerate()
            .map(|(i, c)| match self.marks.get(c["url"].as_str().unwrap_or_default()) {
                Some(read) => !read,
                None => reading.is_some_and(|r| i < r),
            })
            .collect()
    }

    pub fn caught_up(&self) -> bool {
        let list = self.visible();
        (self.progress.is_some() || !self.marks.is_empty()) && !list.is_empty() && self.read_flags(&list).iter().all(|r| *r)
    }

    pub fn unread(&self) -> usize {
        self.new_flags(&self.visible()).iter().filter(|n| **n).count()
    }

    /// Marks chapters read or unread by hand.
    pub fn mark(&mut self, urls: &[String], read: bool) {
        for url in urls {
            self.marks.insert(url.clone(), read);
        }
    }

    pub fn toggle_bookmark(&mut self, url: &str) {
        if !self.bookmarks.remove(url) {
            self.bookmarks.insert(url.to_string());
        }
    }

    fn last_active(&self) -> i64 {
        self.progress.as_ref().map_or(self.added_at, |p| p.at.max(self.added_at))
    }
}

pub fn scanlator(c: &Value) -> &str {
    c["scanlator"].as_str().filter(|s| !s.is_empty()).unwrap_or("Unknown")
}

pub fn scanlator_counts(chapters: &[Value]) -> Vec<(String, usize)> {
    let mut groups: Vec<(String, usize)> = vec![];
    for c in chapters {
        let s = scanlator(c);
        match groups.iter_mut().find(|(g, _)| g == s) {
            Some((_, n)) => *n += 1,
            None => groups.push((s.to_string(), 1)),
        }
    }
    groups.sort_by_key(|(g, n)| (std::cmp::Reverse(*n), g.clone()));
    groups
}

/// Before the user picks, show the group of the chapter being read, or else the biggest group.
pub fn default_hidden(chapters: &[Value], reading_url: Option<&str>) -> Vec<String> {
    let groups = scanlator_counts(chapters);
    let reading = reading_url.and_then(|u| chapters.iter().find(|c| c["url"] == u)).map(scanlator);
    let Some(keep) = reading.or(groups.first().map(|(g, _)| g.as_str())) else { return vec![] };
    groups.iter().filter(|(g, _)| g != keep).map(|(g, _)| g.clone()).collect()
}

pub fn visible_chapters(chapters: &[Value], hidden: &[String]) -> Vec<Value> {
    chapters.iter().filter(|c| !hidden.iter().any(|h| h == scanlator(c))).cloned().collect()
}

#[derive(Serialize, Deserialize, Default)]
struct Data {
    series: Vec<Series>,
    #[serde(default)]
    languages: std::collections::HashMap<String, Vec<String>>,
    #[serde(default)]
    zoom: Option<f32>,
    #[serde(default)]
    solver_url: String,
    #[serde(default)]
    page_cache_mb: Option<u32>,
    /// Unset means sized for this machine.
    #[serde(default)]
    host_heap_mb: Option<u32>,
    #[serde(default)]
    vertical_reader: bool,
    #[serde(default)]
    sync: crate::sync::Config,
    #[serde(default)]
    sidebar_collapsed: bool,
    #[serde(default)]
    repos: Vec<crate::extensions::Repo>,
    #[serde(default)]
    auto_download: bool,
}

#[derive(Clone)]
pub enum SyncStatus {
    Idle,
    Syncing,
    Done(i64),
    Failed(String),
}

pub struct Store {
    path: PathBuf,
    data: Data,
    saving: bool,
    pub sync_status: SyncStatus,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

impl Store {
    pub fn load(path: PathBuf) -> Self {
        let data = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).map_err(|e| eprintln!("{}: {e}", path.display())).ok())
            .unwrap_or_default();
        Store { path, data, saving: false, sync_status: SyncStatus::Idle }
    }

    pub fn get(&self, source: &str, url: &str) -> Option<&Series> {
        self.data.series.iter().find(|s| s.source == source && s.url() == url)
    }

    pub fn library(&self) -> Vec<&Series> {
        let mut list: Vec<_> = self.data.series.iter().filter(|s| s.in_library).collect();
        list.sort_by_key(|s| -s.last_active());
        list
    }

    pub fn history(&self) -> Vec<&Series> {
        let mut list: Vec<_> = self.data.series.iter().filter(|s| s.progress.is_some()).collect();
        list.sort_by_key(|s| -s.progress.as_ref().map_or(0, |p| p.at));
        list
    }

    pub fn update<T>(
        &mut self,
        source: &str,
        source_name: &str,
        manga: &Value,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Series) -> T,
    ) -> T {
        let url = manga["url"].as_str().unwrap_or_default();
        let ix = match self.data.series.iter().position(|s| s.source == source && s.url() == url) {
            Some(ix) => ix,
            None => {
                self.data.series.push(Series::new(source.into(), source_name.into(), manga.clone()));
                self.data.series.len() - 1
            }
        };
        let out = f(&mut self.data.series[ix]);
        self.data.series.retain(Series::kept);
        self.save_soon(cx);
        cx.notify();
        out
    }

    pub fn dir(&self) -> std::path::PathBuf {
        self.path.parent().map(|p| p.to_path_buf()).unwrap_or_default()
    }

    pub fn all_series(&self) -> &[Series] {
        &self.data.series
    }

    pub fn edit_series(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Vec<Series>)) {
        f(&mut self.data.series);
        self.data.series.retain(Series::kept);
        self.save_soon(cx);
        cx.notify();
    }

    pub fn sync_config(&self) -> &crate::sync::Config {
        &self.data.sync
    }

    /// A new server or key starts over.
    pub fn set_sync_account(&mut self, host: &str, key: &str, cx: &mut Context<Self>) {
        let (host, key) = (host.trim().trim_end_matches('/'), key.trim());
        if self.data.sync.host == host && self.data.sync.key == key {
            return;
        }
        self.data.sync.host = host.into();
        self.data.sync.key = key.into();
        self.data.sync.cursor = 0;
        self.edit_series(cx, |series| series.iter_mut().for_each(|s| s.sync = None));
    }

    pub fn set_sync_cursor(&mut self, cursor: i64, cx: &mut Context<Self>) {
        self.data.sync.cursor = cursor;
        self.save_soon(cx);
    }

    pub fn start_sync(&mut self, cx: &mut Context<Self>) -> Option<crate::sync::Config> {
        let c = &mut self.data.sync;
        if matches!(self.sync_status, SyncStatus::Syncing) || c.host.is_empty() || c.key.is_empty() {
            return None;
        }
        if c.device_id.is_empty() {
            c.device_id = format!("nano-{:x}-{:x}", now_ms(), std::process::id());
        }
        self.sync_status = SyncStatus::Syncing;
        cx.notify();
        Some(self.data.sync.clone())
    }

    pub fn finish_sync(&mut self, result: Result<usize, String>, cx: &mut Context<Self>) {
        self.sync_status = match result {
            Ok(_) => SyncStatus::Done(now_ms()),
            Err(e) => SyncStatus::Failed(e),
        };
        cx.notify();
    }

    pub fn zoom(&self) -> f32 {
        self.data.zoom.unwrap_or(1.)
    }

    pub fn set_zoom(&mut self, z: f32, cx: &mut Context<Self>) {
        self.data.zoom = Some(z);
        self.save_soon(cx);
    }

    pub fn repos(&self) -> &[crate::extensions::Repo] {
        &self.data.repos
    }

    pub fn add_repo(&mut self, repo: crate::extensions::Repo, cx: &mut Context<Self>) {
        self.data.repos.retain(|r| r.url != repo.url);
        self.data.repos.push(repo);
        self.save_soon(cx);
    }

    pub fn remove_repo(&mut self, url: &str, cx: &mut Context<Self>) {
        self.data.repos.retain(|r| r.url != url);
        self.save_soon(cx);
    }

    pub fn auto_download(&self) -> bool {
        self.data.auto_download
    }

    pub fn set_auto_download(&mut self, on: bool, cx: &mut Context<Self>) {
        self.data.auto_download = on;
        self.save_soon(cx);
        cx.notify();
    }

    pub fn sidebar_collapsed(&self) -> bool {
        self.data.sidebar_collapsed
    }

    pub fn set_sidebar_collapsed(&mut self, on: bool, cx: &mut Context<Self>) {
        self.data.sidebar_collapsed = on;
        self.save_soon(cx);
    }

    pub fn solver_url(&self) -> &str {
        &self.data.solver_url
    }

    pub fn set_solver_url(&mut self, url: &str, cx: &mut Context<Self>) {
        if self.data.solver_url != url {
            self.data.solver_url = url.to_string();
            crate::solver::set_url(url);
            self.save_soon(cx);
        }
    }

    pub fn vertical_reader(&self) -> bool {
        self.data.vertical_reader
    }

    pub fn set_vertical_reader(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.data.vertical_reader != on {
            self.data.vertical_reader = on;
            self.save_soon(cx);
            cx.notify();
        }
    }

    pub fn host_heap_mb(&self) -> Option<u32> {
        self.data.host_heap_mb
    }

    pub fn set_host_heap_mb(&mut self, mb: Option<u32>, cx: &mut Context<Self>) {
        self.data.host_heap_mb = mb;
        self.save_soon(cx);
        cx.notify();
    }

    pub fn page_cache_mb(&self) -> u32 {
        self.data.page_cache_mb.unwrap_or(crate::page_cache::DEFAULT_MB)
    }

    pub fn set_page_cache_mb(&mut self, mb: u32, cx: &mut Context<Self>) {
        self.data.page_cache_mb = Some(mb);
        crate::page_cache::set_cap_mb(mb);
        self.save_soon(cx);
        cx.notify();
    }

        pub fn languages(&self, pkg: &str) -> Option<&Vec<String>> {
        self.data.languages.get(pkg)
    }

    pub fn set_languages(&mut self, pkg: &str, langs: Vec<String>, cx: &mut Context<Self>) {
        self.data.languages.insert(pkg.to_string(), langs);
        self.save_soon(cx);
        cx.notify();
    }

    fn save_soon(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.saving = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let Ok((path, bytes)) = this.update(cx, |s, _| {
                s.saving = false;
                (s.path.clone(), serde_json::to_vec(&s.data).unwrap_or_default())
            }) else {
                return;
            };
            let tmp = path.with_extension("tmp");
            if let Err(e) = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &path)) {
                eprintln!("save {}: {e}", path.display());
            }
        })
        .detach();
    }
}

pub fn ago(ms: i64, now: i64) -> String {
    let mins = (now - ms).max(0) / 60_000;
    match mins {
        0 => "just now".into(),
        1..60 => format!("{mins} min ago"),
        60..1440 => format!("{} h ago", mins / 60),
        1440..2880 => "Yesterday".into(),
        2880..10080 => format!("{} days ago", mins / 1440),
        _ => crate::manga::ymd(ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn relative_times() {
        let now = 1_763_337_600_000;
        assert_eq!(ago(now - 30_000, now), "just now");
        assert_eq!(ago(now - 5 * 60_000, now), "5 min ago");
        assert_eq!(ago(now - 3 * 3_600_000, now), "3 h ago");
        assert_eq!(ago(now - 30 * 3_600_000, now), "Yesterday");
        assert_eq!(ago(now - 4 * 86_400_000, now), "4 days ago");
        assert_eq!(ago(now - 40 * 86_400_000, now), "2025-10-08");
    }

    #[test]
    fn unread_counts_chapters_newer_than_the_one_being_read() {
        let mut s = Series::new("1".into(), "S".into(), json!({ "url": "/m" }));
        s.in_library = true;
        s.chapters = (0..5).rev().map(|i| json!({ "url": format!("/c{i}") })).collect();
        assert_eq!(s.unread(), 0);
        s.progress = Some(Progress { chapter_url: "/c1".into(), chapter_name: "".into(), page: 0, pages: 1, at: 0 });
        assert_eq!(s.unread(), 3); // c4, c3, c2
        s.progress = Some(Progress { chapter_url: "/c1".into(), chapter_name: "".into(), page: 0, pages: 2, at: 0 });
        assert_eq!(s.read_fraction(), Some(0.3));
        assert!(!s.caught_up());
        s.progress = Some(Progress { chapter_url: "/c4".into(), chapter_name: "".into(), page: 1, pages: 2, at: 0 });
        assert!(s.caught_up());
    }

    #[test]
    fn marks_override_what_progress_implies() {
        let mut s = Series::new("1".into(), "S".into(), json!({ "url": "/m" }));
        s.chapters = (0..5).rev().map(|i| json!({ "url": format!("/c{i}") })).collect();
        s.progress = Some(Progress { chapter_url: "/c2".into(), chapter_name: "".into(), page: 0, pages: 4, at: 0 });
        assert_eq!(s.read_flags(&s.chapters), [false, false, false, true, true]); // c4 c3 c2 c1 c0
        assert_eq!(s.unread(), 2);
        s.mark(&["/c4".into(), "/c3".into()], true);
        s.mark(&["/c0".into()], false);
        assert_eq!(s.read_flags(&s.chapters), [true, true, false, true, false]);
        assert_eq!(s.new_flags(&s.chapters), [false, false, false, false, true]);
        assert_eq!(s.unread(), 1);
        assert!(!s.caught_up());
        s.mark(&["/c2".into(), "/c0".into()], true);
        assert!(s.caught_up());
        s.toggle_bookmark("/c3");
        assert!(s.bookmarks.contains("/c3"));
        s.toggle_bookmark("/c3");
        assert!(s.bookmarks.is_empty());
    }

    #[test]
    fn default_filter_keeps_the_main_or_current_group() {
        let ch = |url: &str, g: &str| json!({ "url": url, "scanlator": g });
        let chapters = vec![ch("/1", "A"), ch("/2", "B"), ch("/3", "B"), ch("/4", "")];
        assert_eq!(default_hidden(&chapters, None), vec!["A".to_string(), "Unknown".to_string()]);
        assert_eq!(default_hidden(&chapters, Some("/1")), vec!["B".to_string(), "Unknown".to_string()]);
        assert!(default_hidden(&[], None).is_empty());
    }
}
