#!/usr/bin/env python3
"""Refresh `src/pricing/models.json` from models.dev.

Usage: scripts/update-model-pricing.py [path-to-api.json]

Without an argument the script downloads https://models.dev/api.json. The
output is a compact table keyed by bare model id (no provider prefix) with
per-million-token USD prices: `[input, output, cache_read, cache_write]` plus
an optional fifth `reasoning` price when the vendor bills thinking tokens
separately. Free/unpriced models are dropped: a missing entry means "unknown",
which the CLI shows as "—" rather than a misleading $0.
"""
import json
import sys
import urllib.request
from datetime import date, timezone, datetime
from pathlib import Path

API_URL = "https://models.dev/api.json"
OUT = Path(__file__).resolve().parent.parent / "src" / "pricing" / "models.json"

# First-party vendors win over gateways/resellers when the same id is listed
# by several providers: the vendor's own list price is what a coding agent
# talking to the vendor pays.
PREFERRED = [
    "anthropic", "openai", "google", "google-vertex", "xai", "moonshotai", "zai",
    "deepseek", "mistral", "alibaba", "alibaba-cn", "meta", "minimax", "cohere",
    "perplexity", "groq", "cerebras", "fireworks-ai", "togetherai", "amazon-bedrock",
    "azure", "github-copilot", "opencode", "kilo",
]


def rank(provider: str) -> tuple[int, str]:
    try:
        return (PREFERRED.index(provider), provider)
    except ValueError:
        return (len(PREFERRED), provider)


def bare(model_id: str) -> str:
    # Gateways list "openai/gpt-5.5"; Bedrock lists "us.anthropic.claude-opus-5".
    mid = model_id.lower().rsplit("/", 1)[-1]
    for prefix in ("us.", "eu.", "jp.", "au.", "apac.", "global."):
        if mid.startswith(prefix):
            mid = mid[len(prefix):]
    for prefix in ("anthropic.", "meta.", "mistral.", "amazon.", "cohere.", "ai21."):
        if mid.startswith(prefix):
            mid = mid[len(prefix):]
    return mid


def main() -> None:
    if len(sys.argv) > 1:
        raw = Path(sys.argv[1]).read_text()
    else:
        with urllib.request.urlopen(API_URL, timeout=30) as resp:
            raw = resp.read().decode()
    api = json.loads(raw)

    best: dict[str, tuple[tuple[int, str], list]] = {}
    for provider, pdata in api.items():
        for model_id, model in (pdata.get("models") or {}).items():
            cost = model.get("cost") or {}
            inp, out = cost.get("input"), cost.get("output")
            if not isinstance(inp, (int, float)) or not isinstance(out, (int, float)):
                continue
            if inp <= 0 and out <= 0:
                continue
            row = [inp, out, cost.get("cache_read") or 0, cost.get("cache_write") or 0]
            if isinstance(cost.get("reasoning"), (int, float)):
                row.append(cost["reasoning"])
            key = bare(model_id)
            r = rank(provider)
            if key not in best or r < best[key][0]:
                best[key] = (r, row)

    # A date-suffixed alias listed only by a reseller (often without cache
    # prices) defers to the vendor's own undated row when that ranks higher.
    for key in list(best):
        stem = key[:-9] if len(key) > 9 and key[-9] == "-" and key[-8:].isdigit() else None
        if stem and stem in best and best[stem][0] < best[key][0]:
            best[key] = best[stem]

    table = {k: v for k, (_, v) in sorted(best.items())}
    payload = {
        "generated": datetime.now(timezone.utc).date().isoformat(),
        "source": API_URL,
        "unit": "usd_per_million_tokens",
        "fields": ["input", "output", "cache_read", "cache_write", "reasoning?"],
        "models": table,
    }
    OUT.write_text(json.dumps(payload, separators=(",", ":"), sort_keys=False) + "\n")
    print(f"wrote {OUT} ({len(table)} models, {OUT.stat().st_size // 1024} KiB)")


if __name__ == "__main__":
    main()
