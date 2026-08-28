//! `ololo update`: self-update from GitHub releases, plus the passive
//! update-available check other commands use for the "new version" notice.
//!
//! The passive check hits the GitHub API at most once per 24h and caches
//! the answer in `~/.config/ololo/update-check.json`; everything about it
//! is best-effort — no network error may ever disturb a session.

use anyhow::{Context, Result, anyhow, bail};
use std::path::PathBuf;
use std::time::Duration;

use crate::ui;

const REPO: &str = "ololo-dev/ololo";
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// `true` when `latest` is a strictly newer dotted-numeric version than
/// `current`. Non-numeric components compare as 0; missing trail as 0.
pub fn is_newer(latest: &str, current: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .map(|p| {
                p.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            })
            .collect()
    }
    let (l, c) = (parts(latest), parts(current));
    let len = l.len().max(c.len());
    for i in 0..len {
        let (a, b) = (
            l.get(i).copied().unwrap_or(0),
            c.get(i).copied().unwrap_or(0),
        );
        if a != b {
            return a > b;
        }
    }
    false
}

// ── Cache ─────────────────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct CheckCache {
    checked_at_unix: u64,
    latest: String,
}

fn cache_path() -> Option<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    Some(
        std::path::Path::new(&home)
            .join(".config")
            .join("ololo")
            .join("update-check.json"),
    )
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_cache() -> Option<CheckCache> {
    let raw = std::fs::read_to_string(cache_path()?).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_cache(latest: &str) {
    let Some(path) = cache_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let cache = CheckCache {
        checked_at_unix: now_unix(),
        latest: latest.to_string(),
    };
    if let Ok(json) = serde_json::to_string(&cache) {
        let _ = std::fs::write(path, json);
    }
}

/// A newer version according to the *cache only* (no network). Used where
/// blocking on the network is unacceptable, e.g. the TUI header.
pub fn cached_newer_version() -> Option<String> {
    let cache = read_cache()?;
    is_newer(&cache.latest, current_version()).then_some(cache.latest)
}

// ── Passive check ─────────────────────────────────────────────────────────────

/// Background refresh: when the cache is stale, ask GitHub for the latest
/// release and re-cache it. Resolves to the newer version, if one exists.
/// Never errors — a failed check just means no notice this run.
pub fn spawn_check() -> tokio::task::JoinHandle<Option<String>> {
    tokio::spawn(async {
        if let Some(cache) = read_cache()
            && now_unix().saturating_sub(cache.checked_at_unix) < CHECK_INTERVAL.as_secs()
        {
            return is_newer(&cache.latest, current_version()).then_some(cache.latest);
        }
        let client = reqwest::Client::new();
        let latest = fetch_latest_version(&client).await.ok()?;
        write_cache(&latest);
        is_newer(&latest, current_version()).then_some(latest)
    })
}

/// Print the "new version available" notice (plain terminal, post-command).
pub fn notice(latest: &str) {
    ui::hint(format!(
        "A new ololo release is available: v{} → v{latest}. Run `ololo update`.",
        current_version()
    ));
}

async fn fetch_latest_version(client: &reqwest::Client) -> Result<String> {
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
    }
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let release: Release = client
        .get(&url)
        .header(
            reqwest::header::USER_AGENT,
            format!("ololo/{}", current_version()),
        )
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .context("latest-release lookup failed")?
        .json()
        .await
        .context("parsing latest-release response")?;
    Ok(release.tag_name.trim_start_matches('v').to_string())
}

// ── ololo update ──────────────────────────────────────────────────────────────

/// Release artifact for this platform, mirroring the installer scripts.
fn platform_artifact() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "x86_64") => Ok("ololo-macos-x86_64.tar.gz"),
        ("macos", "aarch64") => Ok("ololo-macos-aarch64.tar.gz"),
        ("linux", "x86_64") => Ok("ololo-linux-x86_64.tar.gz"),
        ("linux", "aarch64") => Ok("ololo-linux-aarch64.tar.gz"),
        ("windows", "x86_64") => Ok("ololo-windows-x86_64.zip"),
        (os, arch) => bail!("no prebuilt ololo release for {os}/{arch}"),
    }
}

pub async fn run_update(check_only: bool) -> Result<()> {
    let client = reqwest::Client::new();
    ui::step("Checking the latest release...");
    let latest = fetch_latest_version(&client).await?;
    write_cache(&latest);

    let current = current_version();
    if !is_newer(&latest, current) {
        ui::success(format!("ololo v{current} is up to date."));
        return Ok(());
    }
    if check_only {
        ui::warn(format!(
            "Update available: v{current} → v{latest}. Run `ololo update` to install."
        ));
        return Ok(());
    }

    let artifact = platform_artifact()?;
    let url = format!("https://github.com/{REPO}/releases/download/v{latest}/{artifact}");
    ui::step(format!("Downloading {url}"));
    let bytes = client
        .get(&url)
        .timeout(Duration::from_secs(300))
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .context("release download failed")?
        .bytes()
        .await
        .context("reading release archive")?;

    let tmp = std::env::temp_dir().join(format!("ololo-update-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).context("creating temp dir")?;
    // Best-effort cleanup on every exit path below.
    let _cleanup = TempDirGuard(tmp.clone());
    let archive = tmp.join(artifact);
    std::fs::write(&archive, &bytes).context("writing release archive")?;

    // `tar` ships with every supported platform (Windows 10+ bsdtar reads
    // zip too) — the same assumption install.sh makes about unix.
    let status = std::process::Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(&tmp)
        .status()
        .context("running tar (is it installed?)")?;
    if !status.success() {
        bail!("extracting {artifact} failed (tar exit: {status})");
    }

    let bin_name = if cfg!(windows) { "ololo.exe" } else { "ololo" };
    let new_bin = tmp.join(bin_name);
    if !new_bin.is_file() {
        bail!("archive did not contain the {bin_name} binary");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new_bin, std::fs::Permissions::from_mode(0o755))
            .context("marking new binary executable")?;
    }

    let target = std::env::current_exe().context("locating current ololo binary")?;
    let target = std::fs::canonicalize(&target).unwrap_or(target);
    replace_binary(&new_bin, &target).with_context(|| {
        format!(
            "replacing {} (try re-running with write access, or reinstall: \
             curl -fsSL https://ololo.dev/install.sh | bash)",
            target.display()
        )
    })?;

    ui::success(format!("Updated ololo: v{current} → v{latest}"));
    Ok(())
}

/// Swap `new_bin` into `target`'s place. The staging copy lands in the
/// target's directory first so the final step is a same-filesystem rename.
fn replace_binary(new_bin: &std::path::Path, target: &std::path::Path) -> Result<()> {
    let dir = target
        .parent()
        .ok_or_else(|| anyhow!("target binary has no parent directory"))?;
    let staged = dir.join(format!(".ololo-new-{}", std::process::id()));
    std::fs::copy(new_bin, &staged).context("staging new binary next to the old one")?;

    #[cfg(unix)]
    {
        // rename() atomically replaces the running binary on unix.
        if let Err(e) = std::fs::rename(&staged, target) {
            let _ = std::fs::remove_file(&staged);
            return Err(e).context("renaming new binary into place");
        }
    }
    #[cfg(windows)]
    {
        // Windows can't overwrite a running exe, but it CAN rename it away.
        let old = dir.join(format!(".ololo-old-{}", std::process::id()));
        std::fs::rename(target, &old).context("moving the running binary aside")?;
        if let Err(e) = std::fs::rename(&staged, target) {
            // Roll back so the user still has a working binary.
            let _ = std::fs::rename(&old, target);
            let _ = std::fs::remove_file(&staged);
            return Err(e).context("renaming new binary into place");
        }
        let _ = std::fs::remove_file(&old); // fails while running; harmless leftover
    }
    Ok(())
}

struct TempDirGuard(PathBuf);
impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_version_comparisons() {
        assert!(is_newer("0.12.0", "0.11.1"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("0.11.2", "0.11.1"));
        assert!(is_newer("v0.12.0", "0.11.1"), "v prefix is tolerated");
        assert!(!is_newer("0.11.1", "0.11.1"));
        assert!(!is_newer("0.11.0", "0.11.1"));
        assert!(!is_newer("0.9.9", "0.11.1"));
    }

    #[test]
    fn shorter_versions_compare_as_zero_padded() {
        assert!(is_newer("0.12", "0.11.9"));
        assert!(!is_newer("0.11", "0.11.0"));
        assert!(is_newer("0.11.0.1", "0.11.0"));
    }

    #[test]
    fn garbage_components_compare_as_zero() {
        assert!(!is_newer("abc", "0.0.1"));
        assert!(is_newer("0.12.0-rc1", "0.11.1"), "prerelease digits count");
    }

    #[test]
    fn platform_artifact_resolves_on_supported_hosts() {
        // The test runs on a supported dev platform; the mapping must hit.
        assert!(platform_artifact().is_ok());
    }
}
