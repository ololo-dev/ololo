---
title: How a Session Works
section: Playing the Game
---

### The lifecycle

A session moves through three phases, shown both in the terminal app's header
and on the web dashboard:

1. **Lobby** — the session exists and players are joining with the code. A
   short countdown runs before the game begins.
2. **Running** — tasks are live and the session clock is ticking. Length is
   set by the project: the short ones last ten minutes, the build-a-product
   ones half an hour. The host can pause, resume, or cancel; a pause freezes
   the clock for everyone.
3. **Finished** — either every player completed every task, or time ran out.
   Scores settle once the judges are done.

### Tasks, one at a time

You always work on exactly one task. Each task has a description, a set of
checks, and its own point values.

There is no skipping ahead: the next task arrives when you finish the current
one, and the server simply never sends a check for a task you haven't
reached. Tasks are ordered by difficulty and by what they pay, so everyone
climbs the same ladder — which is what makes two players' scores comparable
at all.

The one exception is projects with open-ended tasks ("build a working
Tetris"). Those tasks have their own time window, and when it expires the
session moves you on whether you are finished or not.

### Checks: how your work is scored

While a task is active, ololo sends checks — the app calls them probes — that
run against your working directory. Each one:

- runs on your machine, in your directory, as a shell command from the
  platform;
- has a deadline: answer too slowly and it counts as no response;
- scores by outcome — pass, fail, or no response (see
  [Points & Judges](/documentation/points-and-judges)).

The rhythm of checks follows you: passing one immediately speeds the next one
up, while unanswered checks arrive more and more rarely. The faster you
answer, the more chances you get to earn.

Every player gets their own randomized values inside those checks — the same
question with different numbers — so a neighbour's answer is no use to you.

Every check and its outcome is recorded, and you can replay the whole
timeline on the session dashboard afterwards.

### Snapshots: your code is part of the game

ololo keeps a shadow git repository beside your work — your own repo, if you
have one, is never touched. It commits a snapshot when the session starts,
checkpoints while you work, and a commit when each task closes, then pushes
them to the server.

That history is what the judges read, and what the session report shows as
per-task diffs. It is also why playing in a clean directory matters: work
that existed before the session started is exactly what the anti-cheat judges
look for.

### Finishing

- **You finish early.** The app switches to "tasks done" and you stay in the
  session while the others race — your position keeps moving as judges score
  your work.
- **Time runs out mid-task.** Whatever you pushed still counts: the snapshot
  is committed and its judges still review it.
- **After the session.** Your status reads _awaiting judges_ until the last
  verdict lands — usually a couple of minutes, and the platform stops waiting
  after ten and pays out regardless.
