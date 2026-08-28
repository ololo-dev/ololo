//! `/auth/*` and `/api/users/*` HTTP handlers (FR-001, FR-002, FR-007).
//!
//! Handlers are grouped by surface and re-exported here for `lib.rs` wiring.
//! Public request/response DTOs are re-exported for API fidelity.

pub use auth::{
    LoginRequest, RegisterRequest, TokenResponse, UserDto, post_login, post_logout, post_refresh,
    post_register,
};
pub use me::{
    AvatarAuthResponse, ChangePasswordRequest, MeDto, PatchMeRequest, get_avatar_auth, get_me,
    patch_me, post_change_password,
};
pub use public::{
    PublicSessionEntry, PublicSessionsResponse, PublicUserDto, SessionsQuery, get_by_username,
    get_sessions_by_username,
};

pub(crate) mod auth;
pub(crate) mod me;
pub(crate) mod public;
