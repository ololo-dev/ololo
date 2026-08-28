//! Antigravity IDE usage sync.
//!
//! The Antigravity IDE exposes token usage only through its language
//! server's local Connect RPC — there is no on-disk usage log to read. When
//! the player picks the `antigravity` agent, this module periodically
//! discovers the running language servers, pulls per-session usage over
//! that RPC, and writes JSONL artifacts to
//! `~/.config/ololo/antigravity-cache/sessions/`, where the agent-tokens
//! antigravity extractor picks them up like any other agent log.
//!
//! Discovery, the RPC shape and the artifact line format are ported from
//! tokscale (junhoyeo/tokscale, MIT, `crates/tokscale-cli/src/antigravity.rs`).
//! Unix-only: process discovery rides `ps` and `lsof`, and Antigravity
//! itself ships for macOS and Linux.

#[cfg(unix)]
pub use imp::spawn_if_selected;

#[cfg(not(unix))]
pub fn spawn_if_selected(_agent_name: &str) -> Option<tokio::task::JoinHandle<()>> {
    None
}

#[cfg(unix)]
mod imp {
    use anyhow::{Context, Result, anyhow};
    use serde_json::{Value, json};
    use std::path::Path;
    use std::time::Duration;

    const RPC_SERVICE: &str = "exa.language_server_pb.LanguageServerService";
    const SYNC_INTERVAL: Duration = Duration::from_secs(20);
    /// Newest-first cap on sessions fetched per sync pass. Bounds RPC time
    /// on IDEs with a long history; the game only needs the sessions that
    /// are active during the match.
    const MAX_SESSIONS_PER_SYNC: usize = 20;

    /// Spawn the background sync loop when the chosen agent is the
    /// Antigravity IDE. The `agy` CLI logs usage to sqlite on disk and
    /// needs no sync.
    pub fn spawn_if_selected(agent_name: &str) -> Option<tokio::task::JoinHandle<()>> {
        if agent_name != "antigravity" {
            return None;
        }
        Some(tokio::spawn(async move {
            loop {
                match sync_once().await {
                    Ok(written) if written > 0 => {
                        tracing::info!("antigravity sync: {written} session artifact(s) updated");
                    }
                    Ok(_) => {}
                    // The IDE may simply not be up yet; keep retrying quietly.
                    Err(e) => tracing::debug!("antigravity sync failed: {e:#}"),
                }
                tokio::time::sleep(SYNC_INTERVAL).await;
            }
        }))
    }

    /// One discovery+fetch+write pass. Returns the number of artifacts
    /// whose content changed.
    async fn sync_once() -> Result<usize> {
        let candidates = tokio::task::spawn_blocking(detect_candidates)
            .await
            .context("candidate discovery task panicked")??;
        if candidates.is_empty() {
            return Ok(0);
        }

        let client = rpc_client()?;
        let mut connections: Vec<Connection> = Vec::new();
        for candidate in candidates {
            let pid = candidate.pid;
            let mut ports = tokio::task::spawn_blocking(move || listening_ports(pid))
                .await
                .unwrap_or_default();
            if let Some(declared) = candidate.declared_port
                && !ports.contains(&declared)
            {
                ports.push(declared);
            }
            for port in ports {
                if heartbeat_ok(&client, port, &candidate.csrf_token).await {
                    connections.push(Connection {
                        port,
                        csrf_token: candidate.csrf_token.clone(),
                    });
                    break;
                }
            }
        }
        if connections.is_empty() {
            return Ok(0);
        }

        // List sessions across all servers, dedup by id keeping the freshest.
        let mut summaries: Vec<(Summary, usize)> = Vec::new();
        for (idx, connection) in connections.iter().enumerate() {
            let Ok(response) =
                rpc(&client, connection, "GetAllCascadeTrajectories", &json!({})).await
            else {
                continue;
            };
            for summary in normalize_summaries(&response) {
                match summaries
                    .iter_mut()
                    .find(|(existing, _)| existing.session_id == summary.session_id)
                {
                    Some(slot) if slot.0.last_modified_ms >= summary.last_modified_ms => {}
                    Some(slot) => *slot = (summary, idx),
                    None => summaries.push((summary, idx)),
                }
            }
        }
        summaries.sort_by_key(|entry| std::cmp::Reverse(entry.0.last_modified_ms));
        summaries.truncate(MAX_SESSIONS_PER_SYNC);

        let dir = agent_tokens::paths::ololo_antigravity_cache_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating antigravity cache dir {}", dir.display()))?;

        let mut written = 0_usize;
        for (summary, idx) in summaries {
            let connection = &connections[idx];
            let Ok(response) = rpc(
                &client,
                connection,
                "GetCascadeTrajectoryGeneratorMetadata",
                &json!({ "cascadeId": summary.session_id }),
            )
            .await
            else {
                continue;
            };
            let metadata = response
                .get("generatorMetadata")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if metadata.is_empty() {
                continue;
            }
            let lines = metadata_to_jsonl(&summary.session_id, &metadata);
            if lines.is_empty() {
                continue;
            }
            let contents = format!("{}\n", lines.join("\n"));
            if write_artifact(&dir, &summary.session_id, &contents)? {
                written += 1;
            }
        }
        Ok(written)
    }

    #[derive(Debug, Clone)]
    struct Connection {
        port: u16,
        csrf_token: String,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct Candidate {
        pid: u32,
        declared_port: Option<u16>,
        csrf_token: String,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct Summary {
        session_id: String,
        last_modified_ms: i64,
    }

    // ---------- process discovery ----------

    /// Find Antigravity language-server processes by their argv: they carry
    /// a `--csrf_token` the RPC requires, so a process without one is
    /// unusable anyway. Same-user argv is spoofable in principle; the
    /// heartbeat probe is the second gate.
    fn detect_candidates() -> Result<Vec<Candidate>> {
        let output = run_command("ps", &["-ww", "-eo", "pid,args"])?;
        let mut candidates: Vec<Candidate> = Vec::new();
        for line in output.lines() {
            let trimmed = line.trim();
            let mut parts = trimmed.splitn(2, char::is_whitespace);
            let Some(pid) = parts.next().and_then(|p| p.parse::<u32>().ok()) else {
                continue;
            };
            let Some(command) = parts.next().map(str::trim) else {
                continue;
            };
            if !is_antigravity_process(command) {
                continue;
            }
            let Some(csrf_token) = extract_csrf_token(command) else {
                continue;
            };
            if candidates.iter().any(|c| c.pid == pid) {
                continue;
            }
            candidates.push(Candidate {
                pid,
                declared_port: extract_declared_port(command),
                csrf_token,
            });
        }
        Ok(candidates)
    }

    fn listening_ports(pid: u32) -> Vec<u16> {
        let pid_str = pid.to_string();
        let mut ports = run_command("lsof", &["-Pan", "-p", &pid_str, "-iTCP", "-sTCP:LISTEN"])
            .map(|out| parse_lsof_ports(&out))
            .unwrap_or_default();
        if ports.is_empty() {
            ports = run_command("lsof", &["-Pan", "-p", &pid_str, "-i"])
                .map(|out| parse_lsof_ports(&out))
                .unwrap_or_default();
        }
        ports.sort_unstable();
        ports.dedup();
        ports
    }

    fn run_command(program: &str, args: &[&str]) -> Result<String> {
        let output = std::process::Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("running {program}"))?;
        // lsof exits non-zero when the pid has no matching descriptors; its
        // stdout is still the answer, so status is deliberately not checked.
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn is_antigravity_process(command: &str) -> bool {
        let lower = command.to_lowercase();
        (lower.contains("language_server")
            && (lower.contains("antigravity") || lower.contains("--app_data_dir antigravity")))
            || lower.contains("/antigravity/")
            || lower.contains("\\antigravity\\")
    }

    fn extract_csrf_token(command: &str) -> Option<String> {
        let token = extract_flag_value(command, "--csrf_token")?;
        if token.len() >= 32 && token.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-') {
            Some(token)
        } else {
            None
        }
    }

    fn extract_declared_port(command: &str) -> Option<u16> {
        extract_flag_value(command, "--extension_server_port")?
            .parse::<u16>()
            .ok()
    }

    fn extract_flag_value(command: &str, flag: &str) -> Option<String> {
        let compact = format!("{flag}=");
        if let Some(idx) = command.find(&compact) {
            let rest = &command[idx + compact.len()..];
            return rest
                .split_whitespace()
                .next()
                .map(|value| value.to_string());
        }
        let idx = command.find(flag)?;
        let rest = &command[idx + flag.len()..];
        rest.split_whitespace()
            .find(|value| !value.is_empty())
            .map(|value| value.trim().to_string())
    }

    fn parse_lsof_ports(output: &str) -> Vec<u16> {
        let mut ports = Vec::new();
        for line in output.lines() {
            for token in line.split_whitespace() {
                if let Some(port) = token
                    .strip_prefix("127.0.0.1:")
                    .or_else(|| token.strip_prefix("localhost:"))
                    .or_else(|| token.strip_prefix("*:"))
                    .or_else(|| token.strip_prefix("[::1]:"))
                {
                    let cleaned = port.trim_end_matches("(LISTEN)").trim_end_matches(',');
                    if let Ok(parsed) = cleaned.parse::<u16>() {
                        ports.push(parsed);
                    }
                }
            }
        }
        ports
    }

    // ---------- RPC ----------

    fn rpc_client() -> Result<reqwest::Client> {
        reqwest::Client::builder()
            // The language server speaks HTTPS with a self-signed cert on
            // 127.0.0.1; trust is the csrf token plus the loopback address.
            .danger_accept_invalid_certs(true)
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .context("building antigravity RPC client")
    }

    async fn heartbeat_ok(client: &reqwest::Client, port: u16, csrf_token: &str) -> bool {
        let connection = Connection {
            port,
            csrf_token: csrf_token.to_string(),
        };
        let body = json!({ "uuid": "00000000-0000-0000-0000-000000000000" });
        match rpc(client, &connection, "Heartbeat", &body).await {
            Ok(value) => value.is_object() || value.is_array(),
            Err(_) => false,
        }
    }

    /// One Connect-RPC call; the server may speak plain HTTP or HTTPS
    /// depending on build, so try both schemes.
    async fn rpc(
        client: &reqwest::Client,
        connection: &Connection,
        method: &str,
        body: &Value,
    ) -> Result<Value> {
        let mut last_err: Option<anyhow::Error> = None;
        for scheme in ["http", "https"] {
            let url = format!(
                "{scheme}://127.0.0.1:{}/{RPC_SERVICE}/{method}",
                connection.port
            );
            match client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Connect-Protocol-Version", "1")
                .header("X-Codeium-Csrf-Token", &connection.csrf_token)
                .json(body)
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    return response
                        .json::<Value>()
                        .await
                        .with_context(|| format!("parsing antigravity RPC {method} response"));
                }
                Ok(response) => {
                    last_err = Some(anyhow!(
                        "antigravity RPC {method}: status {}",
                        response.status()
                    ));
                }
                Err(e) => last_err = Some(e.into()),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("antigravity RPC {method}: no scheme attempted")))
    }

    // ---------- response shaping ----------

    /// The trajectory list arrives in one of three shapes across IDE
    /// builds: `trajectorySummaries` as an array, `trajectorySummaries` as
    /// a map keyed by id, or `cascadeTrajectories` as an array.
    fn normalize_summaries(response: &Value) -> Vec<Summary> {
        let items: Vec<Value> = if let Some(array) = response
            .get("trajectorySummaries")
            .and_then(Value::as_array)
        {
            array.to_vec()
        } else if let Some(object) = response
            .get("trajectorySummaries")
            .and_then(Value::as_object)
        {
            object
                .iter()
                .map(|(key, value)| {
                    let mut entry = value.clone();
                    if entry.get("cascadeId").is_none() {
                        entry["cascadeId"] = Value::String(key.clone());
                    }
                    entry
                })
                .collect()
        } else if let Some(array) = response
            .get("cascadeTrajectories")
            .and_then(Value::as_array)
        {
            array.to_vec()
        } else {
            Vec::new()
        };

        items
            .iter()
            .filter_map(|item| {
                let session_id = ["cascadeId", "trajectoryId", "id", "sessionId"]
                    .iter()
                    .find_map(|key| item.get(*key).and_then(Value::as_str))
                    .filter(|text| !text.trim().is_empty())?
                    .to_string();
                let last_modified_ms = [
                    "lastModifiedTime",
                    "lastModified",
                    "updatedAt",
                    "modifiedAt",
                ]
                .iter()
                .find_map(|key| item.get(*key).and_then(parse_timestamp_value))
                .unwrap_or(0);
                Some(Summary {
                    session_id,
                    last_modified_ms,
                })
            })
            .collect()
    }

    /// Convert one session's `generatorMetadata` into the JSONL artifact
    /// lines the agent-tokens antigravity extractor parses: one
    /// `session_meta` line per metadata entry, one `usage` line per
    /// non-empty retry usage record.
    fn metadata_to_jsonl(session_id: &str, metadata: &[Value]) -> Vec<String> {
        let mut lines = Vec::new();
        for meta in metadata {
            let chat_model = meta.get("chatModel").unwrap_or(meta);
            let model_id = chat_model
                .get("responseModel")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    chat_model
                        .get("model")
                        .and_then(Value::as_str)
                        .filter(|value| !value.trim().is_empty())
                })
                .unwrap_or("unknown")
                .to_string();
            let created_at = chat_model
                .get("chatStartMetadata")
                .and_then(|value| value.get("createdAt"))
                .and_then(parse_timestamp_value);

            lines.push(
                json!({
                    "type": "session_meta",
                    "sessionId": session_id,
                    "modelId": model_id,
                    "timestamp": created_at,
                })
                .to_string(),
            );

            let Some(retry_infos) = chat_model.get("retryInfos").and_then(Value::as_array) else {
                continue;
            };
            for retry in retry_infos {
                let usage = retry.get("usage").unwrap_or(retry);
                let input = to_safe_i64(usage.get("inputTokens"));
                let output = to_safe_i64(usage.get("outputTokens"));
                let cache_read = to_safe_i64(usage.get("cacheReadTokens"));
                let reasoning = to_safe_i64(usage.get("thinkingOutputTokens"));
                if input == 0 && output == 0 && cache_read == 0 && reasoning == 0 {
                    continue;
                }
                let timestamp = usage
                    .get("createdAt")
                    .or_else(|| usage.get("timestamp"))
                    .and_then(parse_timestamp_value)
                    .or(created_at);
                lines.push(
                    json!({
                        "type": "usage",
                        "sessionId": session_id,
                        "modelId": model_id,
                        "timestamp": timestamp,
                        "input": input,
                        "output": output,
                        "cacheRead": cache_read,
                        "cacheWrite": 0,
                        "reasoning": reasoning,
                        "responseId": usage.get("responseId").and_then(Value::as_str),
                    })
                    .to_string(),
                );
            }
        }
        lines
    }

    fn to_safe_i64(value: Option<&Value>) -> i64 {
        value
            .and_then(|inner| {
                inner
                    .as_i64()
                    .or_else(|| inner.as_u64().and_then(|number| i64::try_from(number).ok()))
                    .or_else(|| inner.as_str().and_then(|text| text.parse::<i64>().ok()))
            })
            .unwrap_or(0)
            .max(0)
    }

    /// Epoch-ms from an integer, a numeric string, or an RFC 3339 string.
    fn parse_timestamp_value(value: &Value) -> Option<i64> {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
            .or_else(|| {
                value.as_str().and_then(|text| {
                    text.parse::<i64>().ok().or_else(|| {
                        chrono::DateTime::parse_from_rfc3339(text)
                            .ok()
                            .map(|datetime| datetime.timestamp_millis())
                    })
                })
            })
            .filter(|timestamp| *timestamp > 0)
    }

    // ---------- artifact writing ----------

    /// `<sanitized-id>-<sha256[..16]>.jsonl`, matching tokscale's naming so
    /// the extractor's cross-cache dedup by file name recognizes the same
    /// session.
    fn artifact_stem(session_id: &str) -> String {
        use sha2::{Digest, Sha256};
        let sanitized: String = session_id
            .trim()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let trimmed = sanitized.trim_matches('-');
        let base = if trimmed.is_empty() {
            "session"
        } else {
            trimmed
        };
        let hash = format!("{:x}", Sha256::digest(session_id.as_bytes()));
        format!("{}-{}", base, &hash[..16])
    }

    /// Write the artifact atomically; returns whether the content changed.
    fn write_artifact(dir: &Path, session_id: &str, contents: &str) -> Result<bool> {
        let stem = artifact_stem(session_id);
        let path = dir.join(format!("{stem}.jsonl"));
        if std::fs::read_to_string(&path).is_ok_and(|existing| existing == contents) {
            return Ok(false);
        }
        let temp = dir.join(format!(".tmp-{stem}-{}", std::process::id()));
        std::fs::write(&temp, contents)
            .with_context(|| format!("writing antigravity artifact {}", temp.display()))?;
        std::fs::rename(&temp, &path)
            .with_context(|| format!("publishing antigravity artifact {}", path.display()))?;
        Ok(true)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn detects_language_server_argv() {
            assert!(is_antigravity_process(
                "/opt/Antigravity/language_server_linux --csrf_token abc --app_data_dir antigravity"
            ));
            assert!(is_antigravity_process(
                "/Applications/Antigravity.app/Contents/MacOS/language_server --extension_server_port 4242"
            ));
            assert!(!is_antigravity_process(
                "/usr/bin/language_server --generic"
            ));
            assert!(!is_antigravity_process("vim antigravity-notes.md"));
        }

        #[test]
        fn csrf_token_requires_hexish_32_chars() {
            let token = "0123456789abcdef0123456789abcdef";
            let cmd = format!("language_server antigravity --csrf_token {token}");
            assert_eq!(extract_csrf_token(&cmd).as_deref(), Some(token));
            let cmd = format!("language_server antigravity --csrf_token={token}");
            assert_eq!(extract_csrf_token(&cmd).as_deref(), Some(token));
            assert_eq!(
                extract_csrf_token("language_server antigravity --csrf_token short"),
                None
            );
        }

        #[test]
        fn declared_port_parses() {
            assert_eq!(
                extract_declared_port("ls antigravity --extension_server_port 4242 --x"),
                Some(4242)
            );
            assert_eq!(extract_declared_port("ls antigravity"), None);
        }

        #[test]
        fn lsof_ports_parse_loopback_listeners() {
            let out = "COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\n\
                       language 42 apk 23u IPv4 0x0 0t0 TCP 127.0.0.1:4243 (LISTEN)\n\
                       language 42 apk 24u IPv6 0x0 0t0 TCP [::1]:4244 (LISTEN)\n\
                       language 42 apk 25u IPv4 0x0 0t0 TCP *:4245 (LISTEN)\n";
            assert_eq!(parse_lsof_ports(out), vec![4243, 4244, 4245]);
        }

        #[test]
        fn summaries_normalize_all_three_shapes() {
            let array = json!({ "trajectorySummaries": [
                { "cascadeId": "s1", "lastModifiedTime": 100 },
                { "trajectoryId": "s2", "updatedAt": "2026-08-15T10:00:00Z" },
            ] });
            let got = normalize_summaries(&array);
            assert_eq!(got.len(), 2);
            assert_eq!(got[0].session_id, "s1");
            assert_eq!(got[0].last_modified_ms, 100);
            assert!(got[1].last_modified_ms > 1_700_000_000_000);

            let map = json!({ "trajectorySummaries": { "s3": { "stepCount": 4 } } });
            let got = normalize_summaries(&map);
            assert_eq!(got.len(), 1);
            assert_eq!(got[0].session_id, "s3");

            let cascade = json!({ "cascadeTrajectories": [ { "id": "s4" } ] });
            assert_eq!(normalize_summaries(&cascade)[0].session_id, "s4");

            assert!(normalize_summaries(&json!({})).is_empty());
        }

        #[test]
        fn metadata_becomes_session_meta_and_usage_lines() {
            let metadata = vec![json!({
                "chatModel": {
                    "responseModel": "gemini-3-pro",
                    "chatStartMetadata": { "createdAt": 1_000 },
                    "retryInfos": [
                        { "usage": {
                            "inputTokens": 10, "outputTokens": "5",
                            "cacheReadTokens": 2, "thinkingOutputTokens": 1,
                            "createdAt": 2_000, "responseId": "r1"
                        } },
                        { "usage": { "inputTokens": 0, "outputTokens": 0 } },
                    ]
                }
            })];
            let lines = metadata_to_jsonl("sess", &metadata);
            assert_eq!(lines.len(), 2, "zero-usage retry must be skipped");

            let meta: Value = serde_json::from_str(&lines[0]).unwrap();
            assert_eq!(meta["type"], "session_meta");
            assert_eq!(meta["modelId"], "gemini-3-pro");

            let usage: Value = serde_json::from_str(&lines[1]).unwrap();
            assert_eq!(usage["type"], "usage");
            assert_eq!(usage["sessionId"], "sess");
            assert_eq!(usage["input"], 10);
            assert_eq!(usage["output"], 5);
            assert_eq!(usage["cacheRead"], 2);
            assert_eq!(usage["cacheWrite"], 0);
            assert_eq!(usage["reasoning"], 1);
            assert_eq!(usage["timestamp"], 2_000);
            assert_eq!(usage["responseId"], "r1");
        }

        #[test]
        fn usage_lines_parse_in_agent_tokens_extractor() {
            let metadata = vec![json!({
                "chatModel": {
                    "model": "gemini-3-flash",
                    "chatStartMetadata": { "createdAt": 1_000 },
                    "retryInfos": [
                        { "usage": { "inputTokens": 7, "outputTokens": 3, "createdAt": 1_500 } }
                    ]
                }
            })];
            let lines = metadata_to_jsonl("round-trip", &metadata);
            let dir = tempfile::tempdir().unwrap();
            let contents = format!("{}\n", lines.join("\n"));
            assert!(write_artifact(dir.path(), "round-trip", &contents).unwrap());
            // Unchanged content is a no-op.
            assert!(!write_artifact(dir.path(), "round-trip", &contents).unwrap());
            let path = dir
                .path()
                .join(format!("{}.jsonl", artifact_stem("round-trip")));

            let counts = agent_tokens::extractors::antigravity::parse_cache_counts(&path, None);
            assert_eq!(counts.len(), 1);
            assert_eq!(counts[0].counts.input, 7);
            assert_eq!(counts[0].counts.output, 3);
            assert_eq!(counts[0].model.as_deref(), Some("gemini-3-flash"));
        }

        #[test]
        fn artifact_stem_sanitizes_and_hashes() {
            let stem = artifact_stem("a/b c");
            assert!(stem.starts_with("a-b-c-"));
            assert_eq!(stem.len(), "a-b-c-".len() + 16);
            assert!(artifact_stem("///").starts_with("session-"));
            // Distinct ids that sanitize identically still get distinct stems.
            assert_ne!(artifact_stem("a/b"), artifact_stem("a b"));
        }

        #[test]
        fn timestamps_parse_ints_strings_and_rfc3339() {
            assert_eq!(parse_timestamp_value(&json!(1500)), Some(1500));
            assert_eq!(parse_timestamp_value(&json!("1500")), Some(1500));
            assert_eq!(
                parse_timestamp_value(&json!("1970-01-01T00:00:01Z")),
                Some(1000)
            );
            assert_eq!(parse_timestamp_value(&json!(0)), None);
            assert_eq!(parse_timestamp_value(&json!("nonsense")), None);
        }
    }
}
