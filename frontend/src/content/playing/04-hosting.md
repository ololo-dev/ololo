---
title: Hosting a Session
section: Playing the Game
---

Anyone with an account can run a game — for two friends, for a Discord
channel, or as a public event. Hosting takes one command.

### 1. Pick a project to fit your slot

The project decides how long the session runs and what players need on their
machines. Every project page lists both.

| If you have   | Take                                                                                           |
| ------------- | ---------------------------------------------------------------------------------------------- |
| 10 minutes    | a short code challenge — `extreme-startup-cli`, `fizzbuzz`, `hop-hop`, `repeat-each-character` |
| 15–20 minutes | `reinvent-the-wheel-ls`, `extreme-startup` (HTTP)                                              |
| 30 minutes    | a build-a-product project — `tetris`, `weather-widget`                                         |

Short challenges make the better spectator sport: checks fire every 10–90
seconds and the leaderboard never stops moving. Build-a-product projects are
quieter to watch — most of the action happens in each player's editor — but
they end with something you can actually show: a working app, screenshots and
screencasts in the report.

Some projects ask for extra tooling — Node and npm, agent-browser, jscpd.
Tell your players before the day, not at the countdown.

### 2. Tell players what to bring

Send this list with the invitation:

- an account at [ololo.dev](https://ololo.dev), and the CLI
  [installed](/documentation/installation);
- a coding agent on their PATH (see
  [Supported Agents](/documentation/agents)) — the terminal app needs one;
- `git`, or the judges will have nothing to review;
- an empty directory to play from, and the sense not to use their work laptop
  for a session hosted by a stranger.

### 3. Start the session

```bash
ololo start PROJECT-SLUG --name "Friday night race"
```

The command creates the session, prints the join code and the dashboard link,
and puts you in as a player. Share the code; share the dashboard link with
everyone who only wants to watch.

Keep the field small for a first event. The platform accepts up to 16 players
in one session, but a tight field is easier to follow — every row of the
feed still means something.

### 4. Run the game

From the dashboard you can pause, resume, or cancel. A pause freezes the
clock for everyone — that is the tool for a player whose machine died, or for
anything that needs sorting out mid-match.

Watch the activity feed rather than the leaderboard: it shows what is
actually happening — checks passing and failing, tasks closing, judges
returning verdicts with their reasoning in full.

### 5. After the finish

Judges finish within a few minutes of the clock running out, and the session
report settles: final standings, each player's score chart and timeline,
every verdict, and the artifacts — screenshots and screencasts of what was
built, downloadable.

For a public project, that report page stays open to anyone without a login.
It is the thing you post afterwards.

Two things worth planning if you want video: the session page and the report
record well from a browser. A player's terminal does not — ololo never
streams anyone's screen, so if you want that footage, ask the player to
record their own.
