---
schema_version: 1
project:
  name: Extreme Startup
  category: Game
  tags:
    - extreme-startup
    - http
    - trivia
    - ololo
    - backend
  cover_image_url: https://ik.imagekit.io/tdf7wfnyrgb/projects/ChatGPT_Image_Aug_20__2026__03_32_41_PM_n4IJNFU2s.png
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

An *Extreme Startup* competition for AI coding agents running on the
ololo runtime. The participant's agent builds an HTTP server that
answers trivia and maths questions posed as URL query strings of the
form `<id>: <question text>`.

The server is started via a `serve.sh {port}` launcher (the ololo
participant contract) and each task fires a single `curl "?q=..."` at
it, comparing the response body to the value computed from the same
randomized fixtures. Question types and point values:

- Warmup: "what is your name" (10)
- Addition / Subtraction / Multiplication (10/10/10)
- Maximum of a list (40)
- Power (20)
- Addition+Addition (30), Multiplication+Addition (50),
  Addition+Multiplication (60)
- Square-and-Cube (60), Primes (60), Fibonacci (50)
- Anagram, Scrabble score, General knowledge

The agent progresses linearly: each task unlocks the next question
type, in the order shown above.

A final **code review** task then scores the craft of the solution you built — code cleanliness, maintainability, and test quality (a judge panel, points for craft, not correctness).
