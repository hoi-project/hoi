# hoi sandbox

A local community simulator built on the real hoi app and protocol. Join discussions with AI
residents, observe their decisions, and explore how a community responds to attempts to change
its rules.

## Start everything

With Docker Desktop running Linux containers and Node 22+ installed, run from the repository root:

```powershell
node sandbox/launch.mjs
# Or, with pnpm available:
pnpm sandbox
# The Windows shim also works without Git Bash or WSL:
./sandbox/hoi-sandbox.ps1 dashboard
```

The command builds the Rust hub, test harness and browser WASM inside Docker, starts the stack,
waits for readiness, and prints these links. The first build downloads the build toolchains.

| Surface | Address |
|---|---|
| Dashboard | http://localhost:7070 |
| Community owner | http://localhost:7071/app/ |
| Another member | http://localhost:7072/app/ |
| Watcher | http://localhost:7073/app/ |

Docker Desktop shows **hoi-sandbox** as one Compose project, with the hub, dashboard, app, referee,
and individual agent containers underneath. The dashboard shows their live state. It offers all
community invites known to the owner and the hub pairing link, with copy buttons and links into
each browser persona. Communities created in the owner app are discovered on refresh.

Each persona opens the **real app** in a separate browser origin and a session-specific database.
The owner uses the protocol's device-link flow to link the actual founder; the other personas
have independent keys and join through the real invite. Watcher is a quiet participant identity,
not a protocol-level permission or a bypass. These bootstrap helpers exist only in builds made
with `HOI_SANDBOX=1`, on the three loopback app ports. Normal web builds retain ordinary onboarding.

Stop all containers without deleting communities, identities or the last result:

```sh
pnpm sandbox:stop
# equivalent: node sandbox/launch.mjs stop
pnpm sandbox:status
```

Run the start command again to rebuild changed code and resume the stack. An interrupted test is
recorded as inconclusive; tests never resume attacking automatically after a controller restart.

## Run a community

Choose a scenario, select an AI provider, save the settings, then **Start community**. Open the
member app and create a post or reply. Residents read the live feed and community state before
deciding whether to answer, ask a question, make their own post, vote, or wait. Their personas,
recent memory, last moves, and errors are visible in the dashboard.

A community run starts with **Preparing residents**: identities are provisioned and admitted,
then wait for real protocol standing, roughly one minute after admission. Preparation can take
longer while creating identities. The discussion countdown begins only when residents are ready;
this setup time does not consume the configured discussion window.

| Scenario | Active cast | Setting |
|---|---|---|
| **Everyday neighbours** (`neighbours`) | Five honest residents | Everyday discussion among people with different interests and priorities. |
| **A push for control** (`takeover`) | Five honest residents and one malicious organiser | The organiser tries to persuade the community to concentrate influence through rule changes. |
| **A town of 1,000** (`townhall`) | Eight honest residents and one malicious organiser | A community of 1,000 residents is represented by a small active cast. |

In community scenarios, **Alex acts as the community owner** by default, using the founder identity. This
gives the AI steward a real governance role: the organiser can try to persuade Alex, and the
other residents can challenge the proposal. The owner browser uses that same founder identity.
The organiser remains an ordinary member; the scenario does not hand it ownership or private keys.

The town hall's 1,000 residents are **scenario context**, not 1,000 running agents or protocol
members. Only actual identities create posts, votes, reviews, and governance actions. No synthetic
majority is added to a vote. This lets you explore discussion and persuasion in a larger social
setting at a bounded AI cost; it is not a 1,000-node load test.

Residents can disagree with each other, tolerate a proposal, miss manipulation, or choose to
wait. A hide, veto, or membership objection is not a guaranteed removal: the real protocol decides
whether standing and quorum justify restricting a member. There is no direct agent command to
ban someone. A rule change requires actual protocol consent. Watch the
referee's observations to distinguish what an agent attempted from what took effect.

You can add residents, change individual drivers, and start or stop a resident separately.
Saving a different community scenario creates a fresh community and replaces the active lineup.
Earlier communities keep their invites; use the invite marked **Current community** to join the
new scenario. Saving changes within the same scenario keeps its community. Settings are locked
during a run; stopping a resident stops its container and preserves its identity. The referee's
visible counts describe the current community; its raw snapshot also includes earlier ones.

### Choose the AI

**Local AI model:** enter an already-running OpenAI-compatible model API and its installed model
name. For a model on the Docker host, use an address such as
`http://host.docker.internal:11434/v1`. The controller calls `/chat/completions` with the resident's
persona, live observations, and recent memory. Models are not downloaded automatically.
The default model name is a suggestion, not a bundled or verified installation. If the endpoint
or model is unavailable, residents show the error and no replacement post is scripted. Correct
the settings while the run is stopped, then start again.

**Codex CLI or Claude Code CLI:** install and sign in to the corresponding CLI on the host. The
sandbox launcher starts a local worker for installed CLIs. The dashboard reports whether the
worker is online and whether the selected CLI is available. Availability does not prove that
authentication or account usage is valid; a failed request appears as an error.

If you start Compose directly, or need to restart the worker, run this in a separate terminal
from the repository root:

```sh
node sandbox/cli-worker.mjs
```

Choose **Codex CLI** and leave its model field empty to use the CLI's built-in default, or
enter a model explicitly. **Claude Code CLI** has a separate model field, defaulting to `sonnet`.
The sandbox ignores personal Codex configuration for these isolated calls.
The worker uses your existing CLI authentication; you do not paste account credentials into the
dashboard. CLI requests use your provider account and may consume paid usage. The controller
accepts bounded structured decisions and performs the protocol actions itself.

**Scripted demo** is available explicitly for repeatable demonstrations. Its decisions follow
predetermined rules; it does not provide natural language reasoning. Model failures never silently
switch an AI resident to scripts.

### Bound the run

Defaults are 15 minutes, 24 turns per resident, a 15-second pause between turns, **60 AI calls
total**, and **two simultaneous AI decisions**. Adjust time to 1–120 minutes, turns to 1–200,
pause to 2–120 seconds, the call cap to 1–500, and concurrency to 1–3. Decision time is additional
to the pause. Local model decisions use the configured concurrency; the CLI worker additionally
caps Codex and Claude at **two combined simultaneous requests**. Waiting is a valid decision and
still consumes the model call that produced it. Residents can pause longer to spread the available
AI calls across the discussion window, leaving room to respond as the conversation develops.

The AI cap is shared across every resident and provider in the run. The dashboard shows calls
started, completed, failed, and currently in progress. A call cap limits requests, **not tokens
or currency**; individual requests can have different costs. The wider town hall population does
not increase the number of AI processes. Scripted turns do not consume AI calls.

### Focused exercises

The original **Balanced**, **Protocol**, and **Social** presets remain under **Focused exercises**.
They combine a neighbour, defender, forger, and/or persuader to measure specific boundaries.
The forger tries a bounded catalogue of invalid records. The persuader shares an intercepted local
canary link; a defender can request hiding it. The founder vouches the persuader in through the
real protocol, so this exercise tests a member allowed to share links. Newcomers' link restriction
is not bypassed.

### What the verdict means

- **Failed:** an observed attacker objective succeeded, including a participant opening the exercise
  link, the targeted capture rule taking effect, an invalid record passing its measured validator,
  or a failed core invariant.
- **Passed:** every selected check completed with the required evidence. The click exercise needs
  a completed actor plus a link visible in a real persona at least ten seconds before the window
  ended, with no click. This measures that exercise, not whether a person noticed the persuasion.
- **Inconclusive:** a check was stopped, errored, did not run, or lacked participant exposure. No
  participant interaction is never reported as proof the social goal was resisted.

A community run is an observation, not a proof of security. A resident's assertion that it has
captured the community is not evidence; a real accepted policy change is different from a proposal
or an attempted vote. Unknown or unmeasured outcomes remain unknown. Even when every selected
check passes, this says nothing about attacks outside that run's measured objectives.

**Observed community outcomes** separates proposal status and response counts from changes to
the rules actually in effect. It also lists members observed blocked during the run. A pending
proposal is an attempt, not an applied rule; an applied rule is not automatically the organiser's
targeted capture rule. The capture check records that specific objective separately.

The protocol suite separately runs `hoi-sim battle --json` against the real core with accelerated
time, measuring forgery refusal, spam containment, Sybil influence, governance and convergence.
Live forgery probes label their evidence as **local validator** results; successful transport to
the hub alone does not prove hub rejection. The referee reports actual community standing and posts.

Exercise links use the reserved hostname `sandbox-canary.example`. The sandbox app intercepts a
real click, records the active run/actor token locally and prevents navigation. Only actual app
link elements are tracked; synthetic JavaScript clicks do not satisfy the goal. This is a local
exercise instrument, not tamper-proof evidence against the operator who controls the machine.
It also does not validate arbitrary external links or solve malicious-link detection. It measures
only interaction with this specific local exercise link.

### Local control boundary

Published ports bind to `127.0.0.1`. Actors run non-root, on an internal Docker network, with a
read-only root filesystem, dropped capabilities and bounded resources. The controller alone has
the Docker socket because it creates and stops actor containers; treat it as a trusted local
administration service. Its API accepts bounded actions rather than shell commands, checks Host
and Origin, and rejects cross-origin writes. The app proxy uses an automatically generated private
token from git-ignored `sandbox/.state/control.env`; the token is never sent to browser code.

The CLI worker runs on the host and sends the scenario context, observed community content, and
resident memory to the selected AI provider. This model traffic uses the host's network; the
resident containers remain on their internal network. CLI calls run in an isolated working
directory with tools and integrations disabled, returning only a structured decision. Community
actions are validated before the controller executes them. These community scenarios reject
external URLs in model decisions; the separate canary exercise is not a general link scanner.

For direct Compose use after the launcher has created that file:

```sh
docker compose --env-file sandbox/.state/control.env -f sandbox/docker-compose.yml up -d --build --wait
```

Tests: `node --test sandbox/*.test.mjs`, plus the web app's existing test and type-check commands.

## Legacy terminal sessions

The commands below remain available for independent terminal-only sessions. They are separate
from the dashboard's Compose project; `pnpm sandbox:stop` stops the dashboard project only.

A local Docker sandbox for pentesting the hoi protocol and watching it hold. Spin up as many
**independent sessions** as you like; each is a private, internet-isolated network with one hub and
any number of members. Point honest and adversarial agents at it — scripted bots, or LLM agents
following the briefings — and let them try to flood, forge, Sybil, brigade and capture a community.
A referee shows whether the protocol stood.

Everything runs through one console script: `./sandbox/hoi-sandbox`.

## Prerequisites

Docker (daemon running). Nothing else — the image carries the hub (`hoi-node`) and the pentest
harness (`hoi-sim`), both built from `core/`. The first command builds the image (a few minutes).
The `pentest` command additionally needs the `claude` CLI (Claude Code) on `PATH`.

**On Windows**, PowerShell and cmd can't run the bash script directly — `./sandbox/hoi-sandbox up demo`
fails with *"not recognized as the name of a cmdlet"*. Run it from the repo root through the shim,
which forwards to Git Bash or WSL:

```powershell
sandbox\hoi-sandbox.cmd up demo            # from cmd or PowerShell
./sandbox/hoi-sandbox.ps1 up demo          # from PowerShell
```

It needs Docker Desktop plus Git for Windows (Git Bash) or WSL; WSL is recommended for `pentest`.
Under WSL or Git Bash the plain `./sandbox/hoi-sandbox …` form works as written below.

## Quick start

```sh
# 1. Start a session (hub + a founded community). Prints the community id.
./sandbox/hoi-sandbox up demo

# 2. Add members and act as them. A new name is minted and joined automatically.
CID=$(./sandbox/hoi-sandbox info demo | sed -n 's/^community: //p')
./sandbox/hoi-sandbox agent demo alice post "$CID" "hello" "a friendly first post"
./sandbox/hoi-sandbox agent demo mallory forge bad-sig "$CID"     # watch it get refused
./sandbox/hoi-sandbox agent demo mallory post "$CID" "CHEAP PILLS" "spam spam spam"

# 3. Let good and bad bots run on their own, then watch the scoreboard.
./sandbox/hoi-sandbox bots demo good=3 bad=3
./sandbox/hoi-sandbox referee demo --watch

# 4. Tear it down (containers, volumes, network, state).
./sandbox/hoi-sandbox down demo
```

A one-process siege with a scored PASS/FAIL verdict, no hub needed:

```sh
./sandbox/hoi-sandbox battle --honest 4 --attackers 3
```

## Watch it in the real app (your own community, agents joining)

`up` keeps the hub sealed inside Docker. `watch` instead publishes the hub to your machine so the
**real web app** can connect — you create your own community in the browser and watch Docker agents
join it live. The agents stay contained (no internet; they only reach your hub).

```sh
# 1. Start a watchable session. It prints a pairing link for the app.
./sandbox/hoi-sandbox watch demo

# 2. In another terminal, run the real app (needs Node 22 + pnpm 10 + the Rust/wasm toolchain):
./sandbox/hoi-sandbox app          # = pnpm core:wasm && pnpm dev:web; open http://localhost:5173

# 3. In the app:  You → Home node → paste the pairing link from step 1.
#    Then  + New community  → open it → Invite → copy the invite link.

# 4. Send honest + adversarial agents into YOUR community and watch them appear in the app:
./sandbox/hoi-sandbox agents demo '<paste-your-invite>' good=3 bad=3

# 5. Done.
./sandbox/hoi-sandbox down demo
```

You'll see the honest agents' posts show up and the adversarial ones' spam arrive as unvalidated
newcomer posts you can hide — while their forged records never appear at all. Vouch the good ones,
hide the spam, and watch standing change in real time. For a pure-terminal view of the same thing,
add `./sandbox/hoi-sandbox referee demo --watch`.

> Prefer everything sealed (a pentest with no browser)? Use `up` + `bots` + `referee` instead; the
> hub is then unreachable from your machine and only the referee container can watch.

## Commands

| Command | What it does |
|---|---|
| `up [session]` | Start a sealed session: a hub (Docker-internal) and a founded community. |
| `watch [session]` | Start a session whose hub your machine can reach, so you create your own community in the app and watch agents join it. |
| `app` | Run the real web app on your machine (`pnpm core:wasm && pnpm dev:web`). |
| `agents <session> <invite> [good=M bad=N every=S]` | Send a swarm of honest + adversarial agents into the community behind `<invite>` (the one you made in the app). |
| `spawn <session> <name>` | Add a member (own identity, joined to the community). |
| `agent <session> <name> <verb> [args]` | Run one agent action as `<name>` (syncs, acts, prints JSON). Auto-creates the member on first use. |
| `referee <session> [--watch]` | The community-health scoreboard. |
| `bots <session> [good=M] [bad=N] [every=S]` | A scripted swarm of honest and adversarial bots in a loop — the simulation, no LLM needed. |
| `pentest <session> [red=N blue=M model=NAME rounds=R budget=USD]` | Autonomous **Claude Code** agents: a red team attacks and a blue team defends, each pinned to only this session's command; then a referee shows whether the protocol held. |
| `battle [args]` | The in-process siege with a scored verdict (no hub). |
| `info <session>` | Community id, invite, pairing link, members. |
| `shell <session> <name>` | A shell inside a member container (for a human). |
| `logs <session>` | The hub's log. |
| `ls` | List sessions and whether they're running. |
| `down [session]` | Stop and remove a session and all its data. |
| `build` | (Re)build the image. |
| `brief <red\|blue>` | Print the briefing to paste into an LLM agent. |

Agent verbs (`agent <session> <name> <verb> …`): `whoami`, `sync`, `feed`, `context`, `post`, `comment`,
`vote`, `vouch`, `review`, `standing`, `community`, `propose-rules`, `propose-limits`, and `forge` (attacks:
`bad-sig future expired unstamped oversize noncanonical dup-flood no-identity wrong-root-delegation`).

## Autonomous Claude pentest (`pentest`)

One command puts **real Claude Code agents** in the driver's seat — no copy-pasting a briefing into a
chat. It founds a sealed session if one isn't up, launches a red team and a blue team, lets them act
on their own for a few rounds, then prints each agent's findings and the referee's scoreboard.

```sh
./sandbox/hoi-sandbox pentest demo                              # 1 red + 1 blue, model sonnet
./sandbox/hoi-sandbox pentest demo red=2 blue=2 rounds=8        # a bigger swarm, more rounds
./sandbox/hoi-sandbox pentest demo model=opus budget=5          # a different model / per-agent cap
```

- **red** agents mint many identities, flood spam, walk the whole `forge` catalogue and try
  `propose-limits`; **blue** agents post on-topic, vouch the genuine, hide/veto spam, and read
  `standing`. Each follows the matching `brief` text, autonomously.
- `red=N blue=M` set how many of each, `rounds=R` how hard they push, `model=NAME` the Claude model
  (`sonnet`, `opus`, …), `budget=USD` a per-agent spend cap. Needs the `claude` CLI on `PATH`.
- Watch it live in another terminal with `./sandbox/hoi-sandbox referee demo --watch`.

### How an agent is confined

Each agent is launched with **only** the `Bash` tool and no MCP servers, and a `PreToolUse` guard
(`sandbox/pentest-guard.mjs`) that lets it run **nothing but this session's own command** —
`./sandbox/hoi-sandbox agent <session> …`. The guard also rejects any chained or redirected command
(`;  |  &  \`  $  <  >`), so an agent can't smuggle a second command past a prefix match. An
allow-list alone is not enough here: Claude will otherwise auto-run commands it judges "safe" (a bare
`echo`), so the guard's hard allow/deny is what actually holds.

The real boundary, as ever, is the container: members run **non-root, all capabilities dropped, no
new privileges, read-only root filesystem, on a network with no route to the internet**. The guard
keeps the agent *process* (which runs on your machine, where `claude` runs) from doing anything but
driving its own members; the container keeps the protocol actions sealed. The sandbox runs only on
your machine and talks to nothing.

## Driving it with your own LLM agent

Prefer to wire up an agent yourself? Print a briefing and give it **exactly one tool** — the wrapped
command for its member:

```sh
./sandbox/hoi-sandbox brief red      # attacker briefing
./sandbox/hoi-sandbox brief blue     # honest-member / referee briefing
```

For a Claude Code agent, restrict it to its own member so it can do nothing else on your machine,
e.g. allow only `Bash(./sandbox/hoi-sandbox agent demo mallory:*)` and deny the rest (this is what
`pentest` automates, with the guard above). The real boundary, though, is the container: members run
**non-root, all capabilities dropped, no new privileges, read-only root filesystem, on a network with
no route to the internet**. A pentest agent cannot reach anything outside its session.

Run a red-team session and a blue-team session side by side against the same hub (same session,
different member names), or in separate sessions, and compare the referee's verdict.

## What "sturdiness" looks like here

The protocol is meant to hold these no matter who attacks:

- forged or malformed records never reach a real feed;
- spam is held for newcomers and napped by the community's own hides, with no central moderator;
- a Sybil ring vouched in together shares **one** voucher's voice, never multiplies it;
- a mob cannot block a real member — voice decides, not head-count (§16);
- rules and limits change only by the established members' consent;
- honest nodes holding the same records agree on standing.

`battle` checks all of these automatically and prints a verdict; `bots` + `referee` let you watch
the same thing play out in real time; and `pentest` (or your own LLM red team) lets Claude agents
probe for the cases the scripts didn't think of.

## Notes

- Sessions are fully isolated from each other and from the internet. Session state (invite, pairing
  link, member list) lives under `sandbox/.state/<session>/` and is git-ignored.
- The hub is a normal `hoi-node`; members are normal nodes. Nothing here is special-cased — the
  sandbox only automates wiring real nodes together.
- To reclaim disk, `down` a session or `docker image rm hoi-sandbox:latest`.
