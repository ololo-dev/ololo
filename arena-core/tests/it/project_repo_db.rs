//! A project's repository row: named, moved, dropped — and what it means for
//! how a session of the project is scored.

use arena_core::project_repo::{
    ProjectRepo, project_starts_from_existing_code, repo_of, repos_of,
    session_starts_from_existing_code, set_repo,
};

use crate::common::*;

#[tokio::test]
async fn a_repository_is_named_moved_and_dropped() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let other = insert_project(&db, owner).await;
    assert_eq!(repo_of(&db, project).await.unwrap(), None);

    let starter =
        ProjectRepo::parse("https://github.com/ololo-dev/starter.git", Some("v1")).unwrap();
    set_repo(&db, project, Some(&starter)).await.unwrap();
    assert_eq!(repo_of(&db, project).await.unwrap(), Some(starter.clone()));

    let moved = ProjectRepo::parse("https://github.com/ololo-dev/starter.git", None).unwrap();
    set_repo(&db, project, Some(&moved)).await.unwrap();
    assert_eq!(repo_of(&db, project).await.unwrap(), Some(moved.clone()));

    let all = repos_of(&db, &[project, other]).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all.get(&project), Some(&moved));

    set_repo(&db, project, None).await.unwrap();
    assert_eq!(repo_of(&db, project).await.unwrap(), None);
    // Dropping what is not there is nothing.
    set_repo(&db, other, None).await.unwrap();
}

#[tokio::test]
async fn a_session_of_a_project_with_a_repository_starts_from_existing_code() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let from_scratch = insert_project(&db, owner).await;
    let with_repo = insert_project(&db, owner).await;
    set_repo(
        &db,
        with_repo,
        Some(&ProjectRepo::parse("git@github.com:ololo-dev/starter.git", None).unwrap()),
    )
    .await
    .unwrap();

    assert!(
        !project_starts_from_existing_code(&db, from_scratch)
            .await
            .unwrap()
    );
    assert!(
        project_starts_from_existing_code(&db, with_repo)
            .await
            .unwrap()
    );
    // One session per database: the fixture reuses its join code.
    let repo_session = insert_session(&db, with_repo).await;
    assert!(
        session_starts_from_existing_code(&db, repo_session)
            .await
            .unwrap()
    );
    assert!(
        !session_starts_from_existing_code(&db, uuid::Uuid::new_v4())
            .await
            .unwrap(),
        "a missing session does not"
    );
}
