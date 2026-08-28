//! `sessions` API handlers and shared helpers.
//!
//! Route handlers (consumed by `lib.rs` router assembly):
//! - [`post_create`], [`get_list`], [`get_by_code`], [`get_one`],
//!   [`patch_one`], [`delete_one`], [`get_report`], [`get_project_sessions`],
//!   [`post_regenerate_code`], [`post_join`], [`get_admin_list`]
//!
//! Shared request/response types and [`SessionError`] live in [`common`].

pub use admin::get_admin_list;
pub use common::{CreateSessionReq, JoinSessionReq, PatchSessionReq, SessionError};
pub use create::post_create;
pub use join::post_join;
pub(crate) use read::load_session_activity;
pub use read::{
    delete_one, get_activity, get_by_code, get_campaign_by_code, get_list, get_one,
    get_player_stats, get_project_sessions, get_report, get_session_artifact, patch_one,
};
pub use regenerate::post_regenerate_code;

pub(crate) mod admin;
pub(crate) mod common;
pub(crate) mod create;
pub(crate) mod join;
pub(crate) mod read;
pub(crate) mod regenerate;
