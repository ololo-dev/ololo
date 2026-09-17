//! Parent-agent control channel for autonomous play.
//!
//! An AI agent (e.g. Claude Code) can drive an ololo session that itself
//! hosts an inner agent in a PTY: the parent drops message files into an
//! inbox directory and reads back a plain-text dump of the inner agent's
//! screen. Built for debugging sessions end-to-end without a human at the
//! keyboard.
//!
//! Layout, under `{config_dir}/ololo/control/<join_code>/` (override the
//! base with `OLOLO_CONTROL_DIR`):
//!
//! - `inbox/*.md|*.txt` — the parent writes one message per file; ololo
//!   picks files up in name order (zero-pad to sequence: `001-….md`),
//!   pastes the content into the inner agent's PTY as one bracketed paste
//!   followed by Enter, and moves the file to `sent/`.
//! - `sent/` — processed messages, kept for audit.
//! - `screen.txt` — latest plain-text render of the inner agent's screen,
//!   refreshed about once a second while it changes.
//!
//! The control dir deliberately lives OUTSIDE the session worktree:
//! snapshots commit the worktree (including `.ololo/`), and control
//! traffic must never leak into judged snapshots or golf size counts.
//!
//! Same settle discipline as the done-flag watcher: a file is read only
//! once its size is unchanged across two polls, so half-written messages
//! are never sent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tokio::sync::{mpsc, oneshot};

/// Poll cadence for the inbox. One tick of latency buys settle detection.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// Resolve the control directory for a session and create its skeleton.
pub fn control_dir(join_code: &str) -> Option<PathBuf> {
    let base = match std::env::var_os("OLOLO_CONTROL_DIR") {
        Some(d) => PathBuf::from(d),
        None => dirs::config_dir()?.join("ololo").join("control"),
    };
    let dir = base.join(join_code);
    for sub in ["inbox", "sent"] {
        if let Err(e) = std::fs::create_dir_all(dir.join(sub)) {
            tracing::warn!("control dir setup failed at {}: {e}", dir.display());
            return None;
        }
    }
    Some(dir)
}

/// Everything the run loops need to serve the channel: queued parent
/// messages, the screen-dump sink, and the requests of the local HTTP
/// service (see [`serve_http`]).
pub struct ControlChannel {
    pub rx: mpsc::UnboundedReceiver<String>,
    pub screen: ScreenSink,
    /// Requests from the HTTP service, answered by the run loop, which is
    /// the only place that can read the app state and write to the PTY.
    /// Taken by the run loop at start (it polls the receiver in its own
    /// `select!` arm, next to the inbox receiver).
    pub req_rx: Option<mpsc::UnboundedReceiver<ControlRequest>>,
    _watcher: tokio::task::JoinHandle<()>,
    _service: tokio::task::JoinHandle<()>,
    dir: PathBuf,
}

impl ControlChannel {
    /// Spawn the inbox watcher and the HTTP service for `dir` and build the
    /// channel around them.
    pub fn spawn(dir: PathBuf) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let screen = ScreenSink::new(dir.join("screen.txt"));
        let watcher = tokio::spawn(watch_inbox(dir.clone(), tx));
        let service = tokio::spawn(serve_http(dir.clone(), req_tx));
        Self {
            rx,
            screen,
            req_rx: Some(req_rx),
            _watcher: watcher,
            _service: service,
            dir,
        }
    }
}

impl Drop for ControlChannel {
    fn drop(&mut self) {
        self._watcher.abort();
        self._service.abort();
        // A stale port file would send the next `ololo session` call to a
        // socket nobody listens on; remove it so the session reads as gone.
        let _ = std::fs::remove_file(self.dir.join(HTTP_PORT_FILE));
        let _ = std::fs::remove_file(self.dir.join(HTTP_TOKEN_FILE));
    }
}

// ---------------------------------------------------------------------------
// The session service: HTTP on the loopback, for agents and scripts
// ---------------------------------------------------------------------------
//
// `ololo session …` (and anything that can speak HTTP) reads the session's
// state and drives the hosted agent through it. The port is ephemeral and
// written to `http.port` in the control dir; every request carries the
// bearer token from `http.token` (loopback is shared with every local
// process, the token file is owner-readable only).
//
//   GET  /session                 the session as the TUI sees it (JSON)
//   GET  /screen                  the hosted agent's screen (text/plain)
//   POST /agent/message  {"text"} paste into the agent and submit
//   POST /agent/keys     {"keys": ["Enter", "Esc", "y", …]} raw keystrokes
//   POST /agent/permission {"allow": true|false} | {"choice": 2}
//                                 answer the agent's own permission dialog
//   POST /permission     {"decision": "allow"|"always"|"session"|"deny"}
//                                 answer ololo's probe-permission popup

/// File in the control dir holding the service's port.
pub const HTTP_PORT_FILE: &str = "http.port";
/// File in the control dir holding the service's bearer token.
pub const HTTP_TOKEN_FILE: &str = "http.token";

/// A request the service needs the run loop to answer.
pub enum ControlRequest {
    /// The session state as JSON.
    Status(oneshot::Sender<serde_json::Value>),
    /// The hosted agent's screen as plain text.
    Screen(oneshot::Sender<String>),
    /// Paste `text` into the agent and submit it.
    Message(String, oneshot::Sender<Result<(), String>>),
    /// Raw bytes into the agent's PTY.
    Keys(Vec<u8>, oneshot::Sender<Result<(), String>>),
    /// Answer ololo's own probe-permission popup. Replies `false` when no
    /// prompt was open.
    ProbePermission(crate::permissions::Decision, oneshot::Sender<bool>),
}

/// Named keys the `keys` endpoint accepts, and their terminal bytes. Anything
/// else is typed literally.
pub fn key_bytes(name: &str) -> Vec<u8> {
    match name {
        "Enter" | "enter" | "CR" => b"\r".to_vec(),
        "Esc" | "esc" | "Escape" => b"\x1b".to_vec(),
        "Tab" | "tab" => b"\t".to_vec(),
        "Space" | "space" => b" ".to_vec(),
        "BSpace" | "Backspace" => b"\x7f".to_vec(),
        "Up" | "up" => b"\x1b[A".to_vec(),
        "Down" | "down" => b"\x1b[B".to_vec(),
        "Right" | "right" => b"\x1b[C".to_vec(),
        "Left" | "left" => b"\x1b[D".to_vec(),
        "C-c" | "Ctrl-C" => b"\x03".to_vec(),
        "C-d" | "Ctrl-D" => b"\x04".to_vec(),
        other => other.as_bytes().to_vec(),
    }
}

/// Parse a probe-permission decision name.
pub fn parse_decision(name: &str) -> Option<crate::permissions::Decision> {
    use crate::permissions::Decision;
    Some(match name {
        "allow" | "allow_once" | "yes" => Decision::Allow,
        "always" | "allow_always" => Decision::AlwaysAllow,
        "session" | "allow_all_session" => Decision::AllowAllSession,
        "deny" | "decline" | "no" => Decision::Decline,
        _ => return None,
    })
}

/// Where the service of a session listens, read back from its control dir.
pub fn read_service(dir: &Path) -> Option<(u16, String)> {
    let port: u16 = std::fs::read_to_string(dir.join(HTTP_PORT_FILE))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let token = std::fs::read_to_string(dir.join(HTTP_TOKEN_FILE))
        .ok()?
        .trim()
        .to_string();
    Some((port, token))
}

async fn serve_http(dir: PathBuf, req_tx: mpsc::UnboundedSender<ControlRequest>) {
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("control service: bind failed: {e}");
            return;
        }
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let token = uuid::Uuid::new_v4().simple().to_string();
    if let Err(e) = write_private(&dir.join(HTTP_TOKEN_FILE), &token)
        .and_then(|_| write_private(&dir.join(HTTP_PORT_FILE), &port.to_string()))
    {
        tracing::warn!("control service: could not publish port/token: {e}");
        return;
    }
    tracing::info!(port, "control service listening");
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let token = token.clone();
        let tx = req_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, &token, &tx).await {
                tracing::debug!("control service: connection: {e}");
            }
        });
    }
}

fn write_private(path: &Path, content: &str) -> std::io::Result<()> {
    std::fs::write(path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> anyhow::Result<HttpRequest> {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 2048];
    let header_end = loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            anyhow::bail!("connection closed before headers");
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > 64 * 1024 {
            anyhow::bail!("headers too large");
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if length > 1024 * 1024 {
        anyhow::bail!("body too large");
    }
    let mut body = buf[header_end..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

async fn handle_conn(
    mut stream: tokio::net::TcpStream,
    token: &str,
    tx: &mpsc::UnboundedSender<ControlRequest>,
) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let req = read_request(&mut stream).await?;
    let (status, content_type, body) = route(&req, token, tx).await;
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

fn json_body(value: serde_json::Value) -> (&'static str, &'static str, String) {
    ("200 OK", "application/json", value.to_string())
}

fn error_body(status: &'static str, message: &str) -> (&'static str, &'static str, String) {
    (
        status,
        "application/json",
        serde_json::json!({ "error": message }).to_string(),
    )
}

async fn route(
    req: &HttpRequest,
    token: &str,
    tx: &mpsc::UnboundedSender<ControlRequest>,
) -> (&'static str, &'static str, String) {
    let (path, query) = req.path.split_once('?').unwrap_or((req.path.as_str(), ""));
    let bearer = req
        .headers
        .get("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    let query_token = query.split('&').find_map(|kv| kv.strip_prefix("token="));
    if bearer != Some(token) && query_token != Some(token) {
        return error_body(
            "401 Unauthorized",
            "missing or wrong token (see http.token)",
        );
    }
    let json: serde_json::Value = if req.body.is_empty() {
        serde_json::Value::Null
    } else {
        match serde_json::from_slice(&req.body) {
            Ok(v) => v,
            Err(e) => return error_body("400 Bad Request", &format!("body is not JSON: {e}")),
        }
    };
    let ask = |request: ControlRequest| tx.send(request).is_ok();
    match (req.method.as_str(), path) {
        ("GET", "/session") => {
            let (reply, rx) = oneshot::channel();
            if !ask(ControlRequest::Status(reply)) {
                return error_body("503 Service Unavailable", "session loop gone");
            }
            match rx.await {
                Ok(v) => json_body(v),
                Err(_) => error_body("503 Service Unavailable", "session loop gone"),
            }
        }
        ("GET", "/screen") => {
            let (reply, rx) = oneshot::channel();
            if !ask(ControlRequest::Screen(reply)) {
                return error_body("503 Service Unavailable", "session loop gone");
            }
            match rx.await {
                Ok(text) => ("200 OK", "text/plain; charset=utf-8", text),
                Err(_) => error_body("503 Service Unavailable", "session loop gone"),
            }
        }
        ("POST", "/agent/message") => {
            let Some(text) = json.get("text").and_then(|v| v.as_str()) else {
                return error_body("400 Bad Request", "expected {\"text\": \"…\"}");
            };
            let (reply, rx) = oneshot::channel();
            if !ask(ControlRequest::Message(text.to_string(), reply)) {
                return error_body("503 Service Unavailable", "session loop gone");
            }
            match rx.await {
                Ok(Ok(())) => json_body(serde_json::json!({ "sent": true })),
                Ok(Err(e)) => error_body("409 Conflict", &e),
                Err(_) => error_body("503 Service Unavailable", "session loop gone"),
            }
        }
        ("POST", "/agent/keys") => {
            let mut bytes = Vec::new();
            if let Some(keys) = json.get("keys").and_then(|v| v.as_array()) {
                for k in keys {
                    if let Some(name) = k.as_str() {
                        bytes.extend(key_bytes(name));
                    }
                }
            }
            if let Some(raw) = json.get("raw").and_then(|v| v.as_str()) {
                bytes.extend_from_slice(raw.as_bytes());
            }
            if bytes.is_empty() {
                return error_body(
                    "400 Bad Request",
                    "expected {\"keys\": [\"Enter\", …]} or {\"raw\": \"…\"}",
                );
            }
            send_keys(tx, bytes).await
        }
        ("POST", "/agent/permission") => {
            // The hosted agent's own dialog (e.g. claude's "Do you want to
            // proceed?"): a digit picks an option, Enter takes the highlighted
            // one, Esc backs out. `allow` = Enter, `deny` = Esc.
            let bytes = if let Some(choice) = json.get("choice").and_then(|v| v.as_u64()) {
                let mut b = choice.to_string().into_bytes();
                b.extend_from_slice(b"\r");
                b
            } else {
                match json.get("allow").and_then(|v| v.as_bool()) {
                    Some(true) => b"\r".to_vec(),
                    Some(false) => b"\x1b".to_vec(),
                    None => {
                        return error_body(
                            "400 Bad Request",
                            "expected {\"allow\": true|false} or {\"choice\": N}",
                        );
                    }
                }
            };
            send_keys(tx, bytes).await
        }
        ("POST", "/permission") => {
            let Some(decision) = json
                .get("decision")
                .and_then(|v| v.as_str())
                .and_then(parse_decision)
            else {
                return error_body(
                    "400 Bad Request",
                    "expected {\"decision\": \"allow\"|\"always\"|\"session\"|\"deny\"}",
                );
            };
            let (reply, rx) = oneshot::channel();
            if !ask(ControlRequest::ProbePermission(decision, reply)) {
                return error_body("503 Service Unavailable", "session loop gone");
            }
            match rx.await {
                Ok(true) => json_body(serde_json::json!({ "answered": true })),
                Ok(false) => error_body("409 Conflict", "no probe permission prompt is open"),
                Err(_) => error_body("503 Service Unavailable", "session loop gone"),
            }
        }
        _ => error_body("404 Not Found", "unknown route"),
    }
}

async fn send_keys(
    tx: &mpsc::UnboundedSender<ControlRequest>,
    bytes: Vec<u8>,
) -> (&'static str, &'static str, String) {
    let (reply, rx) = oneshot::channel();
    if tx.send(ControlRequest::Keys(bytes, reply)).is_err() {
        return error_body("503 Service Unavailable", "session loop gone");
    }
    match rx.await {
        Ok(Ok(())) => json_body(serde_json::json!({ "sent": true })),
        Ok(Err(e)) => error_body("409 Conflict", &e),
        Err(_) => error_body("503 Service Unavailable", "session loop gone"),
    }
}

/// Per-file memory across polls: size at last sight.
#[derive(Default)]
pub struct InboxState {
    seen: HashMap<PathBuf, u64>,
}

/// One poll over `inbox/`: returns files whose size settled since the last
/// poll, in name order. Settled files are forgotten from `state` — the
/// caller moves them out of the inbox, so they will not reappear.
pub fn settled_messages(state: &mut InboxState, inbox: &Path) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(inbox) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut ready = Vec::new();
    let mut present = HashMap::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !(name.ends_with(".md") || name.ends_with(".txt")) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let path = entry.path();
        let size = meta.len();
        if state.seen.get(&path) == Some(&size) {
            ready.push(path.clone());
        }
        present.insert(path, size);
    }
    for path in &ready {
        present.remove(path);
    }
    state.seen = present;
    ready.sort();
    ready
}

/// Read a settled message and archive it to `sent/`. Empty (whitespace-only)
/// files are archived but produce no message.
fn consume(path: &Path, sent_dir: &Path) -> Option<String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(
                "control message unreadable, skipping {}: {e}",
                path.display()
            );
            return None;
        }
    };
    let archived = sent_dir.join(path.file_name().unwrap_or_default());
    if std::fs::rename(path, &archived).is_err() {
        // Cross-device or permission trouble — at least stop re-sending it.
        let _ = std::fs::remove_file(path);
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

async fn watch_inbox(dir: PathBuf, tx: mpsc::UnboundedSender<String>) {
    let inbox = dir.join("inbox");
    let sent = dir.join("sent");
    let mut state = InboxState::default();
    let mut tick = tokio::time::interval(POLL_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        for path in settled_messages(&mut state, &inbox) {
            if let Some(text) = consume(&path, &sent)
                && tx.send(text).is_err()
            {
                return; // run loop gone — session over
            }
        }
    }
}

/// Writes the inner agent's screen as plain text, only when it changed.
pub struct ScreenSink {
    path: PathBuf,
    last_hash: u64,
}

impl ScreenSink {
    pub fn new(path: PathBuf) -> Self {
        Self { path, last_hash: 0 }
    }

    /// Dump `screen` (vt100 contents) if it differs from the last dump.
    pub fn maybe_write(&mut self, screen: &vt100::Screen) {
        use std::hash::{Hash, Hasher};
        let text = screen.contents();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();
        if hash == self.last_hash {
            return;
        }
        self.last_hash = hash;
        if let Err(e) = std::fs::write(&self.path, &text) {
            tracing::warn!("screen dump failed at {}: {e}", self.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn message_settles_after_two_polls_and_fires_once() {
        let tmp = tempfile::tempdir().unwrap();
        let inbox = tmp.path().join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let mut state = InboxState::default();

        write(&inbox, "001-hello.md", "run the tests");
        // First sight: recorded, not ready.
        assert!(settled_messages(&mut state, &inbox).is_empty());
        // Second sight, same size: ready.
        let ready = settled_messages(&mut state, &inbox);
        assert_eq!(ready.len(), 1);
        // Settled files are forgotten; if the caller leaves the file in
        // place it re-enters the settle cycle rather than double-firing.
        assert!(settled_messages(&mut state, &inbox).is_empty());
    }

    #[test]
    fn growing_file_is_not_ready_and_ordering_is_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        let inbox = tmp.path().join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let mut state = InboxState::default();

        write(&inbox, "002-b.md", "second");
        write(&inbox, "001-a.md", "first");
        assert!(settled_messages(&mut state, &inbox).is_empty());
        // 002 grows between polls — only after it settles again is it sent.
        write(&inbox, "002-b.md", "second, but longer now");
        let ready = settled_messages(&mut state, &inbox);
        assert_eq!(ready.len(), 1);
        assert!(ready[0].ends_with("001-a.md"));
        let ready = settled_messages(&mut state, &inbox);
        assert_eq!(ready.len(), 1);
        assert!(ready[0].ends_with("002-b.md"));
    }

    #[test]
    fn non_message_files_are_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let inbox = tmp.path().join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let mut state = InboxState::default();
        write(&inbox, ".DS_Store", "junk");
        write(&inbox, "notes.json", "{}");
        assert!(settled_messages(&mut state, &inbox).is_empty());
        assert!(settled_messages(&mut state, &inbox).is_empty());
    }

    #[test]
    fn consume_archives_and_trims() {
        let tmp = tempfile::tempdir().unwrap();
        let inbox = tmp.path().join("inbox");
        let sent = tmp.path().join("sent");
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::create_dir_all(&sent).unwrap();
        let p = write(&inbox, "001.md", "  do the thing\n");
        assert_eq!(consume(&p, &sent), Some("do the thing".to_string()));
        assert!(!p.exists());
        assert!(sent.join("001.md").exists());
        // Whitespace-only → archived, no message.
        let p = write(&inbox, "002.md", "\n  \n");
        assert_eq!(consume(&p, &sent), None);
        assert!(sent.join("002.md").exists());
    }

    #[test]
    fn named_keys_map_to_terminal_bytes_and_text_types_literally() {
        assert_eq!(key_bytes("Enter"), b"\r".to_vec());
        assert_eq!(key_bytes("Esc"), b"\x1b".to_vec());
        assert_eq!(key_bytes("Up"), b"\x1b[A".to_vec());
        assert_eq!(key_bytes("C-c"), b"\x03".to_vec());
        assert_eq!(key_bytes("y"), b"y".to_vec());
        assert_eq!(key_bytes("/help"), b"/help".to_vec());
    }

    #[test]
    fn decisions_parse_by_their_everyday_names() {
        use crate::permissions::Decision;
        assert_eq!(parse_decision("allow"), Some(Decision::Allow));
        assert_eq!(parse_decision("always"), Some(Decision::AlwaysAllow));
        assert_eq!(parse_decision("session"), Some(Decision::AllowAllSession));
        assert_eq!(parse_decision("deny"), Some(Decision::Decline));
        assert_eq!(parse_decision("maybe"), None);
    }

    #[tokio::test]
    async fn the_service_answers_status_screen_and_keys_over_http() {
        // A stand-in run loop: answers the requests the way the TUI would.
        let tmp = tempfile::tempdir().unwrap();
        let (req_tx, mut req_rx) = mpsc::unbounded_channel();
        let dir = tmp.path().to_path_buf();
        let _service = tokio::spawn(serve_http(dir.clone(), req_tx));
        let (port, token) = loop {
            if let Some(found) = read_service(&dir) {
                break found;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        let typed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = typed.clone();
        let _loop = tokio::spawn(async move {
            while let Some(req) = req_rx.recv().await {
                match req {
                    ControlRequest::Status(reply) => {
                        let _ = reply.send(serde_json::json!({ "join_code": "ABC123" }));
                    }
                    ControlRequest::Screen(reply) => {
                        let _ = reply.send("> hello".to_string());
                    }
                    ControlRequest::Keys(bytes, reply) => {
                        seen.lock().unwrap().extend(bytes);
                        let _ = reply.send(Ok(()));
                    }
                    ControlRequest::Message(text, reply) => {
                        seen.lock().unwrap().extend(text.into_bytes());
                        let _ = reply.send(Ok(()));
                    }
                    ControlRequest::ProbePermission(_, reply) => {
                        let _ = reply.send(false);
                    }
                }
            }
        });
        let client = reqwest::Client::new();
        let base = format!("http://127.0.0.1:{port}");
        // No token: refused.
        let r = client.get(format!("{base}/session")).send().await.unwrap();
        assert_eq!(r.status().as_u16(), 401);
        let r = client
            .get(format!("{base}/session"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 200);
        let v: serde_json::Value = r.json().await.unwrap();
        assert_eq!(v["join_code"], "ABC123");
        let r = client
            .get(format!("{base}/screen?token={token}"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.text().await.unwrap(), "> hello");
        let r = client
            .post(format!("{base}/agent/keys"))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "keys": ["y", "Enter"] }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 200);
        let r = client
            .post(format!("{base}/permission"))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "decision": "allow" }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 409, "no prompt open");
        assert_eq!(typed.lock().unwrap().as_slice(), b"y\r");
    }

    #[test]
    fn screen_sink_writes_only_on_change() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("screen.txt");
        let mut parser = vt100::Parser::new(5, 20, 0);
        parser.process(b"hello");
        let mut sink = ScreenSink::new(path.clone());
        sink.maybe_write(parser.screen());
        let first = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("hello"));
        // Unchanged screen → no rewrite.
        std::fs::write(&path, "tampered").unwrap();
        sink.maybe_write(parser.screen());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "tampered");
        let _ = first;
        // Changed screen → rewritten.
        parser.process(b" world");
        sink.maybe_write(parser.screen());
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("hello world")
        );
    }
}
