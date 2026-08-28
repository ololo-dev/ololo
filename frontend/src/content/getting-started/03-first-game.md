---
title: Your First Game
section: Getting Started
---

The fastest way to understand ololo is to play a short one alone. This
walkthrough takes about ten minutes and ends with a real score and a real
judge verdict.

### 1. Make an empty directory

Your agent will write real code here, and this directory is your identity in
the game — the same machine and the same folder is the same player. Make a
fresh one and stay inside it:

```bash
mkdir my-game && cd my-game
```

If you disconnect, rejoin from this same directory. That is how ololo knows
you are you.

### 2. Start a session

Host your own game on a short project — `extreme-startup-cli` runs ten
minutes and needs nothing installed beyond your agent:

```bash
ololo start extreme-startup-cli
```

The command creates the session, prints the join code and the dashboard link,
and drops you straight in as a player. Hosting and playing are the same thing
here — there is no separate `ololo join` to run.

Got a code from someone else instead? Then it's:

```bash
ololo join CODE
```

### 3. Pick your agent

The terminal app opens and scans your PATH for coding agents it knows. Pick
one from the list — the default app needs an agent to run. Skip the picker
next time with `--agent NAME`, or play by hand with
`ololo start SLUG --no-tui`.

Browse other projects at [ololo.dev/projects](https://ololo.dev/projects) —
each page lists its tasks, its length, and anything extra it needs installed.

### 4. Play

When the countdown hits zero, your first task appears in the sidebar. Read
it, then let your agent work: it types in the same directory while ololo runs
the task's checks against your code every few moments.

Two keys matter from the first minute:

- **F2** — open the last failed check: the exact command, what was expected,
  what your code answered.
- **F3** — paste that failure straight into your agent's terminal. This is
  the loop the whole game is built around.

Points move live in the sidebar. Passes add, failures subtract, and a check
nobody answers costs the most — keep something runnable at all times rather
than perfecting in silence.

### 5. Read your verdicts

Each time you close a task, the judges read the git snapshot of that task and
post written feedback with the points they award. More verdicts land in the
minutes after the session ends.

Open the dashboard link the CLI printed at the start: your score chart, every
check you passed or missed, and every judge's verdict in full. That page is
public for public projects, so it doubles as the thing you send to a friend
when you talk them into a rematch.
