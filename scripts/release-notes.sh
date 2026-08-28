#!/usr/bin/env bash
# Build a release body from the commits since the previous tag.
#
# GitHub's generated notes are useless here: this repository commits straight
# to master, so there are no pull requests to summarise and the auto notes
# degrade to a flat list of subjects. Conventional-commit prefixes carry the
# shape instead — group by type, keep the scope, drop the noise.
#
# Usage: scripts/release-notes.sh <tag> [previous-tag]
# Writes markdown to stdout. Needs full history (fetch-depth: 0 in CI).
#
# Deliberately bash-3.2 clean (no mapfile, no associative arrays): macOS ships
# 3.2, and a script that only runs on the CI runner cannot be checked before
# it is relied on.
set -euo pipefail

tag="${1:?usage: release-notes.sh <tag> [previous-tag]}"
prev="${2:-}"

if [ -z "$prev" ]; then
  # The tag before this one, walked from the tag itself so a release cut from
  # an older commit still compares against its own ancestor.
  prev="$(git describe --tags --abbrev=0 "${tag}^" 2>/dev/null || true)"
fi

range="${tag}"
[ -n "$prev" ] && range="${prev}..${tag}"
repo="${GITHUB_REPOSITORY:-ololo-dev/app}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

maintenance=0

# Subjects carry the entry; bodies are read only to catch a breaking-change
# note. The long "why" prose stays in `git log`, not on a release page.
for sha in $(git log --no-merges --pretty=%H "$range"); do
  subject="$(git log -1 --pretty=%s "$sha")"
  [ -n "$subject" ] || continue

  # type(scope): description — scope optional, "!" marks a breaking change.
  if [[ "$subject" =~ ^([a-z]+)(\(([^\)]+)\))?(!)?:[[:space:]]*(.+)$ ]]; then
    type="${BASH_REMATCH[1]}"
    scope="${BASH_REMATCH[3]}"
    breaking="${BASH_REMATCH[4]}"
    text="${BASH_REMATCH[5]}"
  else
    type="other"
    scope=""
    breaking=""
    text="$subject"
  fi

  case "$type" in
    feat) section="1-features" ;;
    fix) section="2-fixes" ;;
    perf) section="3-performance" ;;
    refactor) section="4-refactoring" ;;
    release) section="5-client" ;;
    build | ci | deploy) section="6-build" ;;
    docs) section="7-docs" ;;
    # Housekeeping is counted, not listed: nobody reads a release page to
    # learn that formatting was applied.
    test | style | chore)
      maintenance=$((maintenance + 1))
      continue
      ;;
    *) section="8-other" ;;
  esac

  # This repository announces breaks in the body ("BREAKING: ...") rather
  # than with a "!" in the subject, and a break is the one thing a reader
  # must not miss.
  if [ -z "$breaking" ] && git log -1 --pretty=%b "$sha" | grep -qE '^BREAKING( CHANGE)?:'; then
    breaking="!"
  fi

  line="- "
  [ -n "$breaking" ] && line="${line}**BREAKING** "
  [ -n "$scope" ] && line="${line}**${scope}**: "
  line="${line}${text}"
  printf '%s\n' "$line" >>"$work/$section"
done

title_for() {
  case "$1" in
    1-features) echo "Features" ;;
    2-fixes) echo "Fixes" ;;
    3-performance) echo "Performance" ;;
    4-refactoring) echo "Refactoring" ;;
    5-client) echo "Client releases" ;;
    6-build) echo "Build & deploy" ;;
    7-docs) echo "Documentation" ;;
    *) echo "Other" ;;
  esac
}

echo "## What changed"
echo

printed=0
for section in 1-features 2-fixes 3-performance 4-refactoring 5-client 6-build 7-docs 8-other; do
  if [ -s "$work/$section" ]; then
    echo "### $(title_for "$section")"
    cat "$work/$section"
    echo
    printed=1
  fi
done

if [ "$printed" -eq 0 ]; then
  echo "_No user-facing changes._"
  echo
fi

if [ "$maintenance" -gt 0 ]; then
  plural="s"
  [ "$maintenance" -eq 1 ] && plural=""
  echo "_Plus ${maintenance} maintenance commit${plural} (tests, formatting, chores)._"
  echo
fi

if [ -n "$prev" ]; then
  echo "**Full changelog**: https://github.com/${repo}/compare/${prev}...${tag}"
else
  echo "**Full changelog**: https://github.com/${repo}/commits/${tag}"
fi
