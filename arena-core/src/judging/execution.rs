//! Execution-judge core: materialize a player's committed solution into a
//! scratch working tree, and map a probe pass-count onto a rating scale.
//!
//! The bare per-player repo has no working tree, so an execution judge must
//! extract the task's snapshot commit before it can run the solution. The
//! actual sandboxed execution + grading lives in the game-server (it reuses
//! the live probe machinery); this module holds the two pure/IO pieces that
//! belong in `arena-core`.

use std::path::Path;

use crate::validation::judge_results::RatingScale;

use super::JudgeError;

/// Extract `sha` from the bare repo at `repo_dir` into `dest` (a fresh empty
/// directory) as a real working tree, via `git archive <sha> | tar -x`.
///
/// Uses `git archive` rather than a checkout so the bare repo's refs/HEAD are
/// never touched and no lock is taken — many judge runs can materialize
/// different commits from the same repo concurrently.
pub async fn materialize_commit(repo_dir: &Path, sha: &str, dest: &Path) -> Result<(), JudgeError> {
    let repo_dir = repo_dir.to_path_buf();
    let dest = dest.to_path_buf();
    let sha = sha.to_string();
    tokio::task::spawn_blocking(move || {
        let git = which::which("git").map_err(|e| JudgeError::GitReadError(e.to_string()))?;
        let tar = which::which("tar").map_err(|e| JudgeError::GitReadError(e.to_string()))?;

        // git archive <sha>  →  tar -x -C dest
        let mut archive = std::process::Command::new(&git)
            .arg("-C")
            .arg(&repo_dir)
            .arg("archive")
            .arg("--format=tar")
            .arg(&sha)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| JudgeError::GitReadError(format!("git archive spawn: {e}")))?;

        let archive_out = archive.stdout.take().expect("archive stdout piped");
        let tar_status = std::process::Command::new(&tar)
            .arg("-x")
            .arg("-C")
            .arg(&dest)
            .stdin(archive_out)
            .status()
            .map_err(|e| JudgeError::GitReadError(format!("tar extract: {e}")))?;

        let archive_status = archive
            .wait()
            .map_err(|e| JudgeError::GitReadError(format!("git archive wait: {e}")))?;

        if !archive_status.success() {
            // A missing/invalid commit or an empty repo lands here.
            return Err(JudgeError::PlayerRepoEmpty);
        }
        if !tar_status.success() {
            return Err(JudgeError::GitReadError("tar extract failed".to_string()));
        }
        Ok(())
    })
    .await
    .map_err(|e| JudgeError::GitReadError(format!("spawn_blocking join: {e}")))?
}

/// Map `passed` of `total` probes onto `scale`, linearly from `scale.min` (all
/// failed) to `scale.max` (all passed), quantized to `scale.step`.
///
/// Works for both reward scales (`[0, 100]` → pays proportional to probes
/// passed) and penalty scales (`[-500, 0]` → penalizes proportional to probes
/// failed). Returns `(rating, point_delta)` where `point_delta = round(rating)`
/// clamped into the scale. `total == 0` yields the failing end (nothing was
/// verified).
pub fn aggregate_rating(passed: usize, total: usize, scale: &RatingScale) -> (f64, i32) {
    let fraction = if total == 0 {
        0.0
    } else {
        (passed as f64 / total as f64).clamp(0.0, 1.0)
    };
    let raw = scale.min + fraction * (scale.max - scale.min);

    // Quantize to the nearest step from min, then clamp back into range.
    let steps = ((raw - scale.min) / scale.step).round();
    let quantized = (scale.min + steps * scale.step).clamp(scale.min, scale.max);

    let point_delta = quantized.round() as i32;
    (quantized, point_delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scale(min: f64, max: f64, step: f64) -> RatingScale {
        RatingScale { min, max, step }
    }

    #[test]
    fn reward_scale_pays_proportionally() {
        let s = scale(0.0, 100.0, 1.0);
        assert_eq!(aggregate_rating(0, 4, &s), (0.0, 0));
        assert_eq!(aggregate_rating(4, 4, &s), (100.0, 100));
        assert_eq!(aggregate_rating(2, 4, &s), (50.0, 50));
        assert_eq!(aggregate_rating(3, 4, &s), (75.0, 75));
    }

    #[test]
    fn penalty_scale_penalizes_failures() {
        // [-500, 0]: all pass → 0 (no penalty); all fail → -500.
        let s = scale(-500.0, 0.0, 1.0);
        assert_eq!(aggregate_rating(4, 4, &s), (0.0, 0));
        assert_eq!(aggregate_rating(0, 4, &s), (-500.0, -500));
        assert_eq!(aggregate_rating(3, 4, &s), (-125.0, -125));
    }

    #[test]
    fn zero_total_is_the_failing_end() {
        assert_eq!(aggregate_rating(0, 0, &scale(0.0, 10.0, 1.0)), (0.0, 0));
        assert_eq!(
            aggregate_rating(0, 0, &scale(-10.0, 0.0, 1.0)),
            (-10.0, -10)
        );
    }

    #[test]
    fn respects_step_quantization() {
        // step 5 on [0, 20]: 1/3 passed → raw 6.67 → nearest step 5.
        let s = scale(0.0, 20.0, 5.0);
        let (rating, _) = aggregate_rating(1, 3, &s);
        assert_eq!(rating, 5.0);
    }
}
