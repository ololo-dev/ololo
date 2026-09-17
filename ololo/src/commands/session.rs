//! `ololo session …` — talk to a running session's control service.
//!
//! The TUI publishes a loopback HTTP service per session (see
//! `crate::control`); these subcommands are its command-line client, for
//! scripts and for an agent driving a session from another terminal:
//! read the session as the TUI sees it, read the hosted agent's screen,
//! type into it, answer its dialogs, answer ololo's own probe prompts.

use anyhow::{Context, Result, anyhow, bail};
use std::path::PathBuf;

use crate::control::{HTTP_PORT_FILE, read_service};

/// The control base dir (`…/ololo/control`), honouring `OLOLO_CONTROL_DIR`.
fn control_base() -> Option<PathBuf> {
    match std::env::var_os("OLOLO_CONTROL_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None => dirs::config_dir().map(|d| d.join("ololo").join("control")),
    }
}

/// Every session dir that currently publishes a service port.
fn live_sessions() -> Vec<(String, PathBuf)> {
    let Some(base) = control_base() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&base) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().join(HTTP_PORT_FILE).is_file())
        .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
        .collect();
    out.sort();
    out
}

/// Resolve which session to talk to: `--code`, else `$OLOLO_SESSION`, else
/// the only live one.
fn resolve(code: Option<&str>) -> Result<(String, String, String)> {
    let live = live_sessions();
    let code = match code
        .map(str::to_string)
        .or_else(|| std::env::var("OLOLO_SESSION").ok())
    {
        Some(c) => c.to_uppercase(),
        None => match live.as_slice() {
            [] => bail!(
                "no live session: start one with `ololo start <slug> --tui --agent <cmd>` \
                 (the service runs with the TUI)"
            ),
            [(code, _)] => code.clone(),
            many => bail!(
                "several live sessions ({}) — pick one with --code",
                many.iter()
                    .map(|(c, _)| c.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        },
    };
    let dir = control_base()
        .ok_or_else(|| anyhow!("no config dir"))?
        .join(&code);
    let (port, token) =
        read_service(&dir).ok_or_else(|| anyhow!("session {code} has no service running"))?;
    Ok((code, format!("http://127.0.0.1:{port}"), token))
}

async fn call(
    code: Option<&str>,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<(u16, String)> {
    let (_, base, token) = resolve(code)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let mut req = client
        .request(method, format!("{base}{path}"))
        .bearer_auth(token);
    if let Some(body) = body {
        req = req.json(&body);
    }
    let resp = req
        .send()
        .await
        .with_context(|| format!("the session service at {base} did not answer"))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    Ok((status, text))
}

fn fail_unless_ok(status: u16, text: &str) -> Result<()> {
    if (200..300).contains(&status) {
        return Ok(());
    }
    let message = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| text.to_string());
    bail!("{status}: {message}")
}

pub fn run_session_list() -> Result<()> {
    let live = live_sessions();
    if live.is_empty() {
        println!("no live sessions");
        return Ok(());
    }
    for (code, dir) in live {
        match read_service(&dir) {
            Some((port, _)) => println!("{code}\thttp://127.0.0.1:{port}"),
            None => println!("{code}\t(no service)"),
        }
    }
    Ok(())
}

pub async fn run_session_status(code: Option<&str>, json: bool) -> Result<()> {
    let (status, text) = call(code, reqwest::Method::GET, "/session", None).await?;
    fail_unless_ok(status, &text)?;
    if json {
        println!("{text}");
        return Ok(());
    }
    let v: serde_json::Value = serde_json::from_str(&text)?;
    let s = |k: &str| {
        v.get(k)
            .map(|x| x.to_string().trim_matches('"').to_string())
            .unwrap_or_default()
    };
    println!(
        "session {} — {} — {} — score {} — task {}/{}",
        s("session"),
        s("project"),
        s("status"),
        s("score"),
        s("current_task"),
        s("total_tasks")
    );
    if let Some(tasks) = v.get("tasks").and_then(|t| t.as_array()) {
        for t in tasks {
            let checks = t
                .get("checks")
                .and_then(|c| c.as_array())
                .map(|c| c.len())
                .unwrap_or(0);
            let passed = t
                .get("checks")
                .and_then(|c| c.as_array())
                .map(|c| {
                    c.iter()
                        .filter(|p| p.get("outcome").and_then(|o| o.as_str()) == Some("pass"))
                        .count()
                })
                .unwrap_or(0);
            println!(
                "  #{} {} — {}{} checks {}/{} — points {}",
                t.get("ordinal").map(|o| o.to_string()).unwrap_or_default(),
                t.get("title").and_then(|x| x.as_str()).unwrap_or(""),
                if t.get("current").and_then(|c| c.as_bool()) == Some(true) {
                    "current, "
                } else {
                    ""
                },
                if t.get("passed").and_then(|c| c.as_bool()) == Some(true) {
                    "passed, "
                } else {
                    ""
                },
                passed,
                checks,
                t.get("points")
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".into())
            );
        }
    }
    if let Some(runs) = v.get("judge_runs").and_then(|r| r.as_array()) {
        for r in runs
            .iter()
            .filter(|r| r.get("state").and_then(|s| s.as_str()) == Some("reviewing"))
        {
            println!(
                "  judge {} reviewing task #{}",
                r.get("judge").and_then(|x| x.as_str()).unwrap_or(""),
                r.get("task_ordinal")
                    .map(|o| o.to_string())
                    .unwrap_or_default()
            );
        }
    }
    if let Some(verdicts) = v.get("verdicts").and_then(|r| r.as_array()) {
        for r in verdicts {
            println!(
                "  verdict {} on #{}: {} pts",
                r.get("judge").and_then(|x| x.as_str()).unwrap_or(""),
                r.get("task_ordinal")
                    .map(|o| o.to_string())
                    .unwrap_or_default(),
                r.get("points").map(|o| o.to_string()).unwrap_or_default()
            );
        }
    }
    if let Some(p) = v.get("permission_prompt").filter(|p| !p.is_null()) {
        println!(
            "  PERMISSION PROMPT open: {}  (answer with `ololo session permission allow|always|session|deny`)",
            p.get("command").and_then(|c| c.as_str()).unwrap_or("")
        );
    }
    Ok(())
}

pub async fn run_session_screen(code: Option<&str>) -> Result<()> {
    let (status, text) = call(code, reqwest::Method::GET, "/screen", None).await?;
    fail_unless_ok(status, &text)?;
    println!("{text}");
    Ok(())
}

pub async fn run_session_send(code: Option<&str>, text: &str) -> Result<()> {
    let (status, body) = call(
        code,
        reqwest::Method::POST,
        "/agent/message",
        Some(serde_json::json!({ "text": text })),
    )
    .await?;
    fail_unless_ok(status, &body)?;
    crate::ui::success("sent to the agent");
    Ok(())
}

pub async fn run_session_keys(code: Option<&str>, keys: &[String]) -> Result<()> {
    let (status, body) = call(
        code,
        reqwest::Method::POST,
        "/agent/keys",
        Some(serde_json::json!({ "keys": keys })),
    )
    .await?;
    fail_unless_ok(status, &body)?;
    crate::ui::success("keys sent");
    Ok(())
}

pub async fn run_session_agent_permission(code: Option<&str>, answer: &str) -> Result<()> {
    let body = match answer {
        "allow" | "yes" | "y" => serde_json::json!({ "allow": true }),
        "deny" | "no" | "n" => serde_json::json!({ "allow": false }),
        digit => match digit.parse::<u64>() {
            Ok(n) => serde_json::json!({ "choice": n }),
            Err(_) => bail!("answer with allow, deny, or the option number"),
        },
    };
    let (status, text) = call(code, reqwest::Method::POST, "/agent/permission", Some(body)).await?;
    fail_unless_ok(status, &text)?;
    crate::ui::success("answered the agent's dialog");
    Ok(())
}

pub async fn run_session_permission(code: Option<&str>, decision: &str) -> Result<()> {
    if crate::control::parse_decision(decision).is_none() {
        bail!("decision must be allow, always, session or deny");
    }
    let (status, text) = call(
        code,
        reqwest::Method::POST,
        "/permission",
        Some(serde_json::json!({ "decision": decision })),
    )
    .await?;
    fail_unless_ok(status, &text)?;
    crate::ui::success("probe permission answered");
    Ok(())
}
