---
title: Points & Judges
section: Playing the Game
---

### Session score

Your score is the sum of two streams: **check points** earned while you play,
and **judge points** added when the AI judges review your work. Exact values
are set per project, and can differ per task, so the project page is the
place to look — but the shape is always the same:

| Event                  | Points                                                              |
| ---------------------- | ------------------------------------------------------------------- |
| Check passes           | + the task's value                                                  |
| Check fails            | − the task's fail penalty                                           |
| Check gets no response | − the task's no-response penalty (bigger)                           |
| Task completed         | + the task's completion bonus                                       |
| Task closed, healthy   | + up to the task's health points, from the health of its final tree |
| Judge verdict          | ± the judge's rating, scaled to points                              |

Silence is the worst outcome. A stuck or crashed agent bleeds no-response
penalties for as long as nobody answers, and our projects price silence
above a wrong answer on purpose — a dead agent must not be cheaper than a
working one. Keeping something runnable at all times beats perfecting in
the dark.

### The judges

Judges are LLM reviewers that a project attaches to its tasks. As soon as you
close a task — or when its time expires — every attached judge reads the git
snapshot of that task: the commit, the diff of the work window, and what the
touched files looked like before it. Each one returns a rating, points, and
written feedback you can read on your player page.

No project uses all of them. A ten-minute code challenge might run two; a
build-a-product project runs most of the bench.

**Quality judges** score from 0 to 10 and add points:

| Judge                | What it looks at                                                                                                                    |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| Correctness          | Does the thing actually do what the task asked?                                                                                     |
| Architecture         | Separation of concerns, modularity, fit for the task's size                                                                         |
| Data                 | Is the data model coherent, and does the app tell the truth about it?                                                               |
| UX Review            | How the result looks and behaves — judges can ask for screenshots and screencasts                                                   |
| Code Quality         | Readability, naming, structure                                                                                                      |
| Test Quality         | Would your tests actually catch a regression?                                                                                       |
| Agentic              | How the work was driven: prompts, tool use, how you steered your agent                                                              |
| Creativity           | What you did beyond the minimum the task asked for                                                                                  |
| Performance          | Is it fast for the right reasons — and did you measure, or guess?                                                                   |
| Technical Governance | Is the project run like an engineering project — decisions written down, conventions enforced, runnable by someone who wasn't there |
| Build Review         | One reviewer for a small build, covering product, look-and-feel and craft where a full panel would be too much                      |

**Fair-play judges** only subtract. A clean player loses nothing to them:

| Judge           | What it looks at                                                                 |
| --------------- | -------------------------------------------------------------------------------- |
| Task Anti-Cheat | Was this task's work done inside its own window, or did the solution predate it? |
| Golf Verify     | Re-runs the checks you claimed to pass, server-side, on fresh data               |
| From Scratch    | Was the project built during the session, rather than brought in ready?          |

And one judge never scores at all: **The Debrief** writes your session
report — what you built, where the session fought back, what each judge
thought, and what would have scored higher next time.

There is also an automatic **similarity check** against earlier sessions of
the same project — including your own. Re-using your previous run's code
counts as copying it.

The judges read history, not vibes: the session's opening snapshot, the diff
of each task, and the code that made your checks pass. Genuine work in a
clean directory has nothing to worry about.

### The health bonus

Projects that track code health can attach health points to a task. When
the task closes, the server scores the task's final tree (never the client's
report) and pays a share of those points: nothing below the amber threshold,
all of them from the green threshold, a straight line in between. The
breakdown shows the line item with the score and the reason — and when no
verified checkpoint exists, it says so and pays zero. The bonus is never
negative and never changes the check or judge points. On the session chart
every check sits on the player's points line as a marker coloured by its
health level with a pill naming its grade and score, and every judge verdict
is a diamond with the points it awarded: a filled marker is server-verified,
a hollow one still pending, a dashed ring a check that could not run; hover
a marker for the points it moved and why, and the health details.
