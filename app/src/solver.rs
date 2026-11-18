//! An optional FlareSolverr or Byparr the user runs somewhere. Nano tries it before its own window.

use serde_json::{Value, json};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::Duration;

static URL: Mutex<String> = Mutex::new(String::new());

pub fn set_url(url: &str) {
    *URL.lock().unwrap() = url.trim().trim_end_matches('/').to_string();
}

/// The solver's address, if one is set.
pub fn url() -> Option<String> {
    Some(URL.lock().unwrap().clone()).filter(|u| !u.is_empty())
}

/// Checks that something is listening. Byparr's /health runs a whole solve, so it's too slow for this.
pub fn running(base: &str) -> bool {
    let hostport = base.split("://").nth(1).unwrap_or(base).split('/').next().unwrap_or_default();
    let hostport = if hostport.contains(':') { hostport.to_string() } else { format!("{hostport}:80") };
    hostport
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .is_some_and(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok())
}

#[derive(Debug)]
pub struct Solved {
    pub cookies: Vec<String>,
    pub user_agent: String,
}

/// Asks the solver to open `url` and pass the check. Blocks for up to about 70 s.
pub fn solve(base: &str, url: &str) -> Result<Solved, String> {
    let body = json!({ "cmd": "request.get", "url": url, "maxTimeout": 60000, "returnOnlyCookies": true });
    let config = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(120))).http_status_as_error(false).build();
    let mut res = ureq::Agent::new_with_config(config).post(&format!("{base}/v1"))
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .map_err(|e| format!("Solver: {e}"))?;
    let status = res.status().as_u16();
    let text = res.body_mut().read_to_string().map_err(|e| format!("Solver: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("Solver: {e}"))?;
    parse(status, &v)
}

fn parse(status: u16, v: &Value) -> Result<Solved, String> {
    if status != 200 || v["status"] != "ok" {
        return Err(format!("Solver: {}", v["message"].as_str().or(v["detail"].as_str()).unwrap_or("failed")));
    }
    let s = &v["solution"];
    let user_agent = s["userAgent"].as_str().filter(|u| !u.is_empty()).ok_or("Solver: no user agent")?.to_string();
    let now = crate::store::now_ms() / 1000;
    let cookies = s["cookies"].as_array().into_iter().flatten().filter_map(|c| Some(set_cookie(c, now)?)).collect::<Vec<_>>();
    if !cookies.iter().any(|c| c.starts_with("cf_clearance=")) {
        return Err("Solver: no cf_clearance cookie".into());
    }
    Ok(Solved { cookies, user_agent })
}

fn set_cookie(c: &Value, now: i64) -> Option<String> {
    let mut s = format!("{}={}", c["name"].as_str()?, c["value"].as_str()?);
    if let Some(d) = c["domain"].as_str() {
        s += &format!("; Domain={}", d.trim_start_matches('.'));
    }
    s += &format!("; Path={}", c["path"].as_str().unwrap_or("/"));
    if let Some(exp) = c["expires"].as_f64().filter(|e| *e > 0.) {
        s += &format!("; Max-Age={}", (exp as i64 - now).max(0));
    }
    if c["secure"].as_bool() == Some(true) {
        s += "; Secure";
    }
    if c["httpOnly"].as_bool() == Some(true) {
        s += "; HttpOnly";
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_solution() {
        let v = json!({ "status": "ok", "message": "Success", "solution": {
            "userAgent": "Mozilla/5.0 Firefox/140.0",
            "cookies": [
                { "name": "cf_clearance", "value": "abc", "domain": ".example.com", "path": "/", "expires": 1_000_100.0, "secure": true, "httpOnly": true },
                { "name": "sid", "value": "1", "domain": "example.com", "path": "/", "expires": -1 }
            ]
        }});
        let solved = parse(200, &v).unwrap();
        assert_eq!(solved.user_agent, "Mozilla/5.0 Firefox/140.0");
        assert_eq!(solved.cookies[0], "cf_clearance=abc; Domain=example.com; Path=/; Max-Age=999000; Secure; HttpOnly".replace("999000", &(1_000_100 - crate::store::now_ms() / 1000).max(0).to_string()));
        assert_eq!(solved.cookies[1], "sid=1; Domain=example.com; Path=/");
        assert!(parse(408, &json!({ "detail": "Timed out" })).unwrap_err().contains("Timed out"));
        assert!(parse(200, &json!({ "status": "ok", "solution": { "userAgent": "x", "cookies": [] } })).is_err());
    }
}
