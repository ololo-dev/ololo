//! Artifact arrival detection for interactive probes.
//!
//! The participant answers an `ArtifactRequest` by committing the file under
//! `.ololo/artifacts/<probe_id>/` and pushing — git is the channel, the push
//! is the acknowledgment. This module is the receiving end: each ticker pass
//! it checks the player's pushed HEAD for the expected path and resolves the
//! probe from the blob it finds. Validation reads the blob's size against
//! the request's cap; an oversized artifact resolves the probe `error` (a
//! fact for the judge), never a crash.

use std::path::Path;

use arena_core::entities::{activity_event, players, probes, sessions, tasks, tests};
use arena_core::evaluation::{ProbeConfig, ProbeMode};
use arena_core::judging::task_commit::resolve_head;
use arena_core::protocol::ZmqEvent;
use chrono::Utc;
use sea_orm::prelude::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::state::GameServerState;

/// One file at a commit, from `git ls-tree -r -l`.
#[derive(Debug, Clone)]
pub struct TreeFile {
    pub path: String,
    pub size: u64,
}

/// List blobs under `prefix` at `sha` in the bare repo.
pub async fn list_tree(repo_dir: &Path, sha: &str, prefix: &str) -> Vec<TreeFile> {
    let repo_dir = repo_dir.to_path_buf();
    let sha = sha.to_string();
    let prefix = prefix.to_string();
    tokio::task::spawn_blocking(move || {
        let Ok(git_bin) = which::which("git") else {
            return Vec::new();
        };
        let Ok(out) = std::process::Command::new(git_bin)
            .arg("-C")
            .arg(&repo_dir)
            .arg("ls-tree")
            .arg("-r")
            .arg("-l")
            .arg(&sha)
            .arg("--")
            .arg(&prefix)
            .output()
        else {
            return Vec::new();
        };
        if !out.status.success() {
            return Vec::new();
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                // "<mode> blob <sha> <size>\t<path>"
                let (meta, path) = line.split_once('\t')?;
                let mut parts = meta.split_whitespace();
                let _mode = parts.next()?;
                let kind = parts.next()?;
                if kind != "blob" {
                    return None;
                }
                let _sha = parts.next()?;
                let size: u64 = parts.next()?.parse().ok()?;
                Some(TreeFile {
                    path: path.to_string(),
                    size,
                })
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// Attach arrived artifacts to their probes.
///
/// The availability-check probe passes on stdout the moment the file exists
/// in the worktree; the file itself travels only by git (ololo commits and
/// pushes it). This sweep closes the gap: for interactive probes that
/// passed (or are still open) but carry no `artifact_path`, it looks up the
/// pushed tree under the request's folder and records the blob reference —
/// which is what the vision loader and the gallery read.
pub async fn resolve_pending_artifacts(state: &GameServerState) -> Result<(), sea_orm::DbErr> {
    use sea_orm::PaginatorTrait;

    let pending = probes::Entity::find()
        .filter(probes::Column::ArtifactPath.is_null())
        .all(&state.db)
        .await?;
    if pending.is_empty() {
        return Ok(());
    }
    let Some(repos_base) = arena_core::git_store::repos_base_dir() else {
        return Ok(());
    };

    // One announcement per artifact REQUEST (tests row), not per probe row:
    // retries of the same request create several probe rows, all resolved in
    // one sweep once the file lands, and each still records its blob
    // reference — but only the first puts the artifact on the activity feed.
    let mut announced_tests: std::collections::HashSet<Uuid> = std::collections::HashSet::new();

    for probe in pending {
        let Ok(Some(test)) = tests::Entity::find_by_id(probe.test_id)
            .one(&state.db)
            .await
        else {
            continue;
        };
        let Some(config) = test
            .probe_config
            .as_ref()
            .and_then(|c| ProbeConfig::from_json(c).ok())
        else {
            continue;
        };
        if config.mode != ProbeMode::Interactive {
            continue;
        }
        let Some(artifact) = &config.artifact else {
            continue;
        };
        let prefix = artifact
            .path
            .clone()
            .unwrap_or_else(|| format!(".ololo/artifacts/{}/", probe.id));

        let repo_dir =
            arena_core::git_store::player_repo_path(&repos_base, probe.session_id, probe.player_id);
        let Ok(Some((sha, _))) = resolve_head(&repo_dir).await else {
            continue;
        };
        let mut listed = list_tree(&repo_dir, &sha, &prefix).await;
        if listed.is_empty() {
            continue; // not arrived yet — the deadline sweep owns the timeout
        }
        // Deterministic order, bounded count: a request delivers up to
        // MAX_ARTIFACT_FILES files; extras are noted, never silently eaten.
        listed.sort_by(|a, b| a.path.cmp(&b.path));
        let dropped = listed
            .len()
            .saturating_sub(arena_core::evaluation::MAX_ARTIFACT_FILES);
        listed.truncate(arena_core::evaluation::MAX_ARTIFACT_FILES);
        let files = listed;
        let file = &files[0];

        // The delivered file decides the media type — a screencast requested
        // as video/webm legitimately arrives as a .gif; the declared type is
        // only the fallback for unrecognized extensions.
        let content_type = arena_core::evaluation::artifact_content_type_for_path(&file.path)
            .map(str::to_string)
            .unwrap_or_else(|| artifact.content_type.clone());
        // `max_bytes` is a per-file cap.
        let within_cap = files.iter().all(|f| f.size <= artifact.max_bytes);
        let note = if within_cap {
            "artifact received"
        } else {
            "artifact exceeds the requested size cap"
        };
        let now = Utc::now();
        let mut result_json = probe.result_json.clone().unwrap_or(serde_json::json!({}));
        result_json["artifact"] = serde_json::json!({
            "path": file.path,
            "size": file.size,
            "commit": sha,
            "content_type": content_type,
            "within_cap": within_cap,
            "note": note,
            "files": files
                .iter()
                .map(|f| serde_json::json!({ "path": f.path, "size": f.size }))
                .collect::<Vec<_>>(),
            "dropped": dropped,
        });
        let mut update = probes::Entity::update_many()
            .col_expr(probes::Column::UpdatedAt, Expr::value(now))
            .col_expr(
                probes::Column::ArtifactPath,
                Expr::value(format!("{sha}:{}", file.path)),
            )
            .col_expr(probes::Column::ResultJson, Expr::value(result_json));
        // A probe still unresolved (e.g. registered before the check ever
        // ran) resolves by arrival; a graded probe keeps its grade.
        if probe.outcome.is_none() {
            let outcome = if within_cap { "pass" } else { "error" };
            update = update
                .col_expr(probes::Column::Outcome, Expr::value(outcome))
                .col_expr(probes::Column::ResolvedAt, Expr::value(now))
                .col_expr(probes::Column::PointDelta, Expr::value(0))
                .col_expr(
                    probes::Column::Output,
                    Expr::value(format!(
                        "delivered: {} file(s), {} bytes total",
                        files.len(),
                        files.iter().map(|f| f.size).sum::<u64>()
                    )),
                );
        }
        let _ = update
            .filter(probes::Column::Id.eq(probe.id))
            .exec(&state.db)
            .await;
        tracing::info!(
            probe_id = %probe.id, path = %file.path, count = files.len(),
            "interactive artifact blob(s) recorded from repo"
        );
        // Announced before (an earlier sweep resolved a sibling row), or a
        // sibling in THIS sweep already spoke for the request — skip.
        let previously_recorded = probes::Entity::find()
            .filter(probes::Column::TestId.eq(probe.test_id))
            .filter(probes::Column::Id.ne(probe.id))
            .filter(probes::Column::ArtifactPath.is_not_null())
            .count(&state.db)
            .await
            .unwrap_or(0)
            > 0;
        if !previously_recorded && announced_tests.insert(probe.test_id) {
            announce_artifact(state, &probe, &test, &files, &content_type, within_cap).await;
        }
    }
    Ok(())
}

/// Put the delivered artifact on the session activity feed: persist the
/// `activity_event` row, then publish the ZMQ event the server bridges to
/// dashboard clients. Best-effort — a lookup miss only costs the feed entry,
/// never the artifact itself (already recorded on the probe).
async fn announce_artifact(
    state: &GameServerState,
    probe: &probes::Model,
    test: &tests::Model,
    files: &[TreeFile],
    content_type: &str,
    within_cap: bool,
) {
    let Some(file) = files.first() else { return };
    let Ok(Some(task_row)) = tasks::Entity::find_by_id(test.task_id).one(&state.db).await else {
        return;
    };
    let Ok(Some(player)) = players::Entity::find_by_id(probe.player_id)
        .one(&state.db)
        .await
    else {
        return;
    };
    let Ok(Some(session)) = sessions::Entity::find_by_id(probe.session_id)
        .one(&state.db)
        .await
    else {
        return;
    };
    let join_code = session.join_code;
    let version = state
        .session_registry
        .get(&join_code)
        .and_then(|e| e.cache.read().ok().map(|c| c.version))
        .unwrap_or(0);
    let now = Utc::now();

    let detail = serde_json::json!({
        "probe_id": probe.id,
        "path": file.path,
        "size": file.size,
        "content_type": content_type,
        "within_cap": within_cap,
        "files": files
            .iter()
            .map(|f| serde_json::json!({ "path": f.path, "size": f.size }))
            .collect::<Vec<_>>(),
    });
    // Persist-first, like every other activity event: replay for finished
    // sessions reads the row, live dashboards get the broadcast.
    let row = activity_event::ActiveModel {
        id: sea_orm::ActiveValue::Set(Uuid::new_v4()),
        session_id_fk: sea_orm::ActiveValue::Set(probe.session_id),
        player_id_fk: sea_orm::ActiveValue::Set(probe.player_id),
        task_id_fk: sea_orm::ActiveValue::Set(test.task_id),
        event_kind: sea_orm::ActiveValue::Set("artifact_received".to_string()),
        task_ordinal: sea_orm::ActiveValue::Set(task_row.ordinal),
        task_title: sea_orm::ActiveValue::Set(task_row.title.clone()),
        player_display_name: sea_orm::ActiveValue::Set(player.display_name.clone()),
        judge_name: sea_orm::ActiveValue::Set(None),
        point_delta: sea_orm::ActiveValue::Set(None),
        detail: sea_orm::ActiveValue::Set(Some(detail)),
        timestamp: sea_orm::ActiveValue::Set(now),
        version: sea_orm::ActiveValue::Set(version as i64),
    };
    if let Err(e) = activity_event::Entity::insert(row).exec(&state.db).await {
        tracing::warn!(error = %e, "artifact activity_event insert failed, skipping publish");
        return;
    }

    let event = ZmqEvent::ArtifactReceived {
        join_code,
        player_id: probe.player_id,
        player_display_name: player.display_name,
        task_id: test.task_id,
        task_ordinal: task_row.ordinal,
        task_title: task_row.title,
        probe_id: probe.id,
        path: file.path.clone(),
        size: file.size,
        content_type: content_type.to_string(),
        within_cap,
        files: files
            .iter()
            .map(|f| arena_core::protocol::ArtifactFile {
                path: f.path.clone(),
                size: f.size,
            })
            .collect(),
        timestamp: now,
        version,
    };
    state.event_publisher.publish(&event).await;
}

/// One frame sampled from a screencast: when it happens and its JPEG bytes.
#[derive(Debug)]
pub struct VideoFrame {
    /// Seconds from the start of the clip.
    pub at_secs: f64,
    pub jpeg: Vec<u8>,
}

/// Sensitivity of the key-frame pass: the fraction of the picture that must
/// change between frames to count as a new scene. UI transitions (view
/// switches, navigations) score well above this; cursor twitches do not.
const SCENE_CHANGE_THRESHOLD: f64 = 0.1;

/// Sample frames from a delivered screencast so a vision judge can see the
/// flow it demonstrates — LLM providers take images, not video.
///
/// Key-frame selection, not uniform sampling: the first frame plus every
/// frame whose inter-frame difference crosses [`SCENE_CHANGE_THRESHOLD`]
/// (ffmpeg's `select=gt(scene,t)` — frame differencing), so each distinct
/// screen the flow reaches is captured. A near-static clip that yields
/// almost nothing falls back to one frame every two seconds. Frames carry
/// their timestamps so the judge sees the order of events. Best-effort: no
/// ffmpeg on the host (or a corrupt file) returns no frames, and the judge
/// still knows the screencast was delivered.
pub async fn extract_video_frames(bytes: &[u8], max_frames: usize) -> Vec<VideoFrame> {
    if max_frames == 0 {
        return Vec::new();
    }
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || {
        let Ok(ffmpeg) = which::which("ffmpeg") else {
            tracing::debug!("ffmpeg not installed; screencast frames skipped");
            return Vec::new();
        };
        let Ok(dir) = tempfile::tempdir() else {
            return Vec::new();
        };
        let input = dir.path().join("in.bin");
        if std::fs::write(&input, &bytes).is_err() {
            return Vec::new();
        }

        let keyframe_filter =
            format!("select='eq(n,0)+gt(scene,{SCENE_CHANGE_THRESHOLD})',showinfo");
        let frames = run_frame_pass(&ffmpeg, dir.path(), &input, &keyframe_filter, max_frames);
        if frames.len() >= 2 {
            return frames;
        }
        // Nearly nothing changed scene-wise — sample the timeline instead so
        // the judge still sees more than the opening screen.
        run_frame_pass(&ffmpeg, dir.path(), &input, "fps=1/2,showinfo", max_frames)
    })
    .await
    .unwrap_or_default()
}

/// One ffmpeg extraction pass: apply `filter`, keep up to `max_frames`
/// JPEGs, and pair each with the source timestamp parsed from `showinfo`.
fn run_frame_pass(
    ffmpeg: &std::path::Path,
    dir: &Path,
    input: &std::path::Path,
    filter: &str,
    max_frames: usize,
) -> Vec<VideoFrame> {
    let pattern = dir.join("frame%02d.jpg");
    let out = std::process::Command::new(ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("info") // showinfo reports on stderr at info level
        .arg("-i")
        .arg(input)
        .arg("-vf")
        .arg(filter)
        // `-fps_mode` (ffmpeg 5.1+) replaced `-vsync`, which ffmpeg 8
        // REMOVED — with `-vsync` the pass errors out on modern ffmpeg and
        // every screencast silently yields zero frames for the judges.
        .arg("-fps_mode")
        .arg("vfr")
        .arg("-frames:v")
        .arg(max_frames.to_string())
        .arg("-q:v")
        .arg("5")
        .arg("-y")
        .arg(&pattern)
        .output();
    let stderr = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stderr).into_owned(),
        Ok(o) => {
            tracing::warn!(
                stderr = %String::from_utf8_lossy(&o.stderr).trim_end(),
                "ffmpeg failed on screencast; frames skipped"
            );
            return Vec::new();
        }
        Err(e) => {
            tracing::warn!(error = %e, "ffmpeg spawn failed; frames skipped");
            return Vec::new();
        }
    };
    // showinfo lines carry `pts_time:<secs>` per selected frame, in order.
    let mut times = stderr
        .lines()
        .filter(|l| l.contains("Parsed_showinfo"))
        .filter_map(|l| {
            let idx = l.find("pts_time:")?;
            l[idx + "pts_time:".len()..]
                .split_whitespace()
                .next()?
                .parse::<f64>()
                .ok()
        });

    let mut frames = Vec::new();
    for i in 1..=max_frames {
        let path = dir.join(format!("frame{i:02}.jpg"));
        match std::fs::read(&path) {
            Ok(jpeg) if !jpeg.is_empty() => {
                let at_secs = times.next().unwrap_or((i as f64 - 1.0) * 2.0);
                frames.push(VideoFrame { at_secs, jpeg });
                let _ = std::fs::remove_file(&path);
            }
            _ => break,
        }
    }
    frames
}

/// Read an artifact blob (`"<sha>:<path>"` reference) out of a player repo.
/// Capped by the caller's knowledge of `max_bytes`; this trusts the stored
/// reference, which the server itself wrote.
pub async fn read_artifact_blob(repo_dir: &Path, reference: &str) -> Option<Vec<u8>> {
    let repo_dir = repo_dir.to_path_buf();
    let reference = reference.to_string();
    tokio::task::spawn_blocking(move || {
        let git_bin = which::which("git").ok()?;
        let out = std::process::Command::new(git_bin)
            .arg("-C")
            .arg(&repo_dir)
            .arg("show")
            .arg(&reference)
            .output()
            .ok()?;
        out.status.success().then_some(out.stdout)
    })
    .await
    .ok()
    .flatten()
}

/// The probe id an artifact path belongs to, for authz checks on the read
/// endpoint (`.ololo/artifacts/<probe_id>/...`).
pub fn probe_id_from_artifact_path(path: &str) -> Option<Uuid> {
    path.strip_prefix(".ololo/artifacts/")?
        .split('/')
        .next()?
        .parse()
        .ok()
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    #[tokio::test]
    async fn screencast_frames_are_sampled_when_ffmpeg_exists() {
        // The extractor must degrade to "no frames" without ffmpeg — that
        // path needs no fixture. With ffmpeg present, a synthesized two-tone
        // clip must yield decodable JPEG frames.
        let Ok(ffmpeg) = which::which("ffmpeg") else {
            assert!(extract_video_frames(b"not a video", 3).await.is_empty());
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("clip.webm");
        let ok = std::process::Command::new(&ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
            .arg("testsrc=duration=5:size=160x120:rate=5")
            .arg(clip.to_str().unwrap())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok, "ffmpeg could not synthesize the test clip");
        let bytes = std::fs::read(&clip).unwrap();

        let frames = extract_video_frames(&bytes, 3).await;
        assert!(
            (1..=3).contains(&frames.len()),
            "expected 1..=3 frames, got {}",
            frames.len()
        );
        let mut last = -1.0f64;
        for f in &frames {
            assert_eq!(&f.jpeg[..2], b"\xff\xd8", "frames are JPEG");
            assert!(f.at_secs >= last, "timestamps are ordered");
            last = f.at_secs;
        }

        // Garbage in, no frames out — never an error.
        assert!(extract_video_frames(b"not a video", 3).await.is_empty());
    }
}
