use rand::SeedableRng;

use arena_core::util::username_gen::*;

fn is_valid_word(s: &str) -> bool {
    s.chars()
        .next()
        .map(|c| c.is_ascii_lowercase())
        .unwrap_or(false)
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

#[test]
fn all_attributes_valid() {
    for word in ATTRIBUTES {
        assert!(is_valid_word(word), "invalid attribute: {:?}", word);
    }
}

#[test]
fn all_colours_valid() {
    for word in COLOURS {
        assert!(is_valid_word(word), "invalid colour: {:?}", word);
    }
}

#[test]
fn all_flowers_valid() {
    for word in FLOWERS {
        assert!(is_valid_word(word), "invalid flower: {:?}", word);
    }
}

#[test]
fn generated_username_is_lowercase() {
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    for _ in 0..100 {
        let name = generate_username(&mut rng);
        assert_eq!(
            name,
            name.to_lowercase(),
            "username not lowercase: {:?}",
            name
        );
    }
}

#[test]
fn generated_username_is_nonempty() {
    let mut rng = rand::rngs::StdRng::seed_from_u64(99);
    for _ in 0..100 {
        let name = generate_username(&mut rng);
        assert!(!name.is_empty(), "username was empty");
    }
}
