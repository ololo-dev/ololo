import { describe, it, expect } from "vitest";
import { scorePassword, MIN_PASSWORD_LENGTH } from "./password-strength";

describe("scorePassword", () => {
  it("says_nothing_about_an_empty_field", () => {
    const { score, label, hint } = scorePassword("");
    expect(score).toBe(0);
    expect(label).toBe("Too short");
    // An untouched field has not earned a complaint yet.
    expect(hint).toBe("");
  });

  it("scores_anything_under_the_server_minimum_as_too_short", () => {
    for (const pw of ["a", "Ab3$x", "1234567"]) {
      expect(pw.length).toBeLessThan(MIN_PASSWORD_LENGTH);
      expect(scorePassword(pw).score).toBe(0);
    }
    expect(scorePassword("1234567").hint).toContain(String(MIN_PASSWORD_LENGTH));
  });

  it("rewards_length_over_character_class_theatre", () => {
    // The short one wears every class; the long one is bare lowercase words.
    const short = scorePassword("Ab3$Ab3$");
    const long = scorePassword("correcthorsebatterystaple");
    expect(long.score).toBeGreaterThan(short.score);
    expect(long.label).toBe("Strong");
  });

  it("caps_common_words_however_they_are_dressed_up", () => {
    // Long, mixed-case, punctuated — and still built on `password`.
    const dressed = scorePassword("P@ssword!2024xyz");
    expect(dressed.score).toBe(1);
    expect(dressed.label).toBe("Weak");
    expect(dressed.hint).toContain("common");
  });

  it("caps_repetition_and_sequences_that_only_look_long", () => {
    for (const pw of ["aaaaaaaaaaaa", "abcabcabcabc", "abcdefghijkl"]) {
      const { score, hint } = scorePassword(pw);
      expect(score).toBe(1);
      expect(hint).not.toBe("");
    }
  });

  it("climbs_as_a_password_actually_improves", () => {
    const scores = [
      scorePassword("kittens1").score,
      scorePassword("kittensRun12").score,
      scorePassword("kittensRun12!$xy").score,
    ];
    // Monotonic: every step up must not score lower than the one before.
    expect(scores[1]).toBeGreaterThanOrEqual(scores[0]);
    expect(scores[2]).toBeGreaterThanOrEqual(scores[1]);
    expect(scores[2]).toBe(4);
  });

  it("never_returns_a_score_outside_the_meter", () => {
    const samples = [
      "",
      "x",
      "12345678",
      "correcthorsebatterystaple",
      "P@ssw0rd",
      "🙂🙂🙂🙂🙂🙂🙂🙂",
    ];
    for (const pw of samples) {
      const { score, label } = scorePassword(pw);
      expect(score).toBeGreaterThanOrEqual(0);
      expect(score).toBeLessThanOrEqual(4);
      expect(label).not.toBe("");
    }
  });

  it("always_pairs_a_score_with_words_not_just_a_colour", () => {
    // Accessibility contract: the meter's meaning must survive without colour.
    for (const pw of ["1234567", "kittens1", "correcthorsebatterystaple"]) {
      expect(scorePassword(pw).label.trim()).not.toBe("");
    }
  });
});
