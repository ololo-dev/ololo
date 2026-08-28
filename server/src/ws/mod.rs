//! WebSocket subsystem.
//!
//! - [`session::ws_session_handler`] is the axum handler mounted at
//!   `GET /ws/s/:join_code`.
//! - [`project::ws_project_observe_handler`] is the axum handler mounted at
//!   `GET /ws/project/:project_id/observe`.
//! - [`admin::ws_admin_handler`] is the axum handler mounted at
//!   `GET /ws/admin`.
//! - [`zmq_events::ws_zmq_events_handler`] is the axum handler mounted at
//!   `GET /ws/admin/zmq`.

pub mod admin;
pub mod cli_auth;
pub mod git_http;
pub mod landing;
pub mod observe;
pub mod player;
pub mod project;
pub mod session;
pub mod zmq_events;

pub use admin::ws_admin_handler;
pub use git_http::git_http_handler;
pub use landing::ws_landing_observe_handler;
pub use observe::ws_observe_handler;
pub use player::ws_player_handler;
pub use project::ws_project_observe_handler;
pub use zmq_events::ws_zmq_events_handler;
