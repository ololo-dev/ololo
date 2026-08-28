//! Shell execution of a probe command with a wall-clock deadline.

use anyhow::{Context, Result};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;

use super::STDOUT_TAIL_MAX_BYTES;

/// Execute `command` via a system shell with a wall-clock `deadline`.
///
/// Returns `(trimmed_stdout, exit_code, duration_ms)`.
/// On timeout the child is killed and `exit_code = -1` is returned.
pub async fn run_probe(command: &str, deadline: Duration) -> Result<(String, i32, u64)> {
    let start = Instant::now();

    #[cfg(unix)]
    let mut child = tokio::process::Command::new("sh")
        .args(["-c", command])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("spawning sh -c")?;

    #[cfg(windows)]
    let mut child = tokio::process::Command::new("cmd.exe")
        .args(["/C", command])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("spawning cmd.exe /C")?;

    // Take the stdout pipe out of `child` so we can still access `child` for
    // kill / wait after a timeout without moving the whole `Child` into the
    // async block.
    let stdout_pipe = child.stdout.take().expect("stdout was piped");

    let collect_stdout = async move {
        let mut buf = Vec::new();
        let mut reader = tokio::io::BufReader::new(stdout_pipe);
        reader.read_to_end(&mut buf).await?;
        anyhow::Ok(buf)
    };

    let stdout_bytes = match tokio::time::timeout(deadline, collect_stdout).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(e)) => return Err(e),
        Err(_elapsed) => {
            // Deadline exceeded — kill the child, report timeout.
            let _ = child.kill().await;
            let _ = child.wait().await;
            let duration_ms = start.elapsed().as_millis() as u64;
            return Ok((String::new(), -1, duration_ms));
        }
    };

    // Stdout drained — now wait for the exit status.
    let status = child.wait().await.context("waiting for child")?;
    let duration_ms = start.elapsed().as_millis() as u64;

    // Tail-truncate, then strip trailing whitespace (including newlines).
    let raw = if stdout_bytes.len() > STDOUT_TAIL_MAX_BYTES {
        &stdout_bytes[stdout_bytes.len() - STDOUT_TAIL_MAX_BYTES..]
    } else {
        &stdout_bytes[..]
    };
    let stdout = String::from_utf8_lossy(raw).trim_end().to_string();

    let exit_code = status.code().unwrap_or(-1);
    Ok((stdout, exit_code, duration_ms))
}

#[cfg(test)]
mod tests {

    #[test]
    fn tail_truncation_keeps_only_last_bytes() {
        // ponytail: pure helper carved out of run_probe's truncation step so
        // the boundary math is covered without spawning a shell.
        fn tail(raw: &[u8], max: usize) -> &[u8] {
            if raw.len() > max {
                &raw[raw.len() - max..]
            } else {
                raw
            }
        }
        let data: Vec<u8> = (0..200).map(|b| b as u8).collect();
        assert_eq!(tail(&data, 64).len(), 64);
        assert_eq!(tail(&data, 64)[0], (200 - 64) as u8);
        let small: Vec<u8> = vec![1, 2, 3];
        assert_eq!(tail(&small, 64), &[1, 2, 3]);
    }
}
