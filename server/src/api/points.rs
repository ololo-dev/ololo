//! Shared points wire types for project and task CRUD.
//!
//! `PointsReq` is the input shape (per-field `Option<i32>`; `None` means
//! "inherit project default" on create, "preserve current value" on patch).
//! `PointsResp` is the output shape (resolved `i32` per field). `PointsRange`
//! is the min/max of `tasks.point_value` across a project's tasks, surfaced
//! on single-project reads for the frontend's "Points: min–max" display.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PointsReq {
    #[serde(default)]
    pub value: Option<i32>,
    #[serde(default)]
    pub fail: Option<i32>,
    #[serde(default)]
    pub no_response: Option<i32>,
    #[serde(default)]
    pub completion_bonus: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PointsResp {
    pub value: i32,
    pub fail: i32,
    pub no_response: i32,
    pub completion_bonus: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PointsRange {
    pub min: i32,
    pub max: i32,
}
