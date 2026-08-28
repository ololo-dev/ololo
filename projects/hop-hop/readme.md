---
schema_version: 1
project:
  name: Hop-Hop Game
  category: Code Golf
  tags:
    - code-golf
    - hop
    - cli
    - kata
  cover_image_url: https://ik.imagekit.io/tdf7wfnyrgb/projects/shutterstock_3x_63RExjrmf_ao7LtLdOG.jpg
  public: true
  archived_at: null
  points:
    value: 10
    fail: -5
    no_response: -10
    completion_bonus: 10
  intervals:
    deadline_secs: 60
    min_interval_secs: 5
    interval_increment_secs: 5
    max_interval_secs: 60
  session_duration_secs: 600
  memory:
    run: sh answer.sh
    test: sh test.sh
---

A ten-minute code-golf sprint built around the classic "hop" kata. For a
number: if it divides by 3, say `hop`; if it contains the digit 3, say `hop`;
if both are true, say `hop-hop`; otherwise just answer the number. Give it a
list and it replies for each one in order.

Making it correct is only the warm-up — the real game is shrinking your
solution as small as it will go. Every round tightens the budget, and the
player who golfs furthest before time runs out takes it, in any language.
