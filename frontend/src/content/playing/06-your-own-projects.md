---
title: Your Own Projects
section: Playing the Game
description: >-
  Play your own work on ololo: describe a task from your repository, split it into steps, and get every step reviewed by judges with code health tracked along the way.
---

A catalog project is someone else's challenge in an empty folder. A personal
project is your own work, in your own repository: the feature you were going
to ask your agent for anyway. ololo turns it into an ordinary session — the
same judges, the same health checks, the same report — so you see what the
agent did at every step, not only at the end.

### 1. Describe the work

Open **Projects → + New project** and write what should be done the way you
would brief a colleague. The judges read exactly these words, so say what
"done" means: the behaviour you expect, the cases that matter, what must not
break.

Then, optionally, draw the **navigation map**: the tasks the work splits
into, in order. Each task is judged on its own, and the code's health is
measured when it ends. Leave the map empty and the whole description is one
task. **Suggest tasks** drafts a map from your description; you see it before
it replaces anything.

Pick the judges — correctness, code quality and tests are on by default — and
the session length. The form shows how many judge reviews a session will use.

### 2. Start it in your repository

The project page gives you one line:

```sh
ololo start <your-project>
```

Run it in the folder your agent works in — usually the root of your git
repository. Before anything leaves your machine, ololo shows what the session
will upload and asks:

- it uploads what git tracks or would add — your `.gitignore` rules apply,
  nested ones included;
- `.env` files (not their `.example`), private keys and credential files stay
  on your machine, even when the repository tracks them;
- list anything else to leave out in `.ololoignore`, gitignore syntax, at the
  root.

Answer with `--yes` when you script it. ololo also adds `.ololo/` to your
clone's `.git/info/exclude`, so the platform's own files never end up in your
commits.

### 3. Play it like any session

Paste the task into your agent (**F3** in the ololo app) and let it work. When
a task is done, the agent writes the task's done-file — the brief says which —
and the judges review what that task changed. Code that was there before the
session is context, not your work: it is neither credited nor held against
you.

Health is measured against where each task started, not on an absolute
scale: a task that leaves the code no worse than it found it earns its whole
health bonus, however old the codebase. The chart marks the code you started
from with a square.

### What stays private

Personal projects and their sessions are yours alone. Nobody else can find,
join or watch them, your public profile lists them only to you, and they never
count toward any ranking — there is nobody to compete with in your own
repository. Judge reviews do count toward your monthly allowance.

A project can be edited until its first session. After that its tasks stay as
they were — the results refer to them — and **Duplicate and edit** starts a
new version.
