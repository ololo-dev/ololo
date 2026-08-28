use std::time::Instant;

use server::adaptation::command_policy::{normalize_shell_command, split_when_safe};

fn run_sample(iterations: usize) {
    let cases = [
        "test -f ./AGENTS.md && test -f ./README.md",
        "```sh\ncat ./README.md\n```",
        "test -f ./Cargo.toml",
        "cat ./AGENTS.md | grep -q TODO || cat ./README.md",
    ];

    for i in 0..iterations {
        let case = cases[i % cases.len()];
        let _ = normalize_shell_command(case);
        let _ = split_when_safe(case);
    }
}

fn main() {
    let baseline_p95_ms = std::env::var("ARENA_ADAPTATION_BASELINE_P95_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(5);

    let started = Instant::now();
    run_sample(50_000);
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let approx_p95_ms = elapsed_ms / 50;
    let threshold_ms = baseline_p95_ms + ((baseline_p95_ms as f64 * 0.10).ceil() as u64);

    println!(
        "adaptation-bench baseline_p95_ms={} measured_p95_ms={} threshold_ms={}",
        baseline_p95_ms, approx_p95_ms, threshold_ms
    );

    if approx_p95_ms > threshold_ms {
        panic!(
            "adaptation latency regression: measured {}ms > threshold {}ms",
            approx_p95_ms, threshold_ms
        );
    }
}
