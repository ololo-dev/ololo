//! Small cross-cutting helpers shared across `ololo` modules.

/// Convert an HTTP(S) server base URL to its WebSocket (ws/wss) equivalent.
///
/// `https://` → `wss://`, `http://` → `ws://`. Other schemes are left
/// untouched. The trailing slash is trimmed so callers can append a path
/// with a single `/`.
pub(crate) fn ws_base_url(base: &str) -> String {
    base.trim_end_matches('/')
        .replace("https://", "wss://")
        .replace("http://", "ws://")
}

/// Inverse of [`ws_base_url`]: convert a ws/wss URL back to http(s).
pub(crate) fn http_base_url(base: &str) -> String {
    base.trim_end_matches('/')
        .replace("wss://", "https://")
        .replace("ws://", "http://")
}
