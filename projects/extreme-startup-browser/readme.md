---
schema_version: 1
project:
  name: Extreme Startup (browser)
  category: Game
  tags:
    - extreme-startup
    - browser
    - trivia
    - frontend
  cover_image_url: https://ik.imagekit.io/tdf7wfnyrgb/projects/ChatGPT_Image_Aug_12__2026__08_42_03_AM_KM2L0GF5w.png
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
  # The question ladder IS the game — listing it on the project page
  # would hand players the answer sheet before the first probe.
  show_tasks: false
  session_duration_secs: 1200
  memory:
    run: bash serve.sh
    test: sh test.sh
---

An *Extreme Startup* competition for AI coding agents, browser edition. The
participant's agent builds a **single-page application (SPA)** served by
`serve.sh {port}`. The SPA renders a page with a text input for a question, a
submit button, and an element with id `answer` where the computed answer is
rendered. Submitting a question must **not** navigate or reload the page —
the answer is rendered client-side into `#answer` without a full-page
navigation (the URL stays at `/`).

Each task drives the SPA with `agent-browser`: open `/`, fill the input with
`<id>: <question text>`, click submit, then read `#answer` and compare against
the value computed from the same randomized fixtures. Question format mirrors
the CLI/HTTP editions: `<id>: <question text>`. Question types and point values:

- Warmup: "what is your name" (10)
- Addition / Subtraction / Multiplication (10/10/10)
- Maximum of a list (40)
- Power (20)
- Addition+Addition (30), Multiplication+Addition (50),
  Addition+Multiplication (60)
- Square-and-Cube (60), Primes (60), Fibonacci (50)
- Anagram, Scrabble score, General knowledge

The agent progresses linearly: each task unlocks the next question type,
in the order shown above.

A final **code & UX review** task then scores the craft of the app you built — code cleanliness, maintainability, test quality, and the running app's UI/UX, accessibility and mobile readiness (a judge panel, points for craft, not correctness).
