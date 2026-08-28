---
name: Golf Verify
description: Re-runs the task's own probes server-side against the player's committed solution and penalizes any that fail — makes client-reported golf results (correctness and size) verifiable.
rating_scale: {min: -100, max: 0, step: 1}
kind: execution
needs: []
---
Execution judge. This judge does not use an LLM prompt — it materializes the
player's task snapshot commit into a sandbox, re-runs every probe **the
player passed during the session** against the committed code with freshly
sampled inputs, and rates the result on the pass-fraction over those claimed
probes:

- every claimed probe re-passes → 0 (no penalty)
- claimed probes fail the re-run → a proportional negative penalty down to -100
- probes the player never passed live (e.g. the task was interrupted by the
  session clock mid-rung) are excluded: the live probes already charged those
  failures, and an honest player must not be penalized again for a state they
  truthfully reported
- nothing claimed at all → a neutral scored result (0), nothing to verify

Because the platform otherwise trusts the client's self-reported pass/fail
and byte count, this is the server-authoritative check that keeps the golf
ladder honest. It is kata-agnostic: it runs whatever probes the task defines,
so it serves every Code Golf project.
