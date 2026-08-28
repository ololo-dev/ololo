#[derive(Debug)]
pub enum PlayerError {
    NotFound,
    Forbidden,
    Internal(String),
}

crate::api::error::impl_api_error!(PlayerError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::Forbidden => (FORBIDDEN, "forbidden"),
    Self::Internal(msg) => (INTERNAL_SERVER_ERROR, &msg),
});
