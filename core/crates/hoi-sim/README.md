# hoi-sim

A harness for **pentesting the hoi protocol** and watching it hold. It drives the real
`hoi-core` — no reimplementation of the rules — so every result is the protocol's own verdict.

Three ways in.

## 1. `hoi-sim battle` — the in-process siege

One process, accelerated time, honest members against adversaries, a scored verdict. No network,
no Docker; good for a quick look and for CI.

```sh
cargo run -p hoi-sim -- battle --honest 4 --attackers 3
```

A founder starts a community and vouches members in; time passes until they're established; then
the attackers:

- **forge** a barrage of malformed records (bad signatures, future/expired timestamps, no stamp,
  oversize, non-canonical bytes, a delegation without the device's consent, a key with no identity
  work) straight at the hub;
- **flood spam** from fresh keys;
- build a **Sybil ring**, vouched in together by one defector, and try to block a real member and
  rewrite the rules.

The scorecard checks the invariants that must hold no matter the tuning — forged records never
reach the feed, spam is contained without a moderator, a Sybil ring shares one voucher's voice
instead of multiplying it, a mob can't block a real member, rules change only by consent,
and honest nodes agree on standing — and prints a PASS/FAIL verdict. `--json`
emits the same as data. The exit code is non-zero if any invariant broke, so CI fails loudly.

## 2. `hoi-sim agent` — one participant against a running hub

A persistent member in `--data DIR`, talking to a hub (`hoi-node`) over the home-node WebSocket.
It **syncs on every call**, does one thing, prints JSON, and exits — so an LLM or a person can run
it as independent steps, and you can run **as many independent sessions as you like**, each its own
`--data` dir and its own identity. This is what the Docker sandbox drives.

```sh
hoi-sim agent --data ./mira  --name mira  init
hoi-sim agent --data ./mira  pair 'hoi://pair?...'      # the hub's pairing link
hoi-sim agent --data ./mira  --hub ws://hub:7842 create privacy
hoi-sim agent --data ./mallory --name mallory init
hoi-sim agent --data ./mallory pair 'hoi://pair?...'
hoi-sim agent --data ./mallory --hub ws://hub:7842 join 'hoi://join?...'
hoi-sim agent --data ./mallory --hub ws://hub:7842 post <community-hex> "title" "text"
hoi-sim agent --data ./mallory --hub ws://hub:7842 vouch <community-hex> <key>
hoi-sim agent --data ./mallory --hub ws://hub:7842 standing <community-hex> <key>
```

Verbs: `init`, `pair`, `create`, `invite`, `join`, `whoami`, `sync`, `feed`, `post`, `comment`,
`vote`, `vouch`, `review`, `standing`, `community`, `propose-limits`, and `forge` (below).

## 3. `hoi-sim agent … forge` — protocol attacks with an honest oracle

```sh
hoi-sim agent --data ./mallory --hub ws://hub:7842 forge bad-sig <community-hex>
```

Each attack builds raw wire records, feeds them to the agent's own node to capture the **exact
stage of the pipeline that refuses them** (the local oracle), and then injects the same bytes at
the hub as an attacker's tool would. Attacks:
`bad-sig`, `future`, `expired`, `unstamped`, `oversize`, `noncanonical`, `dup-flood`,
`no-identity`, `wrong-root-delegation`. The honest network should never show any of them — run the
referee to confirm.

## 4. `hoi-sim referee` — the scoreboard

An honest observer. Syncs and prints community health: who stands where, whose voice is what, what
is napping, who is banned, which proposals are pending. `--watch` reprints every few seconds.

```sh
hoi-sim referee --data ./ref --hub ws://hub:7842 --watch
```

## Not part of a release

`hoi-sim` is a developer tool. It depends on `hoi-core` like any other host and ships no server.
The Docker sandbox in `sandbox/` wires a hub and any number of agent sessions together on a private
network; see `sandbox/README.md`.
