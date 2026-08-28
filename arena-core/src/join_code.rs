//! Join code generation utility. FR-JC-002: 6-char uppercase base-32
//! from alphabet `ABCDEFGHIJKLMNOPQRSTUVWXYZ234567`.

use rand::Rng;

const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
const CODE_LEN: usize = 6;

/// Generate a random 6-character join code from the base-32 alphabet.
pub fn generate() -> String {
    let mut rng = rand::thread_rng();
    (0..CODE_LEN)
        .map(|_| {
            let idx = rng.gen_range(0..ALPHABET.len());
            ALPHABET[idx] as char
        })
        .collect()
}
