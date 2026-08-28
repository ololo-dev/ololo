//! `/api/projects/:project_id/tasks/*` HTTP handlers.
//!
//! See `common` for shared request/response shapes, [`ProjectTaskError`] and
//! helpers. Public route handlers are re-exported here for `lib.rs` wiring.

pub use common::{CreateTaskReq, PatchTaskReq, ProjectTaskError};
pub use read::{get_list, get_one, get_preview};
pub use write::{ReorderReq, delete_one, patch_one, post_create, reorder};

pub(crate) use common::validate_ordinal;

pub(crate) mod common;
pub(crate) mod read;
pub(crate) mod write;
