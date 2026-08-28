/**
 * Password strength scoring for the sign-up form.
 *
 * Deliberately dependency-free and small: this is a hint that nudges someone
 * away from `password1`, not an authority. The server owns the actual rule
 * (minimum 8 characters), and nothing here can block a submit — a meter that
 * vetoes passwords a password manager generated is worse than no meter.
 */

/** 0 is unusable, 4 is as good as this hint can tell. */
export type StrengthScore = 0 | 1 | 2 | 3 | 4;

export interface PasswordStrength {
  score: StrengthScore;
  /** Shown next to the meter — the meter must never speak in colour alone. */
  label: string;
  /** One actionable suggestion, or '' once there is nothing useful to add. */
  hint: string;
}

/** The server's rule, mirrored so the meter and the field agree. */
export const MIN_PASSWORD_LENGTH = 8;

/**
 * Shapes that look long but carry almost no entropy. Kept short on purpose:
 * a real breach list belongs on the server, and a 100-entry array shipped to
 * every visitor buys very little over catching the obvious cases.
 */
const COMMON = [
  "password",
  "qwerty",
  "azerty",
  "letmein",
  "welcome",
  "admin",
  "iloveyou",
  "monkey",
  "dragon",
  "football",
  "baseball",
  "sunshine",
  "princess",
  "abc123",
];

/** `aaaaaaaa`, `12121212` — length that repetition paid for. */
const isRepetitive = (s: string): boolean => {
  if (/^(.)\1+$/.test(s)) return true;
  // A unit of 1-3 chars tiled across the whole string.
  return /^(.{1,3}?)\1{2,}$/.test(s);
};

/** `abcdef`, `123456`, and their descending twins. */
const hasLongRun = (s: string): boolean => {
  const lower = s.toLowerCase();
  let ascending = 1;
  let descending = 1;
  for (let i = 1; i < lower.length; i++) {
    const step = lower.charCodeAt(i) - lower.charCodeAt(i - 1);
    ascending = step === 1 ? ascending + 1 : 1;
    descending = step === -1 ? descending + 1 : 1;
    if (ascending >= 5 || descending >= 5) return true;
  }
  return false;
};

/**
 * Undo the substitutions people reach for when a meter nags them: `P@ssw0rd`
 * is `password` to anyone running a cracking rule set, so it must be
 * `password` to the common-word check too.
 */
const deLeet = (s: string): string =>
  s
    .toLowerCase()
    .replace(/[@4]/g, "a")
    .replace(/[$5]/g, "s")
    .replace(/[03]/g, (c) => (c === "0" ? "o" : "e"))
    .replace(/[1!|]/g, "i")
    .replace(/7/g, "t");

const classesIn = (s: string): number =>
  Number(/[a-z]/.test(s)) +
  Number(/[A-Z]/.test(s)) +
  Number(/[0-9]/.test(s)) +
  Number(/[^A-Za-z0-9]/.test(s));

const LABELS: Record<StrengthScore, string> = {
  0: "Too short",
  1: "Weak",
  2: "Fair",
  3: "Good",
  4: "Strong",
};

/**
 * Score a candidate password 0-4.
 *
 * Length carries most of the weight because it genuinely does: a long
 * passphrase of nothing but lowercase letters beats a short one wearing every
 * character class. Character variety tops it up, and the shortcuts people
 * reach for when a meter tells them to "add a number" are capped.
 */
export function scorePassword(password: string): PasswordStrength {
  if (!password) return { score: 0, label: LABELS[0], hint: "" };

  if (password.length < MIN_PASSWORD_LENGTH) {
    return {
      score: 0,
      label: LABELS[0],
      hint: `Use at least ${MIN_PASSWORD_LENGTH} characters.`,
    };
  }

  const normalised = deLeet(password);
  const looksCommon = COMMON.some((c) => normalised.includes(c));
  const looksMechanical = isRepetitive(password) || hasLongRun(password);

  // Four length tiers, so a long passphrase can reach Strong on length alone
  // — which is the honest answer, and the habit worth encouraging.
  let points = 0;
  if (password.length >= 8) points += 1;
  if (password.length >= 12) points += 1;
  if (password.length >= 16) points += 1;
  if (password.length >= 20) points += 1;

  const classes = classesIn(password);
  if (classes >= 2) points += 1;
  if (classes >= 3) points += 1;

  // A recognisable word or a tiled pattern is guessed long before it is
  // brute-forced, so no amount of length or punctuation earns it a pass.
  if (looksCommon || looksMechanical) points = Math.min(points, 1);

  const score = Math.max(1, Math.min(4, points)) as StrengthScore;

  let hint = "";
  if (looksCommon) hint = "Avoid common words and predictable substitutions.";
  else if (looksMechanical) hint = "Avoid repeated or sequential characters.";
  else if (score < 4 && password.length < 12) hint = "Longer is stronger — try a passphrase.";
  else if (score < 4 && classes < 3) hint = "Mix in capitals, digits, or symbols.";

  return { score, label: LABELS[score], hint };
}
