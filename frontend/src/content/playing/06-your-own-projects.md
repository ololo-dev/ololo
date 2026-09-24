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

If the work lives in a git repository others can reach, name it under
**Repository** (and a branch, tag or commit if not the default). Then whoever
starts or joins a session in a folder without it gets it cloned there first.

Then, optionally, draw the **navigation map**: the tasks the work splits
into, in order. Each task is judged on its own, and the code's health is
measured when it ends. Leave the map empty and the whole description is one
task. **Suggest tasks** drafts a map from your description; you see it before
it replaces anything.

Pick the judges — correctness, code quality and tests are on by default — and
the session length. They review every task, unless a task chooses its own:
**Choose judges** on a task gives it a panel of its own, say architecture for
the step that restructures the code, or tests alone for the one that only adds
coverage. Then the session length and who sees the project — see below. The
form shows how many judge reviews a session will use.

### 2. Start it in your repository

The project page gives you one line to copy:

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

If your `README.md` or `AGENTS.md` says how to run the tests, sessions that
score tests run them after each check and count them in the health score
(see [Points and judges](/documentation/points-and-judges)). A task is
compared with where it started on what both ends measured: the session's
first tree is scored before the suite ever ran, so the first task is compared
on the code alone, and every later task on the code and its tests.

Not alone? Give the session's join code to a teammate: `ololo join <code>` in
their own copy of the repository makes them a player, judged on their own work
beside yours — and if the project names its repository, an empty folder is
enough: ololo clones it for them. Only you start sessions of your project.

### Who sees it

A personal project is public unless you keep it private:

- **Public**, the default: your profile lists it, and anyone can open it and
  watch its sessions — the code each task changes included. While a session
  runs, it shows on the landing page like any other. The project itself never
  appears in the catalog.
- **Private**: only you see the project, and a session only whoever you give
  its join code. Where ololo sells plans, keeping a project private is part of
  Premium; a private project stays private if Premium ends.

**Change** under the start command on the project's page switches it, even
after it has been played. Either way only you start its sessions, and they never
count toward the Arena or any rating. Judge reviews count toward each player's
monthly allowance.

A project can be edited until its first session. After that its tasks stay as
they were — the results refer to them — and **Duplicate and edit** starts a
new version.
