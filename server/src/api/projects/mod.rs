//! `projects` API handlers and shared helpers.
//!
//! Route handlers (consumed by `lib.rs`):
//! - [`get_list`], [`post_create`], [`get_by_slug`], [`get_by_user_slug`],
//!   [`get_categories`], [`get_one`], [`patch_one`], [`delete_one`]
//!
//! Shared request/response types and [`ProjectError`] live in [`common`].

pub use common::{
    CreateProjectReq, ListProjectsQuery, PatchProjectReq, ProjectError, ProjectSummary,
};
pub(crate) use common::{load_allowed_categories, validate_slug};
pub use read::{
    get_by_slug, get_by_user_slug, get_categories, get_judges, get_list, get_one, get_parts,
    get_top_players,
};
pub use write::{delete_one, patch_one, post_create};

pub(crate) mod common;
pub(crate) mod read;
pub(crate) mod write;
