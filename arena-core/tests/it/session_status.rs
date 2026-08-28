use std::str::FromStr;

use sea_orm::sea_query::{Nullable, ValueType};

use arena_core::session_status::*;

#[test]
fn serde_roundtrip_all_variants() {
    for status in [
        SessionStatus::Lobby,
        SessionStatus::Running,
        SessionStatus::Paused,
        SessionStatus::Finished,
        SessionStatus::Cancelled,
    ] {
        let json = serde_json::to_string(&status).unwrap();
        let back: SessionStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, back, "roundtrip failed for {status:?}: {json}");
    }
}

#[test]
fn serde_renders_snake_case() {
    assert_eq!(
        serde_json::to_string(&SessionStatus::Lobby).unwrap(),
        "\"lobby\""
    );
    assert_eq!(
        serde_json::to_string(&SessionStatus::Running).unwrap(),
        "\"running\""
    );
    assert_eq!(
        serde_json::to_string(&SessionStatus::Paused).unwrap(),
        "\"paused\""
    );
    assert_eq!(
        serde_json::to_string(&SessionStatus::Finished).unwrap(),
        "\"finished\""
    );
    assert_eq!(
        serde_json::to_string(&SessionStatus::Cancelled).unwrap(),
        "\"cancelled\""
    );
}

#[test]
fn serde_rejects_unknown_variant() {
    assert!(serde_json::from_str::<SessionStatus>("\"nope\"").is_err());
}

#[test]
fn display_writes_snake_case() {
    assert_eq!(SessionStatus::Lobby.to_string(), "lobby");
    assert_eq!(SessionStatus::Running.to_string(), "running");
    assert_eq!(SessionStatus::Paused.to_string(), "paused");
    assert_eq!(SessionStatus::Finished.to_string(), "finished");
    assert_eq!(SessionStatus::Cancelled.to_string(), "cancelled");
}

#[test]
fn fromstr_parses_valid() {
    assert_eq!(
        SessionStatus::from_str("lobby").unwrap(),
        SessionStatus::Lobby
    );
    assert_eq!(
        SessionStatus::from_str("running").unwrap(),
        SessionStatus::Running
    );
    assert_eq!(
        SessionStatus::from_str("paused").unwrap(),
        SessionStatus::Paused
    );
    assert_eq!(
        SessionStatus::from_str("finished").unwrap(),
        SessionStatus::Finished
    );
    assert_eq!(
        SessionStatus::from_str("cancelled").unwrap(),
        SessionStatus::Cancelled
    );
}

#[test]
fn fromstr_rejects_invalid_and_case_sensitive() {
    assert!(SessionStatus::from_str("nope").is_err());
    assert!(SessionStatus::from_str("").is_err());
    assert!(
        SessionStatus::from_str("Lobby").is_err(),
        "must be case-sensitive"
    );
    assert!(SessionStatus::from_str("LOBBY").is_err());
    assert!(SessionStatus::from_str(" paused ").is_err(), "no trimming");
}

#[test]
fn value_type_column_type_is_string() {
    let col = SessionStatus::column_type();
    match col {
        sea_orm::sea_query::ColumnType::String(_) => {}
        other => panic!("expected ColumnType::String, got {other:?}"),
    }
}

#[test]
fn value_type_try_from_string_value() {
    let v = sea_orm::sea_query::Value::from(SessionStatus::Paused);
    let back = <SessionStatus as sea_orm::sea_query::ValueType>::try_from(v).unwrap();
    assert_eq!(back, SessionStatus::Paused);
}

#[test]
fn value_type_try_from_wrong_value_errors() {
    let v = sea_orm::sea_query::Value::Int(Some(7));
    assert!(<SessionStatus as sea_orm::sea_query::ValueType>::try_from(v).is_err());
}

#[test]
fn nullable_null_is_string_none() {
    match SessionStatus::null() {
        sea_orm::sea_query::Value::String(None) => {}
        other => panic!("expected Value::String(None), got {other:?}"),
    }
}
