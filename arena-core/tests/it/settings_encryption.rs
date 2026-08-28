use arena_core::settings_encryption::*;

fn make_enc() -> SettingsEncryption {
    SettingsEncryption::new(b"test-signing-key-at-least-32-bytes!!")
}

#[test]
fn encrypt_decrypt_roundtrip() {
    let enc = make_enc();
    let plaintext = "AKIAIOSFODNN7EXAMPLE";
    let stored = enc.encrypt(plaintext);
    let recovered = enc.decrypt(&stored).expect("should decrypt");
    assert_eq!(recovered, plaintext);
}

#[test]
fn repeated_encrypt_differs() {
    let enc = make_enc();
    let a = enc.encrypt("secret");
    let b = enc.encrypt("secret");
    assert_ne!(a, b, "nonces should differ");
}

#[test]
fn wrong_key_fails_decrypt() {
    let enc1 = SettingsEncryption::new(b"key-one-at-least-32-bytes-long!!");
    let enc2 = SettingsEncryption::new(b"key-two-at-least-32-bytes-long!!");
    let stored = enc1.encrypt("secret");
    assert!(enc2.decrypt(&stored).is_err());
}

#[test]
fn too_short_stored_value_errors() {
    let enc = make_enc();
    let result = enc.decrypt("deadbeefdeadbeefdeadbeef");
    let result2 = enc.decrypt("deadbeef");
    assert!(matches!(result2, Err(EncryptionError::TooShort)));
    assert!(result.is_err());
}

#[test]
fn invalid_hex_errors() {
    let enc = make_enc();
    let result = enc.decrypt("not-valid-hex!!");
    assert!(matches!(result, Err(EncryptionError::HexDecode(_))));
}
