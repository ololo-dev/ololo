const MIN_LEN: usize = 4;
const MAX_LEN: usize = 30;

const RESERVED: &[&str] = &[
    "admin", "support", "ololo", "api", "www", "mail", "help", "about", "terms", "privacy",
    "login", "register", "logout", "u", "health",
];

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum UsernameError {
    #[error("username must be between 4 and 30 characters")]
    InvalidLength,
    #[error("username contains invalid characters")]
    InvalidFormat,
    #[error("username is reserved")]
    Reserved,
}

pub fn validate_username(s: &str) -> Result<(), UsernameError> {
    // Length check
    if s.len() < MIN_LEN || s.len() > MAX_LEN {
        return Err(UsernameError::InvalidLength);
    }

    // Reject uppercase explicitly
    if s.chars().any(|c| c.is_uppercase()) {
        return Err(UsernameError::InvalidFormat);
    }

    // All chars must be [a-z0-9_-]
    if !s
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(UsernameError::InvalidFormat);
    }

    // First char must be [a-z]
    if !s
        .chars()
        .next()
        .map(|c| c.is_ascii_lowercase())
        .unwrap_or(false)
    {
        return Err(UsernameError::InvalidFormat);
    }

    // Last char must be [a-z0-9]
    if !s
        .chars()
        .last()
        .map(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .unwrap_or(false)
    {
        return Err(UsernameError::InvalidFormat);
    }

    // Reserved words
    if RESERVED.contains(&s) {
        return Err(UsernameError::Reserved);
    }

    Ok(())
}
