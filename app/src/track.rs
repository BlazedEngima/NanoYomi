//! MyAnimeList and AniList tracking, with Mihon's tracker ids, statuses and scores so phones read the entries as their own.

use crate::store::{Track, now_ms};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

pub const MAL: i64 = 1;
pub const ANILIST: i64 = 2;
pub const TRACKERS: [i64; 2] = [MAL, ANILIST];

const MAL_CLIENT: &str = "ec51dd01e909b73bd7873a2e62e742b4";
const ANILIST_CLIENT: &str = "57476";
/// Where both sites send you after sign-in. Registered with each, so don't change it.
const PORT: u16 = 47621;

/// Statuses shared by both, under Mihon's numbers. Planning and rereading differ, see `plan` and `reread`.
pub const READING: i64 = 1;
pub const COMPLETED: i64 = 2;
pub const ON_HOLD: i64 = 3;
pub const DROPPED: i64 = 4;

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(default)]
pub struct Login {
    pub access: String,
    pub refresh: String,
    /// When `access` expires, in epoch ms. 0 means unknown (AniList tokens last a year).
    pub expires: i64,
    pub user: String,
    pub user_id: i64,
}

/// A search result to pick from.
#[derive(Clone, Debug)]
pub struct Found {
    pub id: i64,
    pub title: String,
    pub kind: String,
    pub year: String,
    pub chapters: i64,
    pub url: String,
}

pub fn name(tracker: i64) -> &'static str {
    match tracker {
        MAL => "MyAnimeList",
        ANILIST => "AniList",
        _ => "Tracker",
    }
}

pub fn configured(tracker: i64) -> bool {
    !match tracker {
        MAL => MAL_CLIENT,
        _ => ANILIST_CLIENT,
    }
    .is_empty()
}

fn plan(tracker: i64) -> i64 {
    if tracker == MAL { 6 } else { 5 }
}

fn reread(tracker: i64) -> i64 {
    if tracker == MAL { 7 } else { 6 }
}

pub fn statuses(tracker: i64) -> [(i64, &'static str); 6] {
    [
        (READING, "Reading"),
        (plan(tracker), "Plan to read"),
        (COMPLETED, "Completed"),
        (ON_HOLD, "On hold"),
        (DROPPED, "Dropped"),
        (reread(tracker), "Rereading"),
    ]
}

/// The 0 to 10 score shown, from what's stored. Mihon keeps AniList's on its 100 point scale.
pub fn score_of(t: &Track) -> i64 {
    if t.tracker == ANILIST { (t.score / 10.).round() as i64 } else { t.score.round() as i64 }
}

pub fn set_score(t: &mut Track, ten: i64) {
    t.score = if t.tracker == ANILIST { ten as f32 * 10. } else { ten as f32 };
}

/// Moves a track up to chapter `number` the way Mihon does after reading. Never moves it back.
pub fn advance(t: &mut Track, number: f32, now: i64) -> bool {
    if number <= t.last_read {
        return false;
    }
    t.last_read = number;
    if t.status == 0 || t.status == plan(t.tracker) {
        t.status = READING;
    }
    if t.status == READING && t.started == 0 {
        t.started = now;
    }
    if t.total > 0 && number as i64 >= t.total && t.status != COMPLETED {
        t.status = COMPLETED;
        if t.finished == 0 {
            t.finished = now;
        }
    }
    true
}

// Signing in

fn random_hex(bytes: usize) -> Result<String, String> {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn redirect(tracker: i64) -> String {
    format!("http://localhost:{PORT}/{}", if tracker == MAL { "mal" } else { "anilist" })
}

/// Where to send the browser. Pass the result's state and verifier to `finish_login`.
pub fn login_url(tracker: i64) -> Result<(String, String, String), String> {
    let (state, verifier) = (random_hex(16)?, random_hex(48)?);
    let url = if tracker == MAL {
        // PKCE "plain", the only kind MyAnimeList takes.
        format!(
            "https://myanimelist.net/v1/oauth2/authorize?response_type=code&client_id={MAL_CLIENT}&code_challenge={verifier}&code_challenge_method=plain&state={state}&redirect_uri={}",
            encode(&redirect(MAL))
        )
    } else {
        format!("https://anilist.co/api/v2/oauth/authorize?client_id={ANILIST_CLIENT}&response_type=token")
    };
    Ok((url, state, verifier))
}

/// Blocks until the browser comes back, then swaps what it brought for a login.
pub fn finish_login(tracker: i64, state: &str, verifier: &str) -> Result<Login, String> {
    let params = wait_for_redirect(tracker)?;
    let get = |k: &str| params.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();
    if !get("error").is_empty() {
        return Err(if get("error") == "access_denied" { "Signing in was cancelled.".into() } else { format!("{} said: {}", name(tracker), get("error")) });
    }
    if tracker == MAL {
        if get("state") != state {
            return Err("The sign-in reply didn't match. Try again.".into());
        }
        let v = form("https://myanimelist.net/v1/oauth2/token", &[
            ("client_id", MAL_CLIENT),
            ("grant_type", "authorization_code"),
            ("code", &get("code")),
            ("code_verifier", verifier),
            ("redirect_uri", &redirect(MAL)),
        ])?;
        let mut login = tokens(&v)?;
        let me = mal_get(&login, "https://api.myanimelist.net/v2/users/@me", &[])?;
        login.user = me["name"].as_str().unwrap_or_default().to_string();
        login.user_id = me["id"].as_i64().unwrap_or(0);
        Ok(login)
    } else {
        let mut login = Login { access: get("access_token"), ..Login::default() };
        if login.access.is_empty() {
            return Err("AniList sent no token. Try again.".into());
        }
        let me = graphql(&login, "query { Viewer { id name } }", json!({}))?;
        login.user = me["Viewer"]["name"].as_str().unwrap_or_default().to_string();
        login.user_id = me["Viewer"]["id"].as_i64().unwrap_or(0);
        Ok(login)
    }
}

/// Serves the redirect page for up to five minutes, on both loopback addresses since "localhost" can mean either.
fn wait_for_redirect(tracker: i64) -> Result<Vec<(String, String)>, String> {
    let listeners: Vec<TcpListener> = ["127.0.0.1", "[::1]"].iter().filter_map(|h| TcpListener::bind(format!("{h}:{PORT}")).ok()).collect();
    if listeners.is_empty() {
        return Err(format!("Port {PORT} is busy, so the sign-in can't finish. Close whatever is using it and try again."));
    }
    for l in &listeners {
        l.set_nonblocking(true).map_err(|e| e.to_string())?;
    }
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline {
        for l in &listeners {
            let Ok((stream, _)) = l.accept() else { continue };
            if let Some(params) = answer(stream, tracker) {
                return Ok(params);
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    Err("Signing in timed out.".into())
}

/// Answers one browser request. AniList's token sits after a #, which servers never see, so the first visit bounces it back as a query.
fn answer(mut stream: TcpStream, tracker: i64) -> Option<Vec<(String, String)>> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).ok()?;
    let head = String::from_utf8_lossy(&buf[..n]);
    let target = head.split_whitespace().nth(1).unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let page = |body: &str| format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let done = "<!doctype html><title>Nano</title><body style=\"font:16px system-ui;padding:40px\">Signed in. You can close this tab and go back to Nano.</body>";
    if tracker == ANILIST && path == "/anilist" {
        let hop = "<!doctype html><title>Nano</title><script>location.replace('/anilist/token?' + location.hash.slice(1))</script>";
        let _ = stream.write_all(page(hop).as_bytes());
        return None;
    }
    let wanted = if tracker == MAL { "/mal" } else { "/anilist/token" };
    if path != wanted {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return None;
    }
    let cancelled = "<!doctype html><title>Nano</title><body style=\"font:16px system-ui;padding:40px\">Not signed in. You can close this tab and try again from Nano.</body>";
    let _ = stream.write_all(page(if query.contains("error=") { cancelled } else { done }).as_bytes());
    // Not percent-decoded. The tokens and codes these sites send don't need it.
    Some(query.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect())
}

fn tokens(v: &Value) -> Result<Login, String> {
    let access = v["access_token"].as_str().ok_or("No token in the reply")?.to_string();
    Ok(Login {
        access,
        refresh: v["refresh_token"].as_str().unwrap_or_default().to_string(),
        expires: now_ms() + v["expires_in"].as_i64().unwrap_or(0) * 1000,
        ..Login::default()
    })
}

/// MyAnimeList tokens last a month. Refreshes one that's about to expire, and returns true if `login` changed.
pub fn refresh(tracker: i64, login: &mut Login) -> Result<bool, String> {
    if tracker != MAL || login.expires == 0 || login.expires > now_ms() + 60_000 {
        return Ok(false);
    }
    let v = form("https://myanimelist.net/v1/oauth2/token", &[("client_id", MAL_CLIENT), ("grant_type", "refresh_token"), ("refresh_token", &login.refresh)])
        .map_err(|_| "MyAnimeList signed you out. Sign in again in Settings.".to_string())?;
    let fresh = tokens(&v)?;
    login.access = fresh.access;
    login.refresh = fresh.refresh;
    login.expires = fresh.expires;
    Ok(true)
}

// HTTP

fn read(res: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value, String> {
    let mut res = res.map_err(|e| match e {
        ureq::Error::StatusCode(401) => "Signed out. Sign in again in Settings.".to_string(),
        ureq::Error::StatusCode(429) => "Too many requests. Try again in a minute.".to_string(),
        e => e.to_string(),
    })?;
    let text = res.body_mut().read_to_string().map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build())
}

fn form(url: &str, fields: &[(&str, &str)]) -> Result<Value, String> {
    read(agent().post(url).send_form(fields.iter().copied()))
}

fn mal_get(login: &Login, url: &str, query: &[(&str, &str)]) -> Result<Value, String> {
    read(agent().get(url).header("Authorization", format!("Bearer {}", login.access)).query_pairs(query.iter().copied()).call())
}

fn graphql(login: &Login, query: &str, variables: Value) -> Result<Value, String> {
    let body = json!({ "query": query, "variables": variables }).to_string();
    let v = read(
        agent()
            .post("https://graphql.anilist.co")
            .header("Authorization", format!("Bearer {}", login.access))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .send(body),
    )?;
    if let Some(e) = v["errors"].as_array().and_then(|a| a.first()) {
        return Err(e["message"].as_str().unwrap_or("AniList refused that").to_string());
    }
    Ok(v["data"].clone())
}

// Search, link and update

pub fn search(tracker: i64, login: &Login, query: &str) -> Result<Vec<Found>, String> {
    if tracker == MAL {
        // MyAnimeList refuses queries over 64 characters.
        let q: String = query.chars().take(64).collect();
        let v = mal_get(login, "https://api.myanimelist.net/v2/manga", &[
            ("q", &q),
            ("limit", "30"),
            ("nsfw", "true"),
            ("fields", "id,title,num_chapters,media_type,start_date"),
        ])?;
        Ok(v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| &d["node"])
            .filter(|n| !n["media_type"].as_str().unwrap_or("").contains("novel"))
            .map(|n| {
                let id = n["id"].as_i64().unwrap_or(0);
                Found {
                    id,
                    title: n["title"].as_str().unwrap_or_default().to_string(),
                    kind: n["media_type"].as_str().unwrap_or_default().replace('_', " "),
                    year: n["start_date"].as_str().unwrap_or_default().chars().take(4).collect(),
                    chapters: n["num_chapters"].as_i64().unwrap_or(0),
                    url: format!("https://myanimelist.net/manga/{id}"),
                }
            })
            .collect())
    } else {
        let v = graphql(
            login,
            "query ($q: String) { Page(perPage: 30) { media(search: $q, type: MANGA, format_not_in: [NOVEL]) { id title { userPreferred } format chapters startDate { year } siteUrl } } }",
            json!({ "q": query }),
        )?;
        Ok(v["Page"]["media"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| Found {
                id: m["id"].as_i64().unwrap_or(0),
                title: m["title"]["userPreferred"].as_str().unwrap_or_default().to_string(),
                kind: m["format"].as_str().unwrap_or_default().to_lowercase().replace('_', " "),
                year: m["startDate"]["year"].as_i64().map(|y| y.to_string()).unwrap_or_default(),
                chapters: m["chapters"].as_i64().unwrap_or(0),
                url: m["siteUrl"].as_str().unwrap_or_default().to_string(),
            })
            .collect())
    }
}

/// Links a series to `found`, keeping your entry there or adding one, and catches it up to `read`.
pub fn bind(tracker: i64, login: &Login, found: &Found, read: f32) -> Result<Track, String> {
    let mut t = Track { tracker, media_id: found.id, url: found.url.clone(), title: found.title.clone(), total: found.chapters, ..Track::default() };
    let listed = if tracker == MAL {
        let v = mal_get(login, &format!("https://api.myanimelist.net/v2/manga/{}", found.id), &[("fields", "num_chapters,my_list_status{start_date,finish_date}")])?;
        let s = &v["my_list_status"];
        s.is_object().then(|| {
            t.status = if s["is_rereading"].as_bool() == Some(true) { reread(MAL) } else { mal_status_from(s["status"].as_str().unwrap_or("")) };
            t.last_read = s["num_chapters_read"].as_f64().unwrap_or(0.) as f32;
            t.score = s["score"].as_f64().unwrap_or(0.) as f32;
            t.started = s["start_date"].as_str().map_or(0, ms_of);
            t.finished = s["finish_date"].as_str().map_or(0, ms_of);
        })
    } else {
        let v = graphql(
            login,
            "query ($u: Int, $m: Int) { Page { mediaList(userId: $u, type: MANGA, mediaId: $m) { id status scoreRaw: score(format: POINT_100) progress private startedAt { year month day } completedAt { year month day } } } }",
            json!({ "u": login.user_id, "m": found.id }),
        )?;
        v["Page"]["mediaList"].as_array().and_then(|a| a.first()).map(|e| {
            t.library_id = e["id"].as_i64().unwrap_or(0);
            t.status = anilist_status_from(e["status"].as_str().unwrap_or(""));
            t.last_read = e["progress"].as_f64().unwrap_or(0.) as f32;
            t.score = e["scoreRaw"].as_f64().unwrap_or(0.) as f32;
            t.private = e["private"].as_bool().unwrap_or(false);
            t.started = fuzzy_ms(&e["startedAt"]);
            t.finished = fuzzy_ms(&e["completedAt"]);
        })
    };
    let caught_up = advance(&mut t, read, now_ms());
    if listed.is_none() && t.status == 0 {
        t.status = plan(tracker);
    }
    if listed.is_none() || caught_up {
        push(login, &mut t)?;
    }
    Ok(t)
}

/// Sends a track's status, progress, score and dates.
pub fn push(login: &Login, t: &mut Track) -> Result<(), String> {
    let date = |ms: i64| if ms == 0 { String::new() } else { crate::manga::ymd(ms) };
    if t.tracker == MAL {
        let (started, finished) = (date(t.started), date(t.finished));
        let status = mal_status_to(t.status);
        let rereading = (t.status == reread(MAL)).to_string();
        let (score, chapters) = ((t.score.round() as i64).to_string(), (t.last_read as i64).to_string());
        let mut fields = vec![("status", status), ("is_rereading", rereading.as_str()), ("score", score.as_str()), ("num_chapters_read", chapters.as_str())];
        if !started.is_empty() {
            fields.push(("start_date", &started));
        }
        if !finished.is_empty() {
            fields.push(("finish_date", &finished));
        }
        let url = format!("https://api.myanimelist.net/v2/manga/{}/my_list_status", t.media_id);
        let res = agent().put(&url).header("Authorization", format!("Bearer {}", login.access)).send_form(fields);
        read(res).map(|_| ())
    } else {
        let fuzzy = |ms: i64| {
            if ms == 0 {
                return json!({ "year": null, "month": null, "day": null });
            }
            let d = date(ms);
            let n = |r: std::ops::Range<usize>| d[r].parse::<i64>().ok();
            json!({ "year": n(0..4), "month": n(5..7), "day": n(8..10) })
        };
        let v = graphql(
            login,
            "mutation ($m: Int, $p: Int, $s: MediaListStatus, $private: Boolean, $score: Int, $start: FuzzyDateInput, $end: FuzzyDateInput) { SaveMediaListEntry(mediaId: $m, progress: $p, status: $s, private: $private, scoreRaw: $score, startedAt: $start, completedAt: $end) { id } }",
            json!({
                "m": t.media_id, "p": t.last_read as i64, "s": anilist_status_to(t.status), "private": t.private,
                "score": t.score.round() as i64, "start": fuzzy(t.started), "end": fuzzy(t.finished),
            }),
        )?;
        t.library_id = v["SaveMediaListEntry"]["id"].as_i64().unwrap_or(t.library_id);
        Ok(())
    }
}

fn mal_status_from(s: &str) -> i64 {
    match s {
        "reading" => READING,
        "completed" => COMPLETED,
        "on_hold" => ON_HOLD,
        "dropped" => DROPPED,
        _ => plan(MAL),
    }
}

fn mal_status_to(s: i64) -> &'static str {
    match s {
        COMPLETED => "completed",
        ON_HOLD => "on_hold",
        DROPPED => "dropped",
        6 => "plan_to_read",
        _ => "reading",
    }
}

fn anilist_status_from(s: &str) -> i64 {
    match s {
        "CURRENT" => READING,
        "COMPLETED" => COMPLETED,
        "PAUSED" => ON_HOLD,
        "DROPPED" => DROPPED,
        "REPEATING" => 6,
        _ => 5,
    }
}

fn anilist_status_to(s: i64) -> &'static str {
    match s {
        COMPLETED => "COMPLETED",
        ON_HOLD => "PAUSED",
        DROPPED => "DROPPED",
        5 => "PLANNING",
        6 => "REPEATING",
        _ => "CURRENT",
    }
}

/// Days since 1970 for a civil date (Howard Hinnant's days_from_civil).
fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "2024-05-01", "2024-05" or "2024" to epoch ms.
fn ms_of(s: &str) -> i64 {
    let mut parts = s.split('-').map(|p| p.parse::<i64>().ok());
    match (parts.next().flatten(), parts.next().flatten().unwrap_or(1), parts.next().flatten().unwrap_or(1)) {
        (Some(y), m, d) => days(y, m, d) * 86_400_000,
        _ => 0,
    }
}

fn fuzzy_ms(v: &Value) -> i64 {
    match v["year"].as_i64() {
        Some(y) => days(y, v["month"].as_i64().unwrap_or(1), v["day"].as_i64().unwrap_or(1)) * 86_400_000,
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip() {
        assert_eq!(ms_of("2000-02-29"), 951_782_400_000);
        assert_eq!(crate::manga::ymd(ms_of("2025-11-17")), "2025-11-17");
        assert_eq!(fuzzy_ms(&json!({ "year": 2025, "month": 11, "day": 17 })), ms_of("2025-11-17"));
        assert_eq!(fuzzy_ms(&json!({ "year": null })), 0);
    }

    #[test]
    fn reading_moves_tracks_like_mihon() {
        let mut t = Track { tracker: MAL, status: 6, total: 10, ..Track::default() };
        assert!(advance(&mut t, 3., 100));
        assert_eq!((t.status, t.last_read, t.started), (READING, 3., 100));
        assert!(!advance(&mut t, 2., 200), "never moves back");
        assert!(advance(&mut t, 10., 300));
        assert_eq!((t.status, t.finished), (COMPLETED, 300));
        let mut a = Track { tracker: ANILIST, ..Track::default() };
        set_score(&mut a, 8);
        assert_eq!((a.score, score_of(&a)), (80., 8));
    }
}
