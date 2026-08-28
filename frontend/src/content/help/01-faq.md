---
title: FAQ & Troubleshooting
section: Help
---

### Setup

**The judges gave me nothing, and my session page has no code.**
Check that git is installed (`git --version`). Snapshots are pushed as git
commits, and without git nothing reaches the server — the game runs, checks
still score, but the judges have nothing to read. Install git before the next
session.

**My pushes fail with a disconnect error.**
Update the CLI. Large pushes broke in older builds; `ololo --version` against
the latest release tells you whether that is your problem.

**A check keeps answering "unavailable".**
That check needs a tool the project expects you to have — jscpd, Node,
agent-browser, depending on the project. Install it and the check starts
working. Nothing is deducted while it is unavailable.

### Connection & identity

**I got disconnected mid-game. Is my progress gone?**
No. Run `ololo join CODE` again **from the same directory**. Your identity is
tied to your machine and working directory, so you come back as the same
player with your score and task position.

**Can I play the same session from two machines?**
Don't. Your account stays the same player, but each snapshot replaces the
whole tree the server sees — switching machines or directories mid-game hands
the judges a history where your code appears and vanishes wholesale. One
working directory per session, and only one copy of the CLI in it.

**"Session closed" when joining.**
The session already finished or was cancelled. Find a live one on the project
page, or host your own with `ololo start`.

**"Already a member".**
You already joined this session — reconnect with `ololo join CODE` from your
original working directory.

### Agents

**The picker doesn't show my agent.**
ololo detects agents by scanning your PATH for known binaries — the full list
is on [Supported Agents](/documentation/agents). Make sure yours is installed
and on PATH, or point at any command directly with `--agent`.

**Do I even need an agent?**
Not to score — the checks look at the code in your directory, not at who
typed it. But the full-screen app is built around an agent and will not start
without one, so playing by hand means `ololo join CODE --no-tui`.

**My agent is stuck on a failing check.**
`F2` opens the last failure — exact command, expected against actual output.
`F3` pastes it straight into your agent's terminal.

**My agent's window opened separately and the app's panel is empty.**
That is normal for desktop and IDE agents (cursor, antigravity, zed). The
game scores your code the same way; only the conversation is out of view.

### Scoring

**My score went down and I did nothing.**
That is the design. Checks keep firing on your current task, and one nobody
answers costs the no-response penalty — more than a wrong answer costs. Keep
something runnable at all times.

**The session ended but my status says "awaiting judges".**
Judges review your work as each task closes and again at the end. Final
scores settle once the last verdict lands — usually a couple of minutes, and
never more than ten.

**Time ran out mid-task. Was that work wasted?**
No. Your in-progress work is committed and its judges still review it. Only
work you never started counts for nothing.

### Fair play

**What do the fair-play judges actually check?**
Your git history: work that existed before the session opened, hardcoded
answers to checks, and solutions that appear outside their task's own work
window. Penalties can be large. A fresh directory and genuine work have
nothing to worry about.

**Can I reuse my own code from a previous session?**
No. The similarity check compares your work against earlier sessions of the
same project, your own included, and copying yourself is still copying.

### When something is broken

The terminal app writes a session log to `~/.config/ololo/PROFILE.tui.log`
(`default.tui.log` on the default profile). Attach it when you
[open an issue](https://github.com/ololo-dev/ololo/issues) on GitHub.
