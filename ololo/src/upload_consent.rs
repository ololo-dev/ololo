//! Before a personal project's first snapshot leaves the machine: what will
//! be uploaded, and the user's yes.
//!
//! A challenge is played in a folder made for it. A personal project is
//! played in the user's own repository — their real code, which the
//! session pushes to the ololo server and the judges' models read. So the
//! first push of such a session waits for a plain answer to a plain
//! question: this many files, this big, from here — go?

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::snapshot::files;

/// The server refuses a push body over 256 MiB; stop well before it.
const MAX_UPLOAD_BYTES: u64 = 200 * 1024 * 1024;

/// How many of the largest files the summary names.
const LARGEST_SHOWN: usize = 3;

/// What a snapshot of the working tree would upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Survey {
    pub files: usize,
    pub bytes: u64,
    /// The biggest files, largest first.
    pub largest: Vec<(PathBuf, u64)>,
    /// Secret-looking files left out.
    pub secrets: usize,
    pub from_git: bool,
}

pub fn survey(worktree: &Path) -> std::io::Result<Survey> {
    let listing = files::list(worktree)?;
    let mut sized: Vec<(PathBuf, u64)> = listing
        .files
        .into_iter()
        .filter_map(|rel| {
            let meta = std::fs::metadata(worktree.join(&rel)).ok()?;
            meta.is_file().then_some((rel, meta.len()))
        })
        .collect();
    let bytes = sized.iter().map(|(_, len)| len).sum();
    let files = sized.len();
    sized.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    sized.truncate(LARGEST_SHOWN);
    Ok(Survey {
        files,
        bytes,
        largest: sized,
        secrets: listing.secrets.len(),
        from_git: listing.from_git,
    })
}

fn human(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= MIB {
        format!("{:.1} MB", bytes as f64 / MIB)
    } else {
        format!("{:.0} KB", (bytes as f64 / 1024.0).ceil())
    }
}

/// The lines the user reads before answering.
pub fn summary(survey: &Survey, server: &str) -> Vec<String> {
    let mut lines = vec![
        format!(
            "This is a personal project: ololo uploads this folder to {server} so the judges \
             can review each task."
        ),
        format!(
            "  {} files, {} ({})",
            survey.files,
            human(survey.bytes),
            if survey.from_git {
                "what git tracks or would add"
            } else {
                "the folder, minus its .gitignore"
            }
        ),
    ];
    if survey.secrets > 0 {
        lines.push(format!(
            "  {} secret-looking file(s) stay here (.env, keys, credentials)",
            survey.secrets
        ));
    }
    if !survey.largest.is_empty() {
        let names: Vec<String> = survey
            .largest
            .iter()
            .map(|(p, len)| format!("{} ({})", p.display(), human(*len)))
            .collect();
        lines.push(format!("  largest: {}", names.join(", ")));
    }
    lines.push(format!(
        "  leave paths out with {} (gitignore syntax)",
        files::OLOLO_IGNORE
    ));
    lines
}

/// Show what will be uploaded and get a yes: `--yes`, or an answer on the
/// terminal. Refuses a tree the server would refuse anyway, and refuses to
/// guess when nobody is there to ask.
pub fn confirm(worktree: &Path, server: &str, yes: bool) -> Result<()> {
    let survey = survey(worktree)?;
    for line in summary(&survey, server) {
        println!("{line}");
    }
    if survey.bytes > MAX_UPLOAD_BYTES {
        bail!(
            "this folder is too large to upload ({} > {}): list big or generated paths in {}",
            human(survey.bytes),
            human(MAX_UPLOAD_BYTES),
            files::OLOLO_IGNORE
        );
    }
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        bail!("no terminal to ask on: re-run with --yes to confirm the upload");
    }
    print!("Upload and start? [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        bail!("cancelled — nothing was uploaded");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_survey_counts_what_the_snapshot_would_hold() {
        let dir = tempfile::tempdir().unwrap();
        let w = dir.path();
        std::fs::write(w.join("small.txt"), "a").unwrap();
        std::fs::write(w.join("big.bin"), vec![0u8; 4096]).unwrap();
        std::fs::write(w.join(".env"), "TOKEN=x").unwrap();
        std::fs::create_dir_all(w.join("node_modules/x")).unwrap();
        std::fs::write(w.join("node_modules/x/i.js"), "dep").unwrap();

        let s = survey(w).unwrap();
        assert_eq!(s.files, 2);
        assert_eq!(s.bytes, 4097);
        assert_eq!(s.secrets, 1);
        assert_eq!(s.largest[0].0, PathBuf::from("big.bin"));

        let text = summary(&s, "https://ololo.dev").join("\n");
        assert!(text.contains("2 files"), "{text}");
        assert!(
            text.contains("1 secret-looking file(s) stay here"),
            "{text}"
        );
        assert!(text.contains(".ololoignore"), "{text}");
    }

    #[test]
    fn yes_confirms_without_asking() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn main() {}").unwrap();
        confirm(dir.path(), "https://ololo.dev", true).expect("--yes confirms");
    }
}
