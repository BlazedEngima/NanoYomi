//! Chapter pages kept on disk up to a size cap. Past it the least recently read pages go first.
//! A file's modified time is when it was last read.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

static DIR: OnceLock<PathBuf> = OnceLock::new();
static CAP_BYTES: AtomicU64 = AtomicU64::new(1024 * 1024 * 1024);

pub const CAPS_MB: [u32; 4] = [256, 512, 1024, 2048];
pub const DEFAULT_MB: u32 = 1024;

pub fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

pub fn set_cap_mb(mb: u32) {
    CAP_BYTES.store(mb as u64 * 1024 * 1024, Ordering::Relaxed);
    if let Some(dir) = DIR.get() {
        evict(dir, mb as u64 * 1024 * 1024);
    }
}

pub fn clear() {
    if let Some(dir) = DIR.get() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Named by a hash of the page's JSON.
fn path(key: &str) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    DIR.get().map(|d| d.join(format!("{:016x}", h.finish())))
}

pub fn get(key: &str) -> Option<Vec<u8>> {
    let p = path(key)?;
    let bytes = std::fs::read(&p).ok()?;
    let _ = std::fs::File::options().write(true).open(&p).and_then(|f| f.set_modified(SystemTime::now()));
    Some(bytes)
}

/// Moves a download into the cache and trims it to the cap. Returns the original path if it couldn't be kept.
pub fn put(key: &str, downloaded: PathBuf) -> PathBuf {
    let Some(target) = path(key) else { return downloaded };
    let dir = target.parent().unwrap();
    let moved = std::fs::create_dir_all(dir)
        .and_then(|_| std::fs::rename(&downloaded, &target).or_else(|_| std::fs::copy(&downloaded, &target).map(|_| ())));
    if moved.is_err() {
        return downloaded;
    }
    let _ = std::fs::remove_file(&downloaded);
    let _ = std::fs::File::options().write(true).open(&target).and_then(|f| f.set_modified(SystemTime::now()));
    evict(dir, CAP_BYTES.load(Ordering::Relaxed));
    target
}

/// For entries that turned out not to be images.
pub fn remove(key: &str) {
    if let Some(p) = path(key) {
        let _ = std::fs::remove_file(p);
    }
}

// Lists the folder on every insert. Fine for thousands of pages.
fn evict(dir: &Path, cap: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.0);
    for (_, len, p) in files {
        if total <= cap {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
#[test]
fn evicts_least_recently_used() {
    let dir = std::env::temp_dir().join(format!("nano-page-cache-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t = SystemTime::now();
    for (name, age) in [("old", 30), ("mid", 20), ("new", 10)] {
        let p = dir.join(name);
        std::fs::write(&p, [0u8; 100]).unwrap();
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(t - std::time::Duration::from_secs(age)).unwrap();
    }
    evict(&dir, 200);
    assert!(!dir.join("old").exists() && dir.join("mid").exists() && dir.join("new").exists());
    let _ = std::fs::remove_dir_all(dir);
}
