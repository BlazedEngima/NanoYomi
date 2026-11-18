//! Extensions from the keiyoushi repo. Every jar's signature is checked against a key pinned here, not one read from the index.
//! Everything here blocks, so run it off the main thread.

use serde_json::Value;
use std::path::{Path, PathBuf};

const INDEX_URL: &str = "https://raw.githubusercontent.com/keiyoushi/extensions/repo/index.json";
/// SHA-256 of keiyoushi's signing certificate.
pub const SIGNING_KEY: &str = "9add655a78e96c4ec7a53ef89dccb557cb5d767489fac5e785d671a5a75d4da2";
pub const DEFAULT_PACKAGE: &str = "eu.kanade.tachiyomi.extension.all.mangadex";

#[derive(Clone, Debug)]
pub struct Ext {
    pub pkg: String,
    pub name: String,
    pub version: String,
    /// "all" for multi-language extensions.
    pub langs: Vec<String>,
    pub nsfw: bool,
    pub icon_url: String,
    jar_url: String,
}

/// Extension icons come from a public CDN, not through a source.
pub fn fetch_icon(url: &str) -> Result<Vec<u8>, String> {
    let mut res = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?;
    res.body_mut().with_config().limit(1024 * 1024).read_to_vec().map_err(|e| e.to_string())
}

pub fn fetch_index(data_dir: &Path) -> Result<Vec<Ext>, String> {
    let text = get(INDEX_URL)?.into_string()?;
    let list = parse_index(&text)?;
    let _ = std::fs::write(data_dir.join("index.json"), text);
    Ok(list)
}

pub fn cached_index(data_dir: &Path) -> Option<Vec<Ext>> {
    parse_index(&std::fs::read_to_string(data_dir.join("index.json")).ok()?).ok()
}

fn parse_index(text: &str) -> Result<Vec<Ext>, String> {
    let index: Value = serde_json::from_str(text).map_err(|e| format!("repo index: {e}"))?;
    check_key(index["signingKey"].as_str())?;
    let mut list: Vec<Ext> = index["extensionList"]["extensions"]
        .as_array()
        .ok_or("repo index has no extensions")?
        .iter()
        .filter_map(|e| {
            let mut langs: Vec<String> =
                e["sources"].as_array()?.iter().filter_map(|s| s["language"].as_str().map(str::to_string)).collect();
            langs.sort();
            langs.dedup();
            if langs.len() > 3 {
                langs = vec!["all".into()];
            }
            Some(Ext {
                pkg: e["packageName"].as_str()?.to_string(),
                name: e["name"].as_str()?.trim_start_matches("Tachiyomi: ").to_string(),
                version: e["versionName"].as_str().unwrap_or("0").to_string(),
                langs,
                nsfw: e["contentWarning"] == "CONTENT_WARNING_NSFW",
                icon_url: e["resources"]["iconUrl"].as_str().unwrap_or_default().to_string(),
                jar_url: e["resources"]["jarUrl"].as_str()?.to_string(),
            })
        })
        .collect();
    list.sort_by_key(|e| e.name.to_lowercase());
    Ok(list)
}

fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("extensions")
}

/// Downloads the jar unless this version is already here, and removes older versions.
pub fn install(ext: &Ext, data_dir: &Path) -> Result<PathBuf, String> {
    let dir = dir(data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let jar = dir.join(format!("{}-{}.jar", ext.pkg, ext.version));
    if !jar.exists() {
        let bytes = get(&ext.jar_url)?.into_bytes()?;
        let tmp = jar.with_extension("part");
        std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &jar)).map_err(|e| e.to_string())?;
    }
    for (pkg, other) in installed(data_dir) {
        if pkg == ext.pkg && other != jar {
            let _ = std::fs::remove_file(other);
        }
    }
    Ok(jar)
}

pub fn uninstall(pkg: &str, data_dir: &Path) {
    for (p, jar) in installed(data_dir) {
        if p == pkg {
            let _ = std::fs::remove_file(jar);
        }
    }
}

/// Installed jars, named `<package>-<version>.jar`.
pub fn installed(data_dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir(data_dir)) else { return vec![] };
    let mut list: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter_map(|p| Some((package_of(p.file_name()?.to_str()?)?, p)))
        .collect();
    list.sort();
    list
}

fn package_of(file: &str) -> Option<String> {
    Some(file.strip_suffix(".jar")?.rsplit_once('-')?.0.to_string())
}

/// If the index names any other key, the repo changed hands or was tampered with.
fn check_key(listed: Option<&str>) -> Result<(), String> {
    match listed {
        Some(k) if k.eq_ignore_ascii_case(SIGNING_KEY) => Ok(()),
        Some(k) => Err(format!(
            "The keiyoushi repo now names a different signing key ({k}). Nothing was installed; \
             this can mean the repo was compromised."
        )),
        None => Err("repo index has no signingKey".into()),
    }
}

fn get(url: &str) -> Result<Body, String> {
    ureq::get(url).call().map(Body).map_err(|e| format!("{url}: {e}"))
}

struct Body(ureq::http::Response<ureq::Body>);

impl Body {
    fn into_bytes(mut self) -> Result<Vec<u8>, String> {
        self.0.body_mut().with_config().limit(64 * 1024 * 1024).read_to_vec().map_err(|e| e.to_string())
    }
    fn into_string(mut self) -> Result<String, String> {
        self.0.body_mut().with_config().limit(64 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_pinned_key_is_accepted() {
        assert!(check_key(Some(SIGNING_KEY)).is_ok());
        assert!(check_key(Some(&SIGNING_KEY.to_uppercase())).is_ok());
        assert!(check_key(Some("00".repeat(32).as_str())).unwrap_err().contains("different signing key"));
        assert!(check_key(None).is_err());
    }

    #[test]
    fn parses_the_index_and_jar_names() {
        let index = serde_json::json!({
            "signingKey": SIGNING_KEY,
            "extensionList": { "extensions": [
                { "name": "Tachiyomi: Zeta", "packageName": "a.zeta", "versionName": "1.4.2",
                  "contentWarning": "CONTENT_WARNING_NSFW", "resources": { "jarUrl": "https://x/z.jar" },
                  "sources": [{ "language": "en" }, { "language": "es" }, { "language": "en" }] },
                { "name": "Alpha", "packageName": "a.alpha", "versionName": "1.0",
                  "contentWarning": "CONTENT_WARNING_SAFE", "resources": { "jarUrl": "https://x/a.jar" },
                  "sources": [{ "language": "fr" }, { "language": "de" }, { "language": "it" }, { "language": "pt" }] }
            ]}
        });
        let list = parse_index(&index.to_string()).unwrap();
        assert_eq!(list.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Alpha", "Zeta"]);
        assert_eq!(list[0].langs, ["all"]);
        assert_eq!(list[1].langs, ["en", "es"]);
        assert!(list[1].nsfw && !list[0].nsfw);
        assert_eq!(package_of("eu.kanade.tachiyomi.extension.all.mangadex-1.6.4.jar").unwrap(), "eu.kanade.tachiyomi.extension.all.mangadex");
        assert_eq!(package_of("x.part"), None);
    }
}
