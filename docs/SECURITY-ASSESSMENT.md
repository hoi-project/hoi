# Security assessment

A short map of what hoi protects, who it defends against and where the detail lives. It adds no
new claims: every threat table is in the specification, and where this page and the code differ,
the code, its tests and the conformance vectors are authoritative. hoi is preview software and
has had no independent security audit ([SECURITY.md](../SECURITY.md)).

To report a vulnerability, follow [SECURITY.md](../SECURITY.md).

## What is protected

- **Identity and authorship.** Keys, device delegations and signatures. Records are immutable and
  signed, so a record cannot be altered or attributed to someone else.
- **Community rules and standing.** Who may change rules, vouch, moderate or veto, and how their
  voice is weighed.
- **Private content.** Posts and media of private communities, which are encrypted on members'
  devices.
- **Availability of posts.** Whether a post can still be fetched while it lives.
- **Readers' and members' metadata.** Addresses, timing and which communities a device is in.
- **The software people run.** Releases, dependencies and build pipeline.

## Trust boundaries

| Part | Trusted with | Not trusted with |
| --- | --- | --- |
| Rust core (`core/`) | keys, signatures, validation | nothing else: UI code only passes key material to it |
| Browser app and desktop app | showing checked data | raw key material beyond handing it to the core |
| Public node | storing and serving public records | secrets: it reads every public record it stores and can stop serving any of it |
| Registry (optional) | public names and discovery hints | authority over a community; it can omit an entry or stop, and every answer is in a signed log |
| Private relay (optional) | storing ciphertext, signatures, sizes and expiry | message content and membership keys; it still sees padded sizes, timing and network metadata |
| Scattered holders and forwarders | holding or passing one piece | a whole object: no single party holds enough pieces to read it |
| The client binary and its distribution | everything on the device | nothing: a compromised distribution can expose data and keys |

The website and browser app are static files. Nodes, registries and relays are optional and run
by users; builds ship no default service endpoints.

## Who the attackers are

- A forger, replayer or flooder of records.
- A Sybil: many identities, nodes or places held by one party.
- A malicious or curious node, forwarder, holder, registry or relay operator.
- A member who abuses community powers, or a mob that hides opinions.
- An observer of network traffic.
- A supplier of hostile content: text, links, media, dependencies.
- A thief of a device or a key.

## Threat tables

| Area | Where |
| --- | --- |
| Records, identity, Sybils, voting, spam, content rules, media, moderation, registries, operators, Warmth | [PROTOCOL.md section 19](spec/PROTOCOL.md) |
| Scattered delivery: holders, forwarders, Sybils across networks, poisoned node lists, repairs, traffic observers, storage spam | [SCATTER.md section 11](spec/SCATTER.md) |
| Transport, hostile input, metadata, keystore, strangers, supply chain, media, governance attacks | [TECHNICAL.md section 11](spec/TECHNICAL.md) |
| Dependencies and the checks that guard them | [DEPENDENCIES.md](DEPENDENCIES.md) |

## Residual risks

The specification states these plainly, and they stay open on purpose:

- Recipients can keep copies after posts expire. Nothing stops a screenshot.
- A public node can read the public records it stores.
- A private relay still sees padded sizes, timing and network metadata. It is not an anonymity
  network, and a VPN still matters.
- A patient attacker who pays for identity work and waits still gets voices. The protocol limits
  how fast they grow, it does not stop them.
- A Sybil across many networks holding a large share of places can gather pieces or withhold.
  The sizing bounds the odds for a given share.
- Perceptual hashes catch edited copies of blocked images, not heavy crops, trimmed videos or
  altered audio.
- Registry discovery depends on its operator.
- A compromised client distribution defeats the rest.

The full list is in PROTOCOL.md section 19 and SECURITY.md.

## Out of scope

- Anonymity against an observer of all traffic.
- Preventing a recipient from keeping what they were sent.
- Protecting a device that is already compromised.
- The operator of a node, registry or relay: hoi gives their users ways to check and replace
  them, it does not make them trustworthy.

## How this is kept current

A protocol change needs an accepted RFC, core tests and conformance vectors. Scheduled and
per-pull-request scans (CodeQL, dependency and workflow checks) are listed in [CI.md](CI.md).
