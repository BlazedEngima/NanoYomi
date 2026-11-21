//! Three font slots, each with a bundled default. Other fonts come from Google Fonts into the data folder.

use gpui::{App, SharedString};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::Duration;

pub const SLOTS: [(&str, &str, &str); 3] = [
    ("display", "Headings", "Big Shoulders Display"),
    ("sans", "Text", "IBM Plex Sans"),
    ("mono", "Labels and numbers", "IBM Plex Mono"),
];

pub fn google() -> &'static [&'static str] {
    static LIST: OnceLock<Vec<&'static str>> = OnceLock::new();
    LIST.get_or_init(|| include_str!("../fonts/google-fonts.txt").lines().collect())
}

static DIR: OnceLock<PathBuf> = OnceLock::new();
static CHOSEN: RwLock<[Option<String>; 3]> = RwLock::new([None, None, None]);
static LOADED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

pub fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

pub fn dir() -> PathBuf {
    DIR.get().cloned().unwrap_or_default()
}

pub fn choose(chosen: &HashMap<String, String>) {
    *CHOSEN.write().unwrap() = SLOTS.map(|(key, ..)| chosen.get(key).cloned());
}

fn family(slot: usize) -> SharedString {
    CHOSEN.read().unwrap()[slot].clone().unwrap_or_else(|| SLOTS[slot].2.to_string()).into()
}

pub fn display() -> SharedString {
    family(0)
}

pub fn sans() -> SharedString {
    family(1)
}

pub fn mono() -> SharedString {
    family(2)
}

pub fn files() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir()) else { return vec![] };
    let names = entries.flatten().filter_map(|e| e.file_name().into_string().ok()).map(|n| n.to_lowercase());
    names.filter(|n| n.ends_with(".ttf") || n.ends_with(".otf")).collect()
}

pub fn installed(files: &[String], family: &str) -> bool {
    let want = family.replace(' ', "").to_lowercase();
    files.iter().any(|f| f.split(['-', '[', '.']).next() == Some(&want))
}

pub fn load(cx: &App) {
    let Ok(entries) = std::fs::read_dir(dir()) else { return };
    let mut loaded = LOADED.lock().unwrap();
    let new: Vec<(PathBuf, Vec<u8>)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "ttf" || x == "otf") && !loaded.contains(p))
        .filter_map(|p| std::fs::read(&p).ok().map(|b| (p, b)))
        .collect();
    if new.is_empty() {
        return;
    }
    let bytes = new.iter().map(|(_, b)| Cow::Owned(b.clone())).collect();
    match cx.text_system().add_fonts(bytes) {
        Ok(()) => loaded.extend(new.into_iter().map(|(p, _)| p)),
        Err(e) => eprintln!("fonts: {e}"),
    }
}

fn faces(css: &str) -> Vec<(u32, String)> {
    css.split("@font-face")
        .skip(1)
        .filter_map(|block| {
            let weight = block.split("font-weight:").nth(1)?.trim_start().split(';').next()?.trim().parse().ok()?;
            let url = block.split("url(").nth(1)?.split(')').next()?.to_string();
            Some((weight, url))
        })
        .collect()
}

pub fn download(name: &str) -> Result<usize, String> {
    let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build());
    let family = name.replace(' ', "+");
    // Not every family has every weight
    let css = [format!("{family}:400,500,600,700"), family]
        .iter()
        .find_map(|f| agent.get(format!("https://fonts.googleapis.com/css?family={f}")).call().ok())
        .ok_or("couldn't reach Google Fonts")?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let faces = faces(&css);
    if faces.is_empty() {
        return Err("Google Fonts has no family by that name".into());
    }
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    for (weight, url) in &faces {
        let bytes = agent.get(url).call().map_err(|e| e.to_string())?.body_mut().read_to_vec().map_err(|e| e.to_string())?;
        let file = dir().join(format!("{}-{weight}.ttf", name.replace(' ', "")));
        std::fs::write(file, bytes).map_err(|e| e.to_string())?;
    }
    Ok(faces.len())
}

pub fn folder() -> String {
    Path::new(&dir()).display().to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn installed_matches_whole_families() {
        let files = vec!["robotomono-400.ttf".to_string(), "inter[opsz,wght].ttf".to_string()];
        assert!(super::installed(&files, "Roboto Mono") && super::installed(&files, "Inter"));
        assert!(!super::installed(&files, "Roboto") && !super::installed(&files, "Google Sans"));
    }

    #[test]
    fn the_google_list_has_google_sans() {
        assert!(super::google().len() > 1500 && super::google().contains(&"Google Sans"));
    }

    #[test]
    fn reads_weights_and_urls_from_google_css() {
        let css = "@font-face {\n  font-weight: 400;\n  src: url(https://x/a.ttf) format('truetype');\n}\n@font-face {\n  font-weight: 700;\n  src: url(https://x/b.ttf) format('truetype');\n}";
        assert_eq!(super::faces(css), vec![(400, "https://x/a.ttf".into()), (700, "https://x/b.ttf".into())]);
        assert!(super::faces("nope").is_empty());
    }
}
