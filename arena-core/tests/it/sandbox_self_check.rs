//! `sandbox::self_check` — the guard against a *present but non-functional*
//! backend.
//!
//! This is the failure that shipped to production: `bwrap` was installed in the
//! game-server image, so `detect_backend` picked it, but the container had no
//! permission to create user namespaces. Every `bwrap` invocation exited 1 with
//! empty stdout, and the execution judge — grading on stdout alone — read that
//! as "every probe failed" and charged honest players up to -100 per task.
//!
//! `bwrap_argv` unit tests cannot catch this: the argv was always correct. Only
//! executing something and checking it came back does.
//!
//! Lives in its own integration-test binary because it manipulates `PATH`, which
//! is process-global.

use std::path::Path;

use arena_core::sandbox::{self, SandboxBackend};

/// Write an executable shell script at `path`.
fn write_exe(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod stub");
    }
}

#[tokio::test]
async fn self_check_detects_a_backend_that_cannot_execute() {
    let work = tempfile::tempdir().expect("workdir");

    // A functioning backend round-trips the canary.
    sandbox::self_check(SandboxBackend::Unsandboxed, work.path())
        .await
        .expect("unsandboxed backend executes commands");

    // Stderr is captured, not discarded — without it a failing sandbox is
    // undiagnosable from production.
    let out = sandbox::run(
        SandboxBackend::Unsandboxed,
        "printf out; printf err 1>&2; exit 3",
        work.path(),
        std::time::Duration::from_secs(5),
    )
    .await
    .expect("run");
    assert_eq!(out.stdout, "out");
    assert_eq!(out.stderr, "err", "stderr is captured for diagnosis");
    assert_eq!(out.exit_code, 3);

    // Now the production failure: a `bwrap` that exists on PATH but refuses to
    // run anything, exactly as it behaves without user-namespace permission.
    let fake_bin = tempfile::tempdir().expect("fake bin");
    write_exe(
        &fake_bin.path().join("bwrap"),
        "#!/bin/sh\necho 'bwrap: Creating new namespace failed: Operation not permitted' 1>&2\nexit 1\n",
    );
    let real_path = std::env::var("PATH").unwrap_or_default();
    unsafe {
        std::env::set_var("PATH", format!("{}:{real_path}", fake_bin.path().display()));
    }
    let verdict = sandbox::self_check(SandboxBackend::Bubblewrap, work.path()).await;
    unsafe { std::env::set_var("PATH", &real_path) };

    let why = verdict.expect_err("a bwrap that runs nothing must fail the self-check");
    assert!(
        why.contains("cannot execute commands"),
        "diagnostic names the cause: {why}"
    );
    assert!(
        why.contains("Operation not permitted"),
        "diagnostic carries the backend's own stderr: {why}"
    );
}
