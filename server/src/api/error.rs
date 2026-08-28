//! Shared rendering for API error responses.
//!
//! Every API error module maps its domain enum onto the same wire shape:
//! an HTTP status plus a JSON body of `{"error": <code>}`, occasionally with
//! one extra payload field. `impl_api_error!` generates the `IntoResponse`
//! impl from that mapping; irregular impls (bare statuses, headers, logging)
//! stay hand-written but should still render JSON arms via [`error_response`].

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Render the standard error body `{"error": code}`.
pub fn error_response(status: StatusCode, code: &str) -> Response {
    error_response_with(status, code, [])
}

/// Render `{"error": code}` plus extra payload fields.
pub fn error_response_with<'a>(
    status: StatusCode,
    code: &str,
    extra: impl IntoIterator<Item = (&'a str, serde_json::Value)>,
) -> Response {
    let mut body = serde_json::Map::new();
    body.insert("error".into(), code.into());
    for (key, value) in extra {
        body.insert(key.into(), value);
    }
    (status, Json(serde_json::Value::Object(body))).into_response()
}

/// Generate an `IntoResponse` impl from a variant → (status, code) table.
///
/// Arms use `Self::`-qualified patterns; the status is a bare `StatusCode`
/// associated-constant name. A variant that carries payload data can append
/// `, "key": value` pairs rendered alongside the error code:
///
/// ```ignore
/// impl_api_error!(ProjectError {
///     Self::NotFound => (NOT_FOUND, "not_found"),
///     Self::ProjectInUse { session_count } =>
///         (CONFLICT, "project_in_use", "session_count": session_count),
/// });
/// ```
macro_rules! impl_api_error {
    ($ty:ty { $( $pat:pat => ($status:ident, $code:expr $(, $key:literal : $val:expr)* $(,)?) ),+ $(,)? }) => {
        impl ::axum::response::IntoResponse for $ty {
            fn into_response(self) -> ::axum::response::Response {
                match self {
                    $( $pat => $crate::api::error::error_response_with(
                        ::axum::http::StatusCode::$status,
                        $code,
                        [ $( ($key, ::serde_json::Value::from($val)) ),* ],
                    ) ),+
                }
            }
        }
    };
}
pub(crate) use impl_api_error;
