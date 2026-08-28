//! Argon2id password hashing per FR-001.
//!
//! Uses `argon2::Argon2::default()` (Argon2id, OWASP baseline:
//! m=19456 KiB, t=2, p=1) with a per-password OS-random salt.

use crate::auth::AuthError;
use argon2::Argon2;
use argon2::password_hash::{
    PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng,
};

/// Hash `password` with a freshly-generated salt. Returns the PHC string.
pub fn hash_password(argon2: &Argon2<'_>, password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| AuthError::HashFailed)?;
    Ok(hash.to_string())
}

/// Verify `password` against a stored PHC string. Returns `true` on match,
/// `false` on mismatch. Returns `HashParseFailed` if the stored string is
/// not a valid PHC encoding.
pub fn verify_password(argon2: &Argon2<'_>, hash: &str, password: &str) -> Result<bool, AuthError> {
    let parsed = PasswordHash::new(hash).map_err(|_| AuthError::HashParseFailed)?;
    match argon2.verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(_) => Err(AuthError::HashParseFailed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_succeeds() {
        let argon2 = Argon2::default();
        let hash = hash_password(&argon2, "correct horse battery staple").unwrap();
        assert!(verify_password(&argon2, &hash, "correct horse battery staple").unwrap());
    }

    #[test]
    fn verify_with_wrong_password_fails() {
        let argon2 = Argon2::default();
        let hash = hash_password(&argon2, "correct horse battery staple").unwrap();
        assert!(!verify_password(&argon2, &hash, "wrong password").unwrap());
    }

    #[test]
    fn parse_failure_on_garbage_hash() {
        let argon2 = Argon2::default();
        let err = verify_password(&argon2, "not-a-phc-string", "x").unwrap_err();
        matches!(err, AuthError::HashParseFailed);
    }
}
