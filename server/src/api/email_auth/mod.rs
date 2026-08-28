//! Email authentication handlers (verify, password reset, magic link).
//!
//! Public route handlers and shared types are re-exported here for `lib.rs`.
//! See [`common`] for [`EmailAuthError`], [`PeerIp`], and request query types.

pub use common::{
    EmailAuthError, ForgotPasswordRequest, MagicLinkRequest, MagicLinkVerifyQuery, PeerIp,
    ResetPasswordRequest, VerifyEmailQuery,
};
pub use magic_link::{get_magic_link_verify, post_magic_link_request};
pub use password::{post_forgot_password, post_reset_password};
pub use verify::{get_verify_email, post_resend_verification};

pub(crate) mod common;
pub(crate) mod magic_link;
pub(crate) mod password;
pub(crate) mod verify;
