//! A project's git repository — the code its sessions start from.
//!
//! Any project may name one: a challenge its starter codebase, a personal
//! project its owner's own repository. Before a session starts in a folder
//! that does not hold it yet, ololo clones it there, so every player — the
//! owner on another machine, a teammate who joined by code, a stranger on a
//! public challenge — begins from the same code.
//!
//! The URL is shown to players and handed to `git clone` on their
//! machines, so only what is safe to clone passes [`validate_url`]: https
//! and ssh remotes (the `git@host:path` shorthand included), no credentials
//! in the URL, and nothing git could read as an option, a local path or a
//! transport that runs a command.
//!
//! A session of a project that starts from existing code — a personal
//! project, or any project with a repository — is scored as work on that
//! code: health against where each task started, and no plagiarism check
//! between players who all began from the same files
//! ([`project_starts_from_existing_code`]).

use std::collections::HashMap;

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entities::{project_repos, sessions};

/// Longest URL a project may name.
pub const MAX_URL_CHARS: usize = 500;
/// Longest branch, tag or commit name.
pub const MAX_REF_CHARS: usize = 200;

/// The repository a project names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRepo {
    pub url: String,
    /// Branch, tag or commit to check out; `None` = the default branch.
    pub git_ref: Option<String>,
}

impl ProjectRepo {
    /// A validated repository from a URL and an optional ref, each as the
    /// user typed it (blank ref = none).
    pub fn parse(url: &str, git_ref: Option<&str>) -> Result<Self, String> {
        let url = validate_url(url)?;
        let git_ref = match git_ref.map(str::trim).filter(|r| !r.is_empty()) {
            Some(r) => Some(validate_ref(r)?),
            None => None,
        };
        Ok(Self { url, git_ref })
    }
}

/// The repository a request names — `None` when its URL is blank.
pub fn requested(url: Option<&str>, git_ref: Option<&str>) -> Result<Option<ProjectRepo>, String> {
    match url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(url) => ProjectRepo::parse(url, git_ref).map(Some),
        None if git_ref.map(str::trim).is_some_and(|r| !r.is_empty()) => {
            Err("a branch, tag or commit needs a repository URL".into())
        }
        None => Ok(None),
    }
}

/// What an edit asks of a project's repository, `current` being the one it
/// names now: `None` leaves it be, `Some(None)` drops it, `Some(Some(_))`
/// names one. A URL sets the repository whole — ref included, absent =
/// default branch; a ref alone moves the current repository's checkout.
pub fn patched(
    current: Option<&ProjectRepo>,
    url: Option<&str>,
    git_ref: Option<&str>,
) -> Result<Option<Option<ProjectRepo>>, String> {
    match (url, git_ref) {
        (None, None) => Ok(None),
        (Some(url), git_ref) => requested(Some(url), git_ref).map(Some),
        (None, Some(git_ref)) => {
            let current = current.ok_or("a branch, tag or commit needs a repository URL")?;
            ProjectRepo::parse(&current.url, Some(git_ref)).map(|repo| Some(Some(repo)))
        }
    }
}

fn is_host_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '.' || c == '-'
}

fn is_user_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

fn is_path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '~' | '/' | '%' | '+')
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.chars().all(is_host_char)
        && !host.starts_with(['-', '.'])
        && !host.ends_with('.')
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.chars().all(is_path_char)
        && !path.starts_with('-')
        && !path.trim_matches('/').is_empty()
        && !path.split('/').any(|seg| seg == "..")
}

/// A repository URL ololo may clone, trimmed, or why not.
///
/// Accepted: `https://host[:port]/path`, `ssh://[user@]host[:port]/path`
/// and `user@host:path`. Refused: every other scheme (`http`, `file`,
/// `git`, `ext::…`), local paths, credentials in the URL — a token there
/// would be shown to every player — and anything that starts like an
/// option.
pub fn validate_url(raw: &str) -> Result<String, String> {
    const SHAPE: &str = "use an https:// or ssh:// URL, or git@host:owner/repo";
    let url = raw.trim();
    if url.is_empty() {
        return Err("a repository URL is required".into());
    }
    if url.chars().count() > MAX_URL_CHARS {
        return Err(format!(
            "a repository URL has at most {MAX_URL_CHARS} characters"
        ));
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("a repository URL has no spaces".into());
    }
    if url.starts_with('-') {
        return Err(SHAPE.into());
    }
    if let Some((scheme, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/').ok_or_else(|| SHAPE.to_string())?;
        let (user, host_port) = match authority.rsplit_once('@') {
            Some((user, host_port)) => (Some(user), host_port),
            None => (None, authority),
        };
        match scheme {
            "https" => {
                if user.is_some() {
                    return Err(
                        "leave credentials out of the URL — every player sees it; your git \
                         credential helper supplies them"
                            .into(),
                    );
                }
            }
            "ssh" => {
                if let Some(user) = user
                    && (user.is_empty() || !user.chars().all(is_user_char))
                {
                    return Err(
                        "an ssh URL names a user and no password, like ssh://git@host/repo".into(),
                    );
                }
            }
            _ => return Err(SHAPE.into()),
        }
        let (host, port) = match host_port.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_port, None),
        };
        if !valid_host(host) {
            return Err(SHAPE.into());
        }
        if let Some(port) = port
            && (port.is_empty() || port.len() > 5 || !port.chars().all(|c| c.is_ascii_digit()))
        {
            return Err(SHAPE.into());
        }
        if !valid_path(path) {
            return Err(SHAPE.into());
        }
        return Ok(url.to_string());
    }
    // The scp-like shorthand git reads as ssh: user@host:path.
    let (user, rest) = url.split_once('@').ok_or_else(|| SHAPE.to_string())?;
    let (host, path) = rest.split_once(':').ok_or_else(|| SHAPE.to_string())?;
    if user.is_empty() || !user.chars().all(is_user_char) || !valid_host(host) || !valid_path(path)
    {
        return Err(SHAPE.into());
    }
    Ok(url.to_string())
}

/// A branch, tag or commit name ololo may check out, trimmed, or why not.
/// The rules are git's own for ref names, narrowed to what never needs
/// quoting: letters, digits, `.`, `_`, `-` and `/`.
pub fn validate_ref(raw: &str) -> Result<String, String> {
    let git_ref = raw.trim();
    let bad = || "a branch, tag or commit is letters, digits, . _ - and /".to_string();
    if git_ref.is_empty() || git_ref.chars().count() > MAX_REF_CHARS {
        return Err(format!(
            "a branch, tag or commit has 1 to {MAX_REF_CHARS} characters"
        ));
    }
    if !git_ref
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
    {
        return Err(bad());
    }
    if git_ref.starts_with(['-', '/', '.'])
        || git_ref.ends_with(['/', '.'])
        || git_ref.ends_with(".lock")
        || git_ref.contains("..")
        || git_ref.contains("//")
        || git_ref.contains("/.")
    {
        return Err(bad());
    }
    Ok(git_ref.to_string())
}

/// The repository project `project_id` names, if any.
pub async fn repo_of<C: ConnectionTrait>(
    db: &C,
    project_id: Uuid,
) -> Result<Option<ProjectRepo>, DbErr> {
    Ok(project_repos::Entity::find_by_id(project_id)
        .one(db)
        .await?
        .map(|row| ProjectRepo {
            url: row.url,
            git_ref: row.git_ref,
        }))
}

/// The repositories of those of `project_ids` that name one.
pub async fn repos_of<C: ConnectionTrait>(
    db: &C,
    project_ids: &[Uuid],
) -> Result<HashMap<Uuid, ProjectRepo>, DbErr> {
    if project_ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(project_repos::Entity::find()
        .filter(project_repos::Column::ProjectIdFk.is_in(project_ids.iter().copied()))
        .all(db)
        .await?
        .into_iter()
        .map(|row| {
            (
                row.project_id_fk,
                ProjectRepo {
                    url: row.url,
                    git_ref: row.git_ref,
                },
            )
        })
        .collect())
}

/// Make project `project_id` name `repo` — or none.
pub async fn set_repo<C: ConnectionTrait>(
    db: &C,
    project_id: Uuid,
    repo: Option<&ProjectRepo>,
) -> Result<(), DbErr> {
    let existing = project_repos::Entity::find_by_id(project_id)
        .one(db)
        .await?;
    let now = Utc::now();
    match (existing, repo) {
        (None, None) => {}
        (Some(row), None) => {
            project_repos::Entity::delete_by_id(row.project_id_fk)
                .exec(db)
                .await?;
        }
        (None, Some(repo)) => {
            project_repos::ActiveModel {
                project_id_fk: Set(project_id),
                url: Set(repo.url.clone()),
                git_ref: Set(repo.git_ref.clone()),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(db)
            .await?;
        }
        (Some(row), Some(repo)) => {
            if row.url != repo.url || row.git_ref != repo.git_ref {
                let mut am: project_repos::ActiveModel = row.into();
                am.url = Set(repo.url.clone());
                am.git_ref = Set(repo.git_ref.clone());
                am.updated_at = Set(now);
                am.update(db).await?;
            }
        }
    }
    Ok(())
}

/// Whether a session of project `project_id` starts from code that
/// predates it: the owner's repository of a personal project, or the
/// repository the project names.
pub async fn project_starts_from_existing_code<C: ConnectionTrait>(
    db: &C,
    project_id: Uuid,
) -> Result<bool, DbErr> {
    Ok(crate::personal::is_personal_project(db, project_id).await?
        || project_repos::Entity::find_by_id(project_id)
            .one(db)
            .await?
            .is_some())
}

/// [`project_starts_from_existing_code`] for the project session
/// `session_id` plays. A missing session does not.
pub async fn session_starts_from_existing_code<C: ConnectionTrait>(
    db: &C,
    session_id: Uuid,
) -> Result<bool, DbErr> {
    let Some(session) = sessions::Entity::find_by_id(session_id).one(db).await? else {
        return Ok(false);
    };
    project_starts_from_existing_code(db, session.project_id_fk).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remotes_git_can_clone_safely_pass() {
        for url in [
            "https://github.com/ololo-dev/ololo.git",
            "https://gitlab.example.com:8443/team/app",
            "https://example.com/~me/repo.git",
            "ssh://git@github.com/ololo-dev/ololo.git",
            "ssh://git.example.com:2222/srv/app.git",
            "git@github.com:ololo-dev/ololo.git",
            "deploy@git.example.com:/srv/app.git",
        ] {
            assert_eq!(validate_url(url).as_deref(), Ok(url), "{url}");
        }
        assert_eq!(
            validate_url("  https://github.com/a/b  ").as_deref(),
            Ok("https://github.com/a/b")
        );
    }

    #[test]
    fn everything_else_is_refused() {
        for url in [
            "",
            "http://github.com/a/b",
            "file:///etc/passwd",
            "git://github.com/a/b",
            "ext::sh -c touch% /tmp/pwned",
            "ext::sh",
            "/home/me/repo",
            "../repo",
            "--upload-pack=touch /tmp/x",
            "https://user:token@github.com/a/b",
            "https://token@github.com/a/b",
            "ssh://git:secret@github.com/a/b",
            "https://github.com",
            "https://github.com/",
            "https://-evil.com/a",
            "https://github.com/a/../../b",
            "https://github.com/a b",
            "git@github.com",
            "git@:repo",
            "git@github.com:-oProxyCommand=x",
            "https://github.com/a?b=c",
            "https://github.com:port/a",
        ] {
            assert!(validate_url(url).is_err(), "{url:?} should be refused");
        }
        let long = format!("https://github.com/{}", "a".repeat(MAX_URL_CHARS));
        assert!(validate_url(&long).is_err());
    }

    #[test]
    fn refs_are_names_git_takes_without_quoting() {
        for r in [
            "main",
            "release/1.2",
            "v1.0.0",
            "feature_x-2",
            "a1b2c3d",
            "9fceb02d0ae598e95dc970b74767f19372d61af8",
        ] {
            assert_eq!(validate_ref(r).as_deref(), Ok(r), "{r}");
        }
        for r in [
            "",
            "-b",
            "/main",
            ".hidden",
            "main/",
            "main.",
            "main.lock",
            "a..b",
            "a//b",
            "a/.b",
            "main branch",
            "main;rm",
            "HEAD@{1}",
            "a:b",
        ] {
            assert!(validate_ref(r).is_err(), "{r:?} should be refused");
        }
    }

    #[test]
    fn a_request_names_a_repository_or_none() {
        assert_eq!(requested(None, None), Ok(None));
        assert_eq!(requested(Some("  "), None), Ok(None));
        assert!(requested(Some(" "), Some("main")).is_err());
        let repo = requested(Some("git@github.com:a/b.git"), Some("dev"))
            .unwrap()
            .unwrap();
        assert_eq!(repo.url, "git@github.com:a/b.git");
        assert_eq!(repo.git_ref.as_deref(), Some("dev"));
        assert!(requested(Some("file:///x"), None).is_err());
    }

    #[test]
    fn an_edit_leaves_drops_replaces_or_moves_the_repository() {
        let current = ProjectRepo::parse("https://github.com/a/b", Some("main")).unwrap();
        assert_eq!(patched(Some(&current), None, None), Ok(None));
        assert_eq!(patched(Some(&current), Some(""), None), Ok(Some(None)));
        let replaced = patched(Some(&current), Some("https://github.com/c/d"), None)
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(replaced.url, "https://github.com/c/d");
        assert_eq!(
            replaced.git_ref, None,
            "a new URL starts at its default branch"
        );
        let moved = patched(Some(&current), None, Some("v2"))
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(moved.url, current.url);
        assert_eq!(moved.git_ref.as_deref(), Some("v2"));
        let reset = patched(Some(&current), None, Some(""))
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(reset.git_ref, None);
        assert!(patched(None, None, Some("v2")).is_err());
    }

    #[test]
    fn a_blank_ref_is_none() {
        let repo = ProjectRepo::parse("https://github.com/a/b", Some("   ")).unwrap();
        assert_eq!(repo.git_ref, None);
        let repo = ProjectRepo::parse("https://github.com/a/b", Some(" v2 ")).unwrap();
        assert_eq!(repo.git_ref.as_deref(), Some("v2"));
        assert!(ProjectRepo::parse("https://github.com/a/b", Some("-x")).is_err());
    }
}
