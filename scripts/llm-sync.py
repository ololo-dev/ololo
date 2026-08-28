#!/usr/bin/env python3
"""Mirror a server's LLM configuration onto another instance.

Copies providers, pools, the default/per-operation assignments, and
per-judge model overrides from a source server (typically plum.ololo.dev)
to a target (typically the local dev-play stack), so local judge runs use
the same providers and models as the deployment.

API keys are never exported by the source. Each provider's key is taken
from the local environment by catalog id — <CATALOG_ID>_API_KEY with `-`
as `_` (CEREBRAS_API_KEY, GROQ_API_KEY, OPENAI_API_KEY,
OPENCODE_GO_API_KEY, OPENROUTER_API_KEY, ...; opencode-* also falls back
to OPENCODE_API_KEY). A provider whose key is missing locally is created
DISABLED so pools keep their shape and failover skips it.

Usage:
  llm-sync.py --from URL --from-token T --to URL --to-token T [--dry-run]
"""

import argparse
import json
import os
import sys
import urllib.request


def api(base, token, path, method="GET", body=None):
    req = urllib.request.Request(
        base.rstrip("/") + path,
        method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            # Cloudflare fronts the deployments and blocks urllib's default UA.
            "User-Agent": "ololo-llm-sync/1.0",
        },
    )
    with urllib.request.urlopen(req) as resp:
        raw = resp.read()
        return json.loads(raw) if raw else None


def env_key_for(catalog_id):
    if not catalog_id:
        return None
    candidates = [catalog_id.upper().replace("-", "_") + "_API_KEY"]
    if catalog_id.startswith("opencode"):
        candidates.append("OPENCODE_API_KEY")
    for name in candidates:
        val = os.environ.get(name, "").strip()
        if val:
            return val
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--from", dest="src", required=True)
    ap.add_argument("--from-token", dest="src_token", required=True)
    ap.add_argument("--to", dest="dst", required=True)
    ap.add_argument("--to-token", dest="dst_token", required=True)
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    src = lambda p, **kw: api(args.src, args.src_token, p, **kw)  # noqa: E731
    dst = lambda p, **kw: api(args.dst, args.dst_token, p, **kw)  # noqa: E731

    s_providers = src("/api/admin/llm/providers")
    s_pools = src("/api/admin/llm/pools")
    s_assign = src("/api/admin/llm/assignments")
    s_judges = src("/api/admin/judges")
    if isinstance(s_judges, dict):
        s_judges = s_judges.get("judges", [])

    d_providers = dst("/api/admin/llm/providers")
    d_by_catalog = {p.get("catalog_id") or p["name"]: p for p in d_providers}

    # ── Providers ──────────────────────────────────────────────────────────
    provider_map = {}  # source id -> target id
    for sp in s_providers:
        key = env_key_for(sp.get("catalog_id"))
        needs_key = sp.get("has_api_key", False)
        enabled = sp["enabled"] and (key is not None or not needs_key)
        ident = sp.get("catalog_id") or sp["name"]
        existing = d_by_catalog.get(ident)
        body = {
            "name": sp["name"],
            "kind": sp["kind"],
            "base_url": sp.get("base_url"),
            "enabled": enabled,
            "catalog_id": sp.get("catalog_id"),
        }
        if key:
            body["api_key"] = key
        state = "with key" if key else ("no key -> disabled" if needs_key else "keyless")
        if args.dry_run:
            action = "update" if existing else "create"
            print(f"provider {sp['name']}: {action} ({state})")
            provider_map[sp["id"]] = (existing or {}).get("id", "dry")
            continue
        if existing:
            out = api(
                args.dst, args.dst_token,
                f"/api/admin/llm/providers/{existing['id']}", method="PUT", body=body,
            )
            provider_map[sp["id"]] = existing["id"]
            print(f"provider {sp['name']}: updated ({state})")
        else:
            out = dst("/api/admin/llm/providers", method="POST", body=body)
            provider_map[sp["id"]] = out["id"]
            print(f"provider {sp['name']}: created ({state})")

    # ── Pools ──────────────────────────────────────────────────────────────
    pool_map = {}  # source id -> target id
    d_pools = {p["name"]: p for p in (dst("/api/admin/llm/pools") or [])}
    for spool in s_pools:
        members = [
            {
                "provider_id": provider_map[m["provider_id"]],
                "model": m["model"],
                "priority": m["priority"],
                "enabled": m["enabled"],
            }
            for m in spool["members"]
            if m["provider_id"] in provider_map
        ]
        existing = d_pools.get(spool["name"])
        if args.dry_run:
            print(f"pool {spool['name']}: {'update' if existing else 'create'} ({len(members)} members)")
            pool_map[spool["id"]] = (existing or {}).get("id", "dry")
            continue
        if existing:
            api(
                args.dst, args.dst_token,
                f"/api/admin/llm/pools/{existing['id']}", method="PUT",
                body={"description": spool.get("description", ""), "members": members},
            )
            pool_map[spool["id"]] = existing["id"]
            print(f"pool {spool['name']}: updated ({len(members)} members)")
        else:
            out = dst(
                "/api/admin/llm/pools", method="POST",
                body={"name": spool["name"], "description": spool.get("description", ""), "members": members},
            )
            pool_map[spool["id"]] = out["id"]
            print(f"pool {spool['name']}: created ({len(members)} members)")

    # ── Assignments ────────────────────────────────────────────────────────
    def translate(a):
        if a is None:
            return None
        if "pool_id" in a:
            pid = pool_map.get(a["pool_id"])
            return {"pool_id": pid} if pid else None
        if "provider_id" in a:
            tid = provider_map.get(a["provider_id"])
            return {"provider_id": tid, "model": a["model"]} if tid else None
        return None

    body = {
        "default": translate(s_assign.get("default")),
        "operations": {
            op: translate(a) for op, a in (s_assign.get("operations") or {}).items()
        },
    }
    if args.dry_run:
        print(f"assignments: {json.dumps(body)}")
    else:
        dst("/api/admin/llm/assignments", method="PUT", body=body)
        print("assignments: applied")

    # ── Per-judge overrides ────────────────────────────────────────────────
    d_judges = dst("/api/admin/judges")
    if isinstance(d_judges, dict):
        d_judges = d_judges.get("judges", [])
    d_by_slug = {j["slug"]: j for j in d_judges}
    for sj in s_judges:
        has_override = sj.get("llm_provider_id") or sj.get("llm_pool_id")
        if not has_override:
            continue
        tj = d_by_slug.get(sj["slug"])
        if not tj:
            print(f"judge {sj['slug']}: not present on target, skipped")
            continue
        patch = {}
        if sj.get("llm_provider_id"):
            tid = provider_map.get(sj["llm_provider_id"])
            if tid:
                patch["llm_provider_id"] = tid
                patch["llm_model"] = sj.get("llm_model")
        if sj.get("llm_pool_id"):
            pid = pool_map.get(sj["llm_pool_id"])
            if pid:
                patch["llm_pool_id"] = pid
        if sj.get("llm_source_order"):
            patch["llm_source_order"] = sj["llm_source_order"]
        if not patch:
            continue
        if args.dry_run:
            print(f"judge {sj['slug']}: {json.dumps(patch)}")
        else:
            api(args.dst, args.dst_token, f"/api/admin/judges/{tj['id']}", method="PUT", body=patch)
            print(f"judge {sj['slug']}: override applied")

    print("done")


if __name__ == "__main__":
    try:
        main()
    except urllib.error.HTTPError as e:
        print(f"HTTP {e.code} on {e.url}: {e.read().decode(errors='replace')[:300]}", file=sys.stderr)
        sys.exit(1)
