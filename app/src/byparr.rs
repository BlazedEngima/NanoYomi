//! Byparr runs Cloudflare checks in its own browser, in Docker. Nano tries it before the in-app window.

use serde_json::{Value, json};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Set NANO_BYPARR to use a different address.
pub fn base_url() -> String {
    std::env::var("NANO_BYPARR").unwrap_or_else(|_| "http://localhost:8191".into()).trim_end_matches('/').to_string()
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

const IMAGE: &str = "ghcr.io/thephaseless/byparr:latest";
const CONTAINER: &str = "nano-byparr";

#[derive(Clone, PartialEq)]
pub enum Status {
    Checking,
    NoDocker,
    Missing,
    Downloading,
    Stopped,
    Starting,
    Running,
    Failed(String),
}

pub struct Service {
    pub status: Status,
    /// Nano started it, so Nano stops it when it quits.
    started: bool,
    /// Set once the image is downloaded, since Docker can't be asked while it's stopped.
    marker: std::path::PathBuf,
}

impl Service {
    pub fn new(data_dir: &std::path::Path, autostart: bool, cx: &mut gpui::Context<Self>) -> Self {
        let marker = data_dir.join("byparr-downloaded");
        cx.on_app_quit(|this, _| {
            let started = this.started;
            async move {
                if started {
                    let _ = docker(&["stop", "-t", "2", CONTAINER]);
                }
            }
        })
        .detach();
        let m = marker.clone();
        cx.spawn(async move |this, cx| {
            let status = cx.background_executor().spawn(async move { check(&m) }).await;
            let start = autostart && status == Status::Stopped;
            let _ = this.update(cx, |s, cx| {
                s.status = status;
                if start {
                    s.run(cx);
                }
                cx.notify();
            });
        })
        .detach();
        Service { status: Status::Checking, started: false, marker }
    }

    pub fn download(&mut self, cx: &mut gpui::Context<Self>) {
        let marker = self.marker.clone();
        self.task(
            Status::Downloading,
            move || {
                docker_up()?;
                docker(&["pull", IMAGE])?;
                std::fs::write(&marker, IMAGE).map_err(|e| e.to_string())?;
                Ok(Status::Stopped)
            },
            cx,
        );
    }

    pub fn run(&mut self, cx: &mut gpui::Context<Self>) {
        self.started = true;
        self.task(Status::Starting, start, cx);
    }

    pub fn stop(&mut self, cx: &mut gpui::Context<Self>) {
        self.started = false;
        self.task(Status::Starting, || docker(&["stop", "-t", "2", CONTAINER]).map(|_| Status::Stopped), cx);
    }

    fn task(&mut self, during: Status, work: impl FnOnce() -> Result<Status, String> + Send + 'static, cx: &mut gpui::Context<Self>) {
        self.status = during;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let status = cx.background_executor().spawn(async move { work().unwrap_or_else(Status::Failed) }).await;
            let _ = this.update(cx, |s, cx| {
                s.status = status;
                cx.notify();
            });
        })
        .detach();
    }
}

/// Apps opened from Finder don't get Docker on their PATH.
fn docker_bin() -> Option<&'static str> {
    ["/usr/local/bin/docker", "/opt/homebrew/bin/docker", "/Applications/Docker.app/Contents/Resources/bin/docker"]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
}

fn docker(args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new(docker_bin().ok_or("Docker isn't installed")?)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().last().unwrap_or("Docker failed").trim().to_string())
    }
}

fn docker_up() -> Result<(), String> {
    if docker(&["info"]).is_ok() {
        return Ok(());
    }
    let _ = std::process::Command::new("open").args(["-g", "-a", "Docker"]).status();
    for _ in 0..60 {
        std::thread::sleep(Duration::from_secs(2));
        if docker(&["info"]).is_ok() {
            return Ok(());
        }
    }
    Err("Docker didn't start".into())
}

fn check(marker: &std::path::Path) -> Status {
    if docker_bin().is_none() {
        return Status::NoDocker;
    }
    if running(&base_url()) {
        return Status::Running;
    }
    // Ask Docker when it's running in case the image was removed. Otherwise trust the marker.
    let have = if docker(&["info"]).is_ok() { docker(&["image", "inspect", IMAGE]).is_ok() } else { marker.exists() };
    if have { Status::Stopped } else { Status::Missing }
}

fn start() -> Result<Status, String> {
    docker_up()?;
    let _ = docker(&["rm", "-f", CONTAINER]);
    docker(&["run", "-d", "--rm", "--name", CONTAINER, "-p", "127.0.0.1:8191:8191", IMAGE])?;
    // Wait for /health. The port opens before Byparr is actually ready.
    let base = "http://127.0.0.1:8191";
    for _ in 0..90 {
        std::thread::sleep(Duration::from_secs(1));
        if ureq::get(&format!("{base}/docs")).call().is_ok() {
            return Ok(Status::Running);
        }
        if docker(&["inspect", "-f", "{{.State.Running}}", CONTAINER]).as_deref() != Ok("true") {
            return Err("Byparr stopped while starting".into());
        }
    }
    Err("Byparr didn't start in time".into())
}

#[derive(Debug)]
pub struct Solved {
    pub cookies: Vec<String>,
    pub user_agent: String,
}

/// Asks Byparr to open `url` and pass the check. Blocks for up to about 70 s.
pub fn solve(base: &str, url: &str) -> Result<Solved, String> {
    let body = json!({ "cmd": "request.get", "url": url, "maxTimeout": 60000, "returnOnlyCookies": true });
    let config = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(120))).http_status_as_error(false).build();
    let mut res = ureq::Agent::new_with_config(config).post(&format!("{base}/v1"))
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .map_err(|e| format!("Byparr: {e}"))?;
    let status = res.status().as_u16();
    let text = res.body_mut().read_to_string().map_err(|e| format!("Byparr: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("Byparr: {e}"))?;
    parse(status, &v)
}

fn parse(status: u16, v: &Value) -> Result<Solved, String> {
    if status != 200 || v["status"] != "ok" {
        return Err(format!("Byparr: {}", v["message"].as_str().or(v["detail"].as_str()).unwrap_or("failed")));
    }
    let s = &v["solution"];
    let user_agent = s["userAgent"].as_str().filter(|u| !u.is_empty()).ok_or("Byparr: no user agent")?.to_string();
    let now = crate::store::now_ms() / 1000;
    let cookies = s["cookies"].as_array().into_iter().flatten().filter_map(|c| Some(set_cookie(c, now)?)).collect::<Vec<_>>();
    if !cookies.iter().any(|c| c.starts_with("cf_clearance=")) {
        return Err("Byparr: no cf_clearance cookie".into());
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
