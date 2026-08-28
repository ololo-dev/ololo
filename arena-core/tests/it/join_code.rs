use arena_core::join_code;

#[test]
fn generates_correct_length() {
    let code = join_code::generate();
    assert_eq!(code.len(), 6);
}

#[test]
fn uses_valid_alphabet_only() {
    let alphabet: std::collections::HashSet<char> =
        "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".chars().collect();
    for _ in 0..100 {
        let code = join_code::generate();
        for c in code.chars() {
            assert!(alphabet.contains(&c), "unexpected char: {c}");
        }
    }
}
