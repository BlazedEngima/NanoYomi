//! The library on disk as SQLite, with a row per series and each chapter list in a table of its own.

use super::{Data, Series};
use rusqlite::{Connection, params};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

type Res<T> = Result<T, Box<dyn std::error::Error>>;
type Chapters = Arc<Vec<Value>>;

/// What a series row looks like on disk, so a save can skip what hasn't changed.
struct Saved {
    pos: usize,
    body: u64,
    chapters: Chapters,
}

pub struct Db {
    path: PathBuf,
    conn: Option<Connection>,
    series: HashMap<String, Saved>,
    config: HashMap<String, u64>,
}

fn hash(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// A row key per series, counting repeats so two with the same source and url, or none, both survive.
fn keys(series: &[Series]) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    series
        .iter()
        .map(|s| {
            let base = format!("{}\u{1}{}", s.source, s.url());
            let n = seen.entry(base.clone()).or_insert(0);
            *n += 1;
            if *n == 1 { base } else { format!("{base}\u{1}{n}") }
        })
        .collect()
}

/// Copies the library to `to` as a consistent snapshot, replacing the last one only once the new one checks out.
pub fn backup(from: &Path, to: &Path) -> Res<()> {
    let tmp = to.with_extension("bak.new");
    let _ = std::fs::remove_file(&tmp);
    Connection::open(from)?.execute("VACUUM INTO ?1", params![tmp.to_string_lossy()])?;
    let verdict: String = Connection::open(&tmp)?.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if verdict != "ok" {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("the copy failed its check: {verdict}").into());
    }
    let _ = std::fs::remove_file(to);
    std::fs::rename(&tmp, to)?;
    Ok(())
}

impl Db {
    pub fn new(path: PathBuf) -> Self {
        Db { path, conn: None, series: HashMap::new(), config: HashMap::new() }
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    fn open(&mut self) -> Res<&mut Connection> {
        if self.conn.is_none() {
            let conn = Connection::open(&self.path)?;
            let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
            conn.pragma_update(None, "synchronous", "NORMAL")?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS config (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS series (key TEXT PRIMARY KEY, pos INTEGER NOT NULL, body TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS chapters (key TEXT PRIMARY KEY REFERENCES series(key) ON DELETE CASCADE, list TEXT NOT NULL);
                 CREATE INDEX IF NOT EXISTS series_pos ON series(pos);",
            )?;
            self.conn = Some(conn);
        }
        Ok(self.conn.as_mut().unwrap())
    }

    /// Reads everything back, or None when there is no database yet.
    pub fn load(&mut self) -> Res<Option<Data>> {
        if !self.exists() {
            return Ok(None);
        }
        let conn = self.open()?;
        let mut cfg = Map::new();
        let mut hashes = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT key, value FROM config")?;
            for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                let (k, v) = row?;
                hashes.insert(k.clone(), hash(&v));
                cfg.insert(k, serde_json::from_str(&v)?);
            }
        }
        let mut data: Data = serde_json::from_value(Value::Object(cfg))?;
        let mut saved = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT s.key, s.pos, s.body, c.list FROM series s LEFT JOIN chapters c ON c.key = s.key ORDER BY s.pos")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?)))?;
            for row in rows {
                let (key, pos, body, list) = row?;
                let mut s: Series = serde_json::from_str(&body)?;
                if let Some(list) = list {
                    s.chapters = Arc::new(serde_json::from_str(&list)?);
                }
                saved.insert(key, Saved { pos: pos as usize, body: hash(&body), chapters: s.chapters.clone() });
                data.series.push(s);
            }
        }
        self.config = hashes;
        self.series = saved;
        Ok(Some(data))
    }

    /// Writes what changed since the last save and answers how many rows that took.
    pub fn save(&mut self, mut data: Data) -> Res<usize> {
        let mut series = std::mem::take(&mut data.series);
        let Value::Object(cfg) = serde_json::to_value(&data)? else { return Ok(0) };
        let keys = keys(&series);
        let empty: Chapters = Arc::default();
        // The chapter lists are saved on their own, so the body is the series without them.
        let bodies: Vec<String> = series
            .iter_mut()
            .map(|s| {
                let chapters = std::mem::replace(&mut s.chapters, empty.clone());
                let body = serde_json::to_string(&*s);
                s.chapters = chapters;
                body
            })
            .collect::<Result<_, _>>()?;
        let (old_series, old_config) = (std::mem::take(&mut self.series), std::mem::take(&mut self.config));
        match self.write(&cfg, &series, &keys, &bodies, &old_series, &old_config) {
            Ok((writes, next_series, next_config)) => {
                self.series = next_series;
                self.config = next_config;
                Ok(writes)
            }
            Err(e) => {
                // The transaction rolled back, so disk still matches what it was before.
                self.series = old_series;
                self.config = old_config;
                Err(e)
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn write(
        &mut self,
        cfg: &Map<String, Value>,
        series: &[Series],
        keys: &[String],
        bodies: &[String],
        old_series: &HashMap<String, Saved>,
        old_config: &HashMap<String, u64>,
    ) -> Res<(usize, HashMap<String, Saved>, HashMap<String, u64>)> {
        let conn = self.open()?;
        let tx = conn.transaction()?;
        let mut writes = 0;
        let mut next_config = HashMap::new();
        {
            let mut put = tx.prepare_cached("INSERT INTO config(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value")?;
            for (k, v) in cfg {
                let text = v.to_string();
                let h = hash(&text);
                if old_config.get(k) != Some(&h) {
                    put.execute(params![k, text])?;
                    writes += 1;
                }
                next_config.insert(k.clone(), h);
            }
            for k in old_config.keys().filter(|k| !cfg.contains_key(*k)) {
                tx.execute("DELETE FROM config WHERE key = ?1", params![k])?;
                writes += 1;
            }
        }
        let mut next_series = HashMap::with_capacity(series.len());
        {
            let mut put = tx.prepare_cached("INSERT INTO series(key, pos, body) VALUES(?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET pos = excluded.pos, body = excluded.body")?;
            let mut mv = tx.prepare_cached("UPDATE series SET pos = ?2 WHERE key = ?1")?;
            let mut chapters = tx.prepare_cached("INSERT INTO chapters(key, list) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET list = excluded.list")?;
            for (pos, ((s, key), body)) in series.iter().zip(keys).zip(bodies).enumerate() {
                let h = hash(body);
                let old = old_series.get(key);
                match old {
                    Some(o) if o.body == h => {
                        if o.pos != pos {
                            mv.execute(params![key, pos as i64])?;
                            writes += 1;
                        }
                    }
                    _ => {
                        put.execute(params![key, pos as i64, body])?;
                        writes += 1;
                    }
                }
                let changed = match old {
                    Some(o) => !Arc::ptr_eq(&o.chapters, &s.chapters),
                    None => !s.chapters.is_empty(),
                };
                if changed {
                    chapters.execute(params![key, serde_json::to_string(&*s.chapters)?])?;
                    writes += 1;
                }
                next_series.insert(key.clone(), Saved { pos, body: h, chapters: s.chapters.clone() });
            }
        }
        let gone: HashSet<&String> = old_series.keys().filter(|k| !next_series.contains_key(*k)).collect();
        for k in gone {
            tx.execute("DELETE FROM series WHERE key = ?1", params![k])?;
            writes += 1;
        }
        tx.commit()?;
        Ok((writes, next_series, next_config))
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Progress, Series};
    use super::*;
    use serde_json::json;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nano-db-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn series(url: &str, chapters: usize) -> Series {
        let mut s = Series::new("1".into(), "S".into(), json!({ "url": url, "title": url }));
        s.chapters = Arc::new((0..chapters).rev().map(|i| json!({ "url": format!("{url}/c{i}"), "name": format!("Ch.{i}") })).collect());
        s
    }

    fn data() -> Data {
        let mut d = Data::default();
        let mut a = series("/a", 5);
        a.in_library = true;
        a.progress = Some(Progress { chapter_url: "/a/c1".into(), chapter_name: "Ch.1".into(), page: 2, pages: 9, at: 100 });
        a.marks.insert("/a/c0".into(), true);
        d.series = vec![a, series("/b", 3), series("", 0), series("", 2)];
        d.zoom = Some(1.5);
        d
    }

    fn json_of(d: &Data) -> Value {
        serde_json::to_value(d).unwrap()
    }

    #[test]
    fn everything_survives_a_round_trip_including_repeated_keys() {
        let path = dir("round").join("library.db");
        let d = data();
        Db::new(path.clone()).save(d.clone()).unwrap();
        let back = Db::new(path).load().unwrap().unwrap();
        assert_eq!(json_of(&back), json_of(&d));
        assert_eq!(back.series.len(), 4);
        assert_eq!(back.series[3].chapters.len(), 2);
    }

    #[test]
    fn a_save_writes_only_what_changed() {
        let path = dir("incremental").join("library.db");
        let mut db = Db::new(path.clone());
        let d = data();
        assert!(db.save(d.clone()).unwrap() > 0);
        assert_eq!(db.save(d.clone()).unwrap(), 0, "nothing changed");

        // A reload must not look changed either.
        let mut db = Db::new(path);
        let mut d = db.load().unwrap().unwrap();
        assert_eq!(db.save(d.clone()).unwrap(), 0, "a loaded library saves nothing");

        // Reading on rewrites one small row and leaves the chapter list alone.
        d.series[0].progress.as_mut().unwrap().page = 3;
        assert_eq!(db.save(d.clone()).unwrap(), 1);

        // A new chapter list writes that list, and the row only if the series itself changed.
        d.series[1].chapters = Arc::new(vec![json!({ "url": "/b/c9" })]);
        assert_eq!(db.save(d.clone()).unwrap(), 1);

        // A setting is one row too.
        d.zoom = Some(2.0);
        assert_eq!(db.save(d.clone()).unwrap(), 1);

        // Removing a series removes its row and its chapters.
        d.series.remove(1);
        assert_eq!(db.save(d.clone()).unwrap(), 1 + 2, "the row, then the two series after it move up");
        let back = Db::new(db.path.clone()).load().unwrap().unwrap();
        assert_eq!(json_of(&back), json_of(&d));
    }

    #[test]
    fn moving_a_series_only_updates_its_position() {
        let path = dir("order").join("library.db");
        let mut db = Db::new(path.clone());
        let mut d = data();
        db.save(d.clone()).unwrap();
        d.series.swap(0, 1);
        assert_eq!(db.save(d.clone()).unwrap(), 2);
        let back = Db::new(path).load().unwrap().unwrap();
        assert_eq!(back.series[0].url(), "/b");
        assert_eq!(back.series[1].url(), "/a");
    }

    #[test]
    fn a_backup_reads_back_the_same_and_replaces_the_last_one() {
        let d = dir("backup");
        let (path, copy) = (d.join("library.db"), d.join("library.db.bak"));
        let mut db = Db::new(path.clone());
        let mut data = data();
        db.save(data.clone()).unwrap();
        backup(&path, &copy).unwrap();
        data.zoom = Some(3.0);
        db.save(data.clone()).unwrap();
        backup(&path, &copy).unwrap();
        let back = Db::new(copy).load().unwrap().unwrap();
        assert_eq!(json_of(&back), json_of(&data));
        assert!(!d.join("library.db.bak.new").exists());
    }

    #[test]
    fn no_database_means_nothing_to_load() {
        assert!(Db::new(dir("none").join("library.db")).load().unwrap().is_none());
    }
}
