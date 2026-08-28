//! `git http-backend` CGI bridge: serves bare player repos over HTTP for
//! `git push` / `git fetch` from ololo.
//!
//! Route: `GET/POST /git/{session_id}/{player_id}.git/*`
//!
//! Auth: the player's PAT is validated via the standard `auth::pat::lookup_pat`
//! path. The caller must be a member of the session (player row exists and
//! matches the path's `player_id`).
//!
//! Implementation: spawns `git http-backend` as a CGI process with the
//! appropriate env vars (GIT_PROJECT_ROOT, PATH_INFO, QUERY_STRING,
//! GIT_HTTP_EXPORT_ALL), pipes the raw request body to its stdin, and
//! streams its stdout back as the HTTP response. This is the canonical way
//! to serve git smart HTTP from a custom server without linking libgit2.

use crate::auth::pat::lookup_pat;
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::process::Stdio;

/// Path capture: `/{session_id}/{player_id}.git/{*rest}`.
pub async fn git_http_handler(
    State(state): State<AppState>,
    Path((session_id, player_id, rest)): Path<(String, String, String)>,
    req: Request,
) -> Response {
    let method = req.method().clone();
    let headers = req.headers().clone();
    let query = req.uri().query().unwrap_or("").to_string();
    let body = match axum::body::to_bytes(req.into_body(), 256 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("git http: failed to read request body: {e}");
            return (StatusCode::BAD_REQUEST, "body read failed").into_response();
        }
    };

    // 1. Auth: require a valid PAT in Authorization: Bearer or X-API-Key.
    let pat = extract_pat(&headers);
    let Some(pat) = pat else {
        // git smart HTTP first probes without credentials; we must respond
        // with a WWW-Authenticate challenge so git retries with Basic auth.
        return (
            StatusCode::UNAUTHORIZED,
            [(
                HeaderName::from_static("www-authenticate"),
                HeaderValue::from_static("Basic realm=\"ololo\""),
            )],
            "missing credentials",
        )
            .into_response();
    };
    let claims = match lookup_pat(&state.db, &pat).await {
        Ok(c) => c,
        Err(_) => return (StatusCode::UNAUTHORIZED, "invalid credentials").into_response(),
    };
    let user_id: uuid::Uuid = match claims.sub.parse() {
        Ok(u) => u,
        Err(_) => return (StatusCode::UNAUTHORIZED, "invalid user").into_response(),
    };

    // 2. Validate session_id + player_id format.
    let session_uuid: uuid::Uuid = match session_id.parse() {
        Ok(u) => u,
        Err(_) => return (StatusCode::NOT_FOUND, "no such repo").into_response(),
    };
    let player_uuid: uuid::Uuid = match player_id.strip_suffix(".git").unwrap_or(&player_id).parse()
    {
        Ok(u) => u,
        Err(_) => return (StatusCode::NOT_FOUND, "no such repo").into_response(),
    };

    // 3. Authorization: the player row must exist, belong to this session,
    //    be owned by the authenticated user, and not be revoked.
    use arena_core::entities::players;
    let player = players::Entity::find()
        .filter(players::Column::Id.eq(player_uuid))
        .filter(players::Column::SessionIdFk.eq(session_uuid))
        .filter(players::Column::RevokedAt.is_null())
        .one(&state.db)
        .await;
    let player = match player {
        Ok(Some(p)) => p,
        Ok(None) => return (StatusCode::NOT_FOUND, "no such repo").into_response(),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response(),
    };
    if player.user_id_fk != Some(user_id) {
        return (StatusCode::FORBIDDEN, "not your repo").into_response();
    }

    // 4. Resolve the bare repo path on disk.
    let Some(base) = crate::git_store::repos_base_dir() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "git store disabled").into_response();
    };
    let repo_dir = crate::git_store::player_repo_path(&base, session_uuid, player_uuid);
    if !repo_dir.join("HEAD").exists() {
        return (StatusCode::NOT_FOUND, "no such repo").into_response();
    }

    // 5. Spawn `git http-backend` CGI.
    let git_bin = match which::which("git") {
        Ok(p) => p,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "git not installed").into_response(),
    };

    // `player_id` from axum includes the `.git` suffix (the route pattern
    // is `:player_id.git` — axum treats the whole path segment as the param).
    // PATH_INFO must be `/{player_uuid}.git/{rest}` — reconstruct from the
    // parsed UUID to avoid doubling the `.git` suffix.
    let path_info = format!("/{player_uuid}.git/{rest}");

    let mut cmd = std::process::Command::new(git_bin);
    cmd.arg("http-backend");
    cmd.env_clear();
    // CGI env vars required by git http-backend.
    cmd.env("GIT_PROJECT_ROOT", base.join(&session_id));
    cmd.env("GIT_HTTP_EXPORT_ALL", "1");
    cmd.env("PATH_INFO", &path_info);
    cmd.env("QUERY_STRING", &query);
    cmd.env("REQUEST_METHOD", method.as_str());
    cmd.env(
        "REQUEST_URI",
        format!(
            "/git/{session_uuid}/{player_uuid}.git/{rest}{}",
            if query.is_empty() {
                String::new()
            } else {
                format!("?{query}")
            }
        ),
    );
    cmd.env("REMOTE_USER", &claims.sub);
    cmd.env("REMOTE_ADDR", "127.0.0.1");
    if let Some(ct) = headers.get("content-type")
        && let Ok(s) = ct.to_str()
    {
        cmd.env("CONTENT_TYPE", s);
    }
    if !body.is_empty() {
        cmd.env("CONTENT_LENGTH", body.len().to_string());
    }
    // git http-backend needs a minimal PATH to find itself + git subcommands.
    cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
    // Pass through git-protocol extension header (V2) so smart HTTP negotiation works.
    if let Some(gp) = headers.get("git-protocol")
        && let Ok(s) = gp.to_str()
    {
        cmd.env("GIT_PROTOCOL", s);
    }
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    // `git http-backend` is a synchronous CGI: spawning it and blocking on
    // `wait_with_output()` on the async worker would stall the runtime for the
    // whole push/fetch (concurrent pushes from many players starve tokio). Run
    // the spawn + drain on the blocking pool instead.
    let body_to_write = body.clone();
    let out = match tokio::task::spawn_blocking(move || -> std::io::Result<std::process::Output> {
        let mut cmd = cmd;
        let mut child = cmd.spawn()?;
        // Write the (already-buffered) request body to stdin on a helper thread
        // so a large push can't deadlock against git filling its stdout pipe.
        let writer = child.stdin.take().map(|mut stdin| {
            std::thread::spawn(move || {
                use std::io::Write;
                let _ = stdin.write_all(&body_to_write);
                // Drop closes stdin so git sees EOF.
            })
        });
        let out = child.wait_with_output();
        if let Some(writer) = writer {
            let _ = writer.join();
        }
        out
    })
    .await
    {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            tracing::error!("git http-backend io failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "git io failed").into_response();
        }
        Err(e) => {
            tracing::error!("git http-backend task failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "git task failed").into_response();
        }
    };

    // A failed backend produces an empty/truncated CGI response; the client
    // then reports only "unexpected disconnect while reading sideband
    // packet", so the server log is the ONLY place the real reason exists —
    // debug-level hid a broken-repo push failure for a whole session.
    if !out.status.success() || out.stdout.is_empty() {
        tracing::warn!(
            session_id,
            player_id,
            exit = ?out.status.code(),
            stdout_bytes = out.stdout.len(),
            "git http-backend failed: {}",
            String::from_utf8_lossy(&out.stderr).trim_end()
        );
    } else if !out.stderr.is_empty() {
        tracing::debug!(
            "git http-backend stderr: {}",
            String::from_utf8_lossy(&out.stderr).trim_end()
        );
    }

    // 6. Parse CGI output: headers + blank line + body.
    parse_cgi_output(&out.stdout)
}

fn extract_pat(headers: &HeaderMap) -> Option<String> {
    // Authorization: Bearer <pat> — ololo's WS/API path.
    if let Some(v) = headers.get(HeaderName::from_static("authorization"))
        && let Ok(s) = v.to_str()
        && let Some(rest) = s.strip_prefix("Bearer ")
    {
        return Some(rest.to_string());
    }
    // Authorization: Basic <base64(user:pass)> — git smart HTTP path.
    // git push sends HTTP Basic auth with the PAT as the password
    // (username is conventionally "x" or the PAT itself).
    if let Some(v) = headers.get(HeaderName::from_static("authorization"))
        && let Ok(s) = v.to_str()
        && let Some(rest) = s.strip_prefix("Basic ")
        && let Some(decoded) = base64_decode(rest)
    {
        if let Some((_user, pass)) = decoded.split_once(':')
            && !pass.is_empty()
        {
            return Some(pass.to_string());
        }
        // Some clients send the PAT as the username only.
        if let Some((user, "")) = decoded.split_once(':')
            && !user.is_empty()
        {
            return Some(user.to_string());
        }
    }
    if let Some(v) = headers.get(HeaderName::from_static("x-api-key"))
        && let Ok(s) = v.to_str()
    {
        return Some(s.to_string());
    }
    None
}

/// Minimal base64 decoder — avoids pulling in a base64 crate dependency.
fn base64_decode(s: &str) -> Option<String> {
    let table: [u8; 256] = {
        let mut t = [255u8; 256];
        for (i, c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
            .iter()
            .enumerate()
        {
            t[*c as usize] = i as u8;
        }
        t
    };
    let s = s.trim();
    let mut buf = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for ch in s.bytes() {
        if ch == b'=' {
            break;
        }
        let val = table[ch as usize];
        if val == 255 {
            continue; // skip whitespace/invalid
        }
        acc = (acc << 6) | val as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            buf.push((acc >> bits) as u8);
        }
    }
    String::from_utf8(buf).ok()
}

/// Parse the CGI stdout of `git http-backend`: a block of HTTP headers
/// separated from the body by a blank line. Returns an axum Response.
fn parse_cgi_output(raw: &[u8]) -> Response {
    let sep = find_header_body_separator(raw);
    let (header_block, body) = match sep {
        Some((idx, sep_len)) => (&raw[..idx], &raw[idx + sep_len..]),
        None => (raw, &[][..]),
    };
    let header_text = String::from_utf8_lossy(header_block);
    let mut status = StatusCode::OK;
    let mut out_headers = HeaderMap::new();
    for line in header_text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("status") {
            if let Some(code) = value.split_whitespace().next()
                && let Ok(c) = code.parse::<u16>()
                && let Ok(s) = StatusCode::from_u16(c)
            {
                status = s;
            }
            continue;
        }
        if let Ok(hn) = HeaderName::try_from(name)
            && let Ok(hv) = HeaderValue::try_from(value)
        {
            out_headers.insert(hn, hv);
        }
    }
    let mut resp = Response::builder().status(status);
    for (k, v) in out_headers.iter() {
        resp = resp.header(k.clone(), v.clone());
    }
    resp.body(Body::from(body.to_vec()))
        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "bad cgi output").into_response())
}

/// Find the `\r\n\r\n` or `\n\n` separator between headers and body.
/// Returns `(index_of_separator_start, separator_length)`.
fn find_header_body_separator(raw: &[u8]) -> Option<(usize, usize)> {
    if let Some(idx) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
        return Some((idx, 4));
    }
    raw.windows(2)
        .position(|w| w == b"\n\n")
        .map(|idx| (idx, 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cgi_output_extracts_status_and_headers() {
        let raw =
            b"Status: 200 OK\r\nContent-Type: application/x-git-upload-pack-result\r\n\r\nBODY";
        let resp = parse_cgi_output(raw);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/x-git-upload-pack-result"
        );
    }

    #[test]
    fn parse_cgi_output_handles_non_default_status() {
        let raw = b"Status: 403 Forbidden\r\n\r\n";
        let resp = parse_cgi_output(raw);
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn parse_cgi_output_handles_lf_only_separator() {
        let raw = b"Content-Type: text/plain\n\nhello";
        let resp = parse_cgi_output(raw);
        assert_eq!(resp.headers().get("content-type").unwrap(), "text/plain");
    }

    #[test]
    fn find_separator_crlf() {
        assert_eq!(find_header_body_separator(b"a\r\n\r\nb"), Some((1, 4)));
    }

    #[test]
    fn find_separator_lf() {
        assert_eq!(find_header_body_separator(b"a\n\nb"), Some((1, 2)));
    }

    #[test]
    fn find_separator_none() {
        assert!(find_header_body_separator(b"no separator").is_none());
    }

    #[test]
    fn extract_pat_from_bearer() {
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer ololo_abc"),
        );
        assert_eq!(extract_pat(&h), Some("ololo_abc".to_string()));
    }

    #[test]
    fn extract_pat_from_x_api_key() {
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_static("ololo_xyz"),
        );
        assert_eq!(extract_pat(&h), Some("ololo_xyz".to_string()));
    }

    #[test]
    fn extract_pat_missing() {
        let h = HeaderMap::new();
        assert!(extract_pat(&h).is_none());
    }

    #[test]
    fn extract_pat_from_basic_password() {
        // git push sends `Basic base64("x:ololo_pat")` — PAT in password field.
        let cred = base64_encode("x:ololo_abc123");
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Basic {cred}")).unwrap(),
        );
        assert_eq!(extract_pat(&h), Some("ololo_abc123".to_string()));
    }

    #[test]
    fn extract_pat_from_basic_username_only() {
        // Some clients send PAT as username with empty password.
        let cred = base64_encode("ololo_xyz:");
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Basic {cred}")).unwrap(),
        );
        assert_eq!(extract_pat(&h), Some("ololo_xyz".to_string()));
    }

    fn base64_encode(s: &str) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let bytes = s.as_bytes();
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
            let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            if chunk.len() > 1 {
                out.push(TABLE[((n >> 6) & 63) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                out.push(TABLE[(n & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }
}
