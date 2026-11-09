# Scattered delivery: no single node holds or hands over a whole post

**Completion scope, 2026-11-06:** the concrete follow-ups are complete (see "Completion
follow-ups" at the end). Section 16's research questions are not all implementation commitments; the outside
cryptography review remains open.

**Status:** S1–S7 and the concrete completion follow-ups are implemented. The development
sequence began on 2026-11-04 (sizing, repair, deletion, candidates and wording revised
the same day; the repair trigger later): the measuring stick, Warmth
counting copies by independent place and by checked links, which is what placement below
is chosen with; phase S1, the pieces library, exact sizing, envelopes, the manifest, vectors and
fuzz targets; most of S2: `Limits.scatter`, holders on public nodes (`hoi-node --hold-pieces`) and
on desktops that opt in, the wire messages, placement under the rules of 5.7 with first-hand and
vouched-for holders, the placement simulation (`hoi-sim scatter`), and mailboxes for private
groups (5.13) with a bell per epoch, tested end to end with real nodes and browsers
(`scripts/e2e-mailboxes.mjs`); and most of S3: forwarding by every public node, two-hop
store and load with no forwarder place carrying k indexes of a generation, the object registry
with checked pointers and updates, probes with the two-place loss rule, budgeted repair,
regeneration and hot regeneration on members' durable devices, private groups' posts and media
sent in pieces with pointers through their mailboxes, and the lifetime simulation
(`hoi-sim scatter --life`), then the rest of S3: one object per post, drops by authors
and, for removed posts, by the group's drop key, mailboxes reached through forwarders, and
home-node rebuild (section 10); S4, members-only communities as MLS groups that follow their
records, with several committers, drop keys for all of them, a clean break from older apps and
invitations newcomers pick up themselves; S5, open communities in pieces,
on by default for new ones; S6, the load simulation to 200,000 online and what it
showed fixed, holders on by default and the notices' new words (the outside
cryptography review is open); and S7, uploads in three
hops and an injector for a device's own records. This document is the
implementation plan, the security target and how it is met, the setting communities get, the
experience it gives people, and what it costs at a million people with 200,000 online. The
arithmetic is reproducible: `node docs/spec/scatter-params.mjs` prints every table in sections
3, 5.4 and 6 with exact integer arithmetic; `hoi-crypto/src/scatter/size.rs` and
`core/vectors/scatter.json` agree with it.

**In one sentence.** A post is sealed on the author's device, cut into pieces so that any k of n
rebuild it, and each piece travels by its own path to its own independent place; k and n are
chosen so that **a party running a tenth of the places pieces can go to cannot rebuild or stop a
post** (with certainty in a small network, and for fewer than one post in a million over its whole
life in a large one); a reader fetches a few more than k pieces through different nodes and puts
the post back together on their own device. No node can read a piece, no node holds more than one
piece of a post, no node can ask for the other pieces, and no node hands a reader a whole post.

### What changed from the first draft

| # | First draft | Problem | Now |
|---|---|---|---|
| 1 | k = 3…6 by size, n = 6…16 | a party holding 10% of places rebuilds 1.6% of posts (3 of 6) | k and n chosen from a security level: 15 of 28 in a large network at the default, section 5.4 |
| 2 | repair re-places lost pieces on new holders | every re-placement is a new draw: with each index placed 5 times over a post's life, a 10% party holds a rebuildable set of **48%** of posts | each *generation* places every index once, in a place that never held one of its siblings, within a fixed budget; then a fresh generation that cannot be combined with the old one (5.5) |
| 3 | delete tokens derived from the object key | in open communities anyone who joined, in sealed ones any member, could drop any post | drops need the author's own secret or the community's drop key; a drop without a ledger removal is repaired like a loss (5.11) |
| 4 | locators keyed by the community key (open) | a holder that joined could compute every sibling's locator | locators keyed by a per-generation secret that only the sealed manifest carries (5.1) |
| 5 | object keys from the MLS epoch exporter | members who join later cannot derive old epochs; repair across epochs breaks | random per-object keys carried in the sealed manifest, which rides MLS like any record (5.1) |
| 6 | weights by age and uptime; candidates from any node list | an always-on attacker weighs more than honest nodes; one node's list can fill a post's holders with its own | flat weights above the bar; candidates first-hand or corroborated by two introducers in different places; caps per introducer and per /16 (5.7) |
| 7 | repairers publish new holder hints freely | a member could "repair" a post onto one party's holders and withhold it | update rules: established members only, same sealed object, plausible placement, one regeneration a day, older generations kept as fallback (5.10) |
| 8 | hot repair adds pieces | more pieces at the same k raise the share a party collects | hot posts regenerate with a larger k and n under the same risk budget (5.10) |
| 9 | "No single node sees who posted here" (open, S5) | false: the node a device connects through sees its address and its pointer record | the words say what holds; address privacy is a phase of its own (S7) and outside the 10% target (section 3) |
| 10 | requests padded to the piece size | doubles read traffic; direction and volume show anyway | fixed-size requests (5.8) |
| 11 | "Use less data: fewer pieces" | a device setting that lowers everyone's protection | fewer spare pieces and the home node only; never fewer pieces or places (section 6) |
| 12 | holders chosen only where the device accepted their notice | a browser would answer dozens of notices; holders never see the device | the consent gate guards direct links (forwarders); declined nodes are never used as holders either (5.7) |
| 13 | floats in the sizing | hosts round differently; manifests would be valid on one and not another | exact integer arithmetic and vectors (5.4, section 8) |

---

## 1. Why: what changes for trust

Today:

| | Open community | Members-only (approve, invite) | Private group (MLS) |
|---|---|---|---|
| Who can read a post | anyone who joins | **every node that holds it** (not encrypted) | members only |
| Where it is stored | whole, on every carrier and member node | whole, on every member's node | whole ciphertext, on each relay |
| Who can withhold it | each carrier, for whoever reaches it | each member node | each relay |
| Who sees which address posted it | the first node it reaches | the first node it reaches | each relay |
| What a node operator could hand over | full readable copies | full readable copies | ciphertext and metadata |
| What a party running 10% of nodes gets | a whole copy of 27% of posts (one of three carriers) | same | each relay it runs |

With scattered delivery at the default level:

| | Open community | Members-only (after S4) | Private group |
|---|---|---|---|
| Who can read a post | anyone who joins (it is public) | members only | members only |
| Where it is stored | in pieces, at most one piece per independent place | same | same |
| Who can withhold it | nobody alone; a party needs more than a tenth of the places, and repair moves the post | same | same |
| Who sees which address posted it | the node the author connects through, as today (until S7); holders and readers do not | same, and that node cannot read what it passes | same |
| What an operator could hand over | one unreadable piece per post, no way to find the others | same | same |
| What a party running 10% of places gets | a rebuildable set of fewer than one post in a million, over the post's life (still locked for members-only and private) | same | same |

**The honest limits.** An open community is public: anyone who joins reads every post, a node
that joins included, so scattering there protects *who can stop a post* and *what a storing node
carries*, not the words. The share a party needs is a share of the places that pieces can go to,
as devices see them: in a young network a tenth of the places is a handful of nodes, cheap to
run. And the address a device posts from is seen by the node it connects through, until S7; the
10% target is about pieces, not about addresses. The app says only what holds for the community in
front of the reader (section 12).

---

## 2. The path of one post

```
 author's device                      the network                                    reader's device
 ───────────────                      ───────────                                    ───────────────
 record (signed) ─┐
                  ├─ seal with a random object key (5.2)
                  ├─ pad to a size class
                  ├─ all-or-nothing transform with a fresh key per generation (AONT-RS)
                  └─ Reed–Solomon: n pieces, any k rebuild (15 of 28 at the default level)
                       │
  pieces  1–9  ──► forwarder A (place a) ──► 9 holders, each in a place of its own  ◄── forwarder X ◄─┐
  pieces 10–18 ──► forwarder B (place b) ──► 9 holders                              ◄── forwarder Y ◄─┤ k+2 pieces,
  pieces 19–28 ──► forwarder C (place c) ──► 10 holders                             ◄── forwarder X ◄─┤ ≤ k−1 per
                                                                                                       │ forwarder
 pointer record (sealed manifest: generation, k, n, piece hashes, holder ids) ── normal sync ──►  rebuild,
                                                                                                  check, show
```

Each forwarder carries at most k−1 pieces of one generation, each holder holds one, and every
holder is in a place that holds no other piece of that generation, ever (one network, one
operator: one place, as Warmth counts them). Forwarders see envelopes sealed to holders; holders
see a piece but not who sent it; readers' requests go through forwarders too, so a holder does not
see who reads.

---

## 3. The security target: a tenth of all places

**The target, the same everywhere.** A party that runs a share ℓ of the independent places a
device chooses holders from may, for any one object over its whole life:

- hold k or more pieces of one generation (enough to rebuild it) with probability at most **10⁻⁶**;
- hold n−k+1 or more live pieces of one generation (enough to withhold it until repair) with
  probability at most 10⁻⁶;

while the object stays readable with probability ≥ 0.999 per repair interval when holders are up
80% of the time. ℓ is the community's **level**: 10% by default and at least (section 6).

**Why a share and not a number of nodes.** Placement is random over places, so no share is ever
"enough" or "not enough" with certainty in a large network; what can be bounded is how often a
party gets lucky. One piece per place per generation adds a hard bound on top: a party with fewer
than k places can never rebuild, whatever its luck.

**Two regimes, one rule.** The device knows how many independent places N clear the bar (5.7). It
assumes the party holds ⌊ℓ·N⌋ of them and computes the exact hypergeometric risk (binomial, which
is slightly stricter, above N = 1000):

- **Small networks** (N up to about 90 at 10%): k comes out larger than ⌊N/10⌋, so a party with a
  tenth of the places **cannot** rebuild or withhold a post at all. This is the "more than 10%"
  requirement met with certainty. Below 12 places nothing is scattered.
- **Large networks**: the bound is probabilistic. At the default, 15 of 28: a party with 10% of the
  places rebuilds a given post's generation with probability 4.6·10⁻⁷ and withholds it with
  1.0·10⁻⁷; with 5% the figures are 3.8·10⁻¹¹ and 1.3·10⁻¹¹. It cannot choose which posts: placement
  is random and its locators are secret.

| Share a party runs | 15 of 28 (default): rebuild | withhold | First draft 3 of 6 | Today: one of three whole copies |
|---|---|---|---|---|
| 5% | 3.8·10⁻¹¹ | 1.3·10⁻¹¹ | 2.2·10⁻³ | 14% |
| 10% | 4.6·10⁻⁷ | 1.0·10⁻⁷ | 1.6·10⁻² (48% with repair, row 2 above) | 27% |
| 15% | 7.0·10⁻⁵ | 1.4·10⁻⁵ | 4.7·10⁻² | 39% |
| 20% | 1.8·10⁻³ | 3.7·10⁻⁴ | 9.9·10⁻² | 49% |

**Over a whole life.** Repair regenerates (5.5); each generation is a fresh draw. Generation g may
spend at most 10⁻⁶ / 2^(g+1), so all generations of an object together stay under 10⁻⁶ however
often it is repaired. Later generations are therefore a little larger (5.4).

**What the target does not cover.** The address a device posts or reads from (its first hop sees
it; section 10, S7), a party that is a member (members read by design), a global observer
correlating traffic, and a party that hides a much larger share behind many networks with no name
(a Sybil: then ℓ is simply larger than it looks; section 11).

---

## 4. What it guarantees, and the rule behind each

| # | Property | Rule that gives it |
|---|---|---|
| P1 | No node can read a piece | Pieces are AONT-RS output over a sealed object: fewer than k pieces of a generation reveal nothing, and for members-only and private communities even all n are ciphertext under a key only members get. Nothing is ever decrypted piece by piece. |
| P2 | No party with less than the level's share holds enough to rebuild | One piece per place per generation, over the generation's whole life (5.5); k and n from the level (5.4); flat weights and corroborated candidates (5.7). Forwarders carry at most k−1 pieces of a generation, uploads and fetches alike. |
| P3 | No node can ask for the other pieces | Locators are keyed by a per-generation secret only the sealed manifest carries; holders answer only `get` by exact locator, never a listing; which nodes hold the others is only in the manifest. Repair is done by members' own devices, never by holders. |
| P4 | No single node hands over a whole post | A reader fetches k+2 pieces through at least two forwarders in different places, each carrying at most k−1. Nobody re-serves rebuilt posts in a scattered community; only pieces travel. |
| P5 | A post is rebuilt only by its reader | Rebuilding needs the manifest (and the community's key for sealed tiers): a reader's own devices. A paired home node is the reader's own device and may rebuild for its person's phone (5.12). |
| P6 | Any k of n are enough | Systematic Reed–Solomon; n−k holders may be gone or refuse. |
| P7 | Nobody can swap a piece or a post | Each piece's hash is in the manifest; the sealed object carries its AEAD tag and its hash is fixed in generation 0; the record inside keeps its author signature and stamp, checked as today. Later generations must rebuild to the same sealed object. |
| P8 | It can be taken back, but not by just anyone | A `drop` needs the author's own drop secret or a signature by the community's drop key, which only moderators hold. A drop with no matching removal in the ledger is treated as a loss and repaired. Pieces also expire with the post's lifespan. |
| P9 | Shape gives little away | Objects are padded to size classes; pieces of one class and k are the same size; requests are one fixed size; no piece carries its index; expiry is rounded to the day; uploads leave with jitter. |
| P10 | The bound holds for the whole life | Generation g spends at most 10⁻⁶/2^(g+1); a generation never re-places an index and has a fixed index budget; regeneration uses fresh keys, so pieces of different generations never combine. |

---

## 5. Building blocks

### 5.1 Keys and secrets

| Secret | Made by | Carried in | Purpose |
|---|---|---|---|
| object id | author, 32 random bytes | the manifest | names the object; not secret |
| `K_obj` | author, random | the sealed manifest | seals the padded record (5.2); the same for every generation |
| `K_a,g` | whoever makes generation g, random | nowhere: recovered from k pieces | the all-or-nothing transform of generation g (5.3) |
| `S_g` | whoever makes generation g, random | the sealed manifest | locators of generation g (5.6) |
| author drop secret `A` | the person, once, kept in the vault and shared with their own devices | never sent | `D = BLAKE3_keyed(derive("hoi scatter drop v1", A), loc)`; a `put` carries `BLAKE3(D)` |
| community drop key | in a private group, a committer's device, an Ed25519 key in the mailbox list, its secret sealed to each committer's device (to an X25519 seal key it announces in the group); in an open community (S5), owners, in a `drop-key` ledger entry, sealed to a key each moderator's delegation carries | the mailbox list (S5: the ledger); its id in each `put` | committers' devices sign `drop`s for removed posts (5.11) |
| holder key `xk` | each holder, X25519, rotated monthly | its hello and node entries, signed by its endpoint key | envelopes are sealed to it (5.8) |

The **pointer record** that carries the manifest is an MLS application message for the sealed
tiers (re-sent on epoch changes as other records are, so members who join later get it) and a
signed record sealed under `derive_key("hoi read v1", community key)` for open communities, so nodes
that carry pieces but never joined cannot open it. Random per-object keys, instead of keys derived
from the epoch or the community key, keep old objects readable for later members, let any member
repair any object, and let no locator be computed from anything a holder could know.

### 5.2 Sealing and padding
The record (signed and stamped as today) is padded to a size class (4, 16, 64, 256 KiB, 1, 2 or
4 MiB) and sealed with XChaCha20-Poly1305 under `K_obj` (`hoi_crypto::sealed`
style), the AAD binding `"hoi scatter object v1" ‖ object id ‖ size class`. The
24-byte nonce is synthetic, a keyed hash of the padded content under a key derived from `K_obj`,
so a member who kept the content can seal the very same object again (5.10).
Media blobs are objects of their own; a post's pointer names its media's pointers. Objects are at
most 4 MiB: longer audio and video (up to 8 and 16 MiB, PROTOCOL.md §17) are cut into 4 MiB objects
first, named in order by the post's pointer.

### 5.3 All-or-nothing transform (AONT-RS)
Following Resch and Plank's AONT-RS: a fresh random `K_a,g` encrypts `sealed ‖ 16 zero bytes`
(the canary) with ChaCha20; the last block carries `K_a,g ⊕ BLAKE3("hoi aont v1", ciphertext)`.
Without every byte of the ciphertext `K_a,g` is unknown, and with fewer than k pieces part of the
ciphertext is missing. Even a leaked community key opens nothing from fewer than k pieces, and
open communities get "a piece alone is nothing" without any key. A fresh `K_a,g` per generation
makes pieces of different generations unrelated bytes: holding 10 of generation 0 and 10 of
generation 1 is holding 10 and 10, never 20.

### 5.4 How many pieces: sizing from the level

Run on the device that makes a generation, with exact integer arithmetic (the Rust function and
`core/vectors/scatter.json` must agree with `scatter-params.mjs`):

```
input    level ℓ (10, 15 or 20, from the community's limits), generation g,
         N = candidate places that clear the bar (5.7), not counting this object's forwarders' places
party    A = ⌊ℓ·N / 100⌋ places
budget   ε_g = 10⁻⁶ / 2^(g+1)
for n = 5, 6, … up to min(N, 160), and for each n, k = 3 … n−2:
    M = min(n + ⌈n/4⌉, N)                                   indexes this generation may ever use
    stop raising k once P(Bin(n, 0.8) ≥ k) < 0.999          (availability per repair interval)
    skip k while n > 3·(k − 1)                               (a browser's three links carry it)
    accept the first (k, n) with
        risk(N, A, M, k)         ≤ ε_g                       (rebuild: k of everything ever placed)
        risk(N, A, n, n − k + 1) ≤ ε_g                       (withhold: enough of the live pieces)
risk     hypergeometric tail for N ≤ 1000, binomial with share ℓ above (stricter)
none     N < 12 or no (k, n) fits: the object is not scattered (private: the relay or members'
         nodes as today; open: today's sync), and Warmth reads cooler
```

**Large networks**, the first nine generations (k/n, budget M):

| Level | g0 | g1 | g2 | g3 | g4 | g5 | g6 | g7 | g8 |
|---|---|---|---|---|---|---|---|---|---|
| 10% (default) | **15/28** (35) | 16/29 (37) | 17/31 (39) | 18/33 (42) | 19/35 (44) | 19/35 (44) | 20/37 (47) | 21/39 (49) | 22/40 (50) |
| 15% | **24/44** (55) | 26/47 (59) | 27/49 (62) | 28/51 (64) | 29/53 (67) | 30/55 (69) | 32/59 (74) | 34/62 (78) | 35/64 (80) |
| 20% | **39/71** (89) | 41/75 (94) | 43/79 (99) | 45/83 (104) | 47/86 (108) | 49/90 (113) | 52/95 (119) | 54/99 (124) | 56/103 (129) |

**Generation 0 by places seen** ("hard": the party's ⌊ℓ·N⌋ places are fewer than k, so it cannot
rebuild or withhold at all):

| Places N | 10% | 15% | 20% |
|---|---|---|---|
| under 12 | not scattered | not scattered | not scattered |
| 12–20 | 5/12 (12–15), hard | 5/12 (12–15), hard | 5/12 (12–15), hard |
| 30 | 5/12 (15), hard | 5/12 (15), hard | 7/15 (19), hard |
| 40 | 5/12 (15), hard | 7/15 (19), hard | 9/18 (23), hard |
| 50 | 6/14 (18), hard | 8/17 (22), hard | 11/21 (27), hard |
| 80 | 9/18 (23), hard | 13/24 (30), hard | 17/32 (40), hard |
| 90 | 10/20 (25), hard | 13/24 (30) | 18/34 (43) |
| 100 | 10/20 (25) | 14/26 (33) | 19/35 (44) |
| 200 | 13/24 (30) | 18/33 (42) | 26/48 (60) |
| 1,000 | 15/27 (34) | 23/42 (53) | 35/64 (80) |
| more | 15/28 (35) | 24/44 (55) | 39/71 (89) |

Readability at the default, per repair interval: 0.99963 with holders up 80% of the time,
0.9999999 at 90%, and 0.99979 at 90% while a 10% party withholds everything it holds (three whole
copies today: 0.992 and 0.999). Storage is n/k ≈ 1.87× the padded object. k is never below 5,
so a 4 MiB object's pieces stay under the 1 MiB piece limit. The rule n ≤ 3(k−1) lets a browser's
three links carry any upload with at most k−1 pieces each; it is why the smallest size is 5 of 12
and not 3 of 9.

### 5.5 Generations and the index budget
A generation is one codeword: one `K_a,g`, one `S_g`, indexes 0 … M−1 of a Reed–Solomon code over
GF(2⁸) (at most 255 indexes; M never exceeds 160). Rules:

1. Generation 0 places indexes 0 … n−1. Repair places **fresh indexes** n, n+1, … — never an
   index already placed in this generation — until M are used.
2. Every index of a generation goes to a place that has never held an index of that generation:
   a holder's endpoint and every network and operator key it was seen with count as used.
3. When reachable pieces fall to `k + ⌈k/3⌉` and the budget is spent, the next repair makes
   generation g+1, sized by 5.4 for g+1 and today's level, with fresh `K_a`, `S`, indexes and
   places, and publishes it. The old generation is left to expire (its creator may drop it).
4. Generations are at least 24 hours apart, except when the previous one has fewer than k
   reachable pieces.

With rule 1 and 2, everything a party could ever collect of one generation is a draw of at most
M places without replacement: exactly what 5.4 bounds. With rule 3, generations are independent
draws whose budgets sum to under 10⁻⁶.

### 5.6 Locators
`loc_g,i = BLAKE3_keyed(derive_key("hoi scatter loc v1", S_g), object id ‖ g ‖ i)`, 32 bytes. A
holder keeps a piece under its locator with an expiry (rounded up to the next UTC day), a size,
`BLAKE3(D)` of its creator and the community drop key id. No other index exists on a holder; the
piece index i is not stored or sent, only the manifest knows it.

### 5.7 Candidates and placement
Placement is where the 10% is won or lost: the bound in 5.4 holds only if a party's share among a
device's candidates is no larger than its share of real places.

1. **Who can be a candidate.** A node listing `scatter` and `keeps` in `hello.fx`, a public node or
   an opted-in desktop, with a valid signed `xk`, whose notice this device has not declined.
2. **Known first hand or corroborated.** A candidate counts once this device or the person's own
   devices (a paired home node, their desktop) saw it answer at least 80% of probes over 7 days,
   *or* at least two introducers in different places list it: the device's own links' node lists,
   or holder rosters that established members' durable devices publish in the community, weighed
   by voices as reviews are. Reported uptime is taken as the lowest any introducer
   gives. One node's list alone never makes a candidate.
   *Built in S2:* a device samples its linked holders once an hour while it runs and keeps
   decaying counts (weight 1/168 per sample, about a week); a holder takes pieces once the device
   first saw it at least two days ago, after at least eleven samples, linked in four of five.
   Sightings live in their own store blob (`scatter`), which older releases never read. Node lists
   carry the sender's own first-hand sightings that clear its bar (`nodes.h`: endpoint, share up
   rounded down to 5%, days known up to 30); a linked holder also counts when the person's own
   devices vouch for it, or two dialed links in different places, neither the holder's own, vouch
   with at least this device's bar. It is then counted against the introducer first heard from.
   *Rosters.* A desktop or node whose person turned on "Share the holders I see"
   publishes, in each community where that person is established, a `roster` record naming the
   holders it saw up for days, first hand (endpoints only, at most 32, every twelve hours at most,
   for three days). For an object of that community, a holder listed by the current rosters of at
   least two established people, neither banned nor struck, whose voices together reach the
   community's hiding quorum, counts as one more introducer place: with one dialed link that
   vouches for it, it counts. Every holder that counts only thanks to rosters is counted against
   one introducer, the rosters, so they take at most ⌈n/4⌉ of a generation's pieces. A roster
   alone never makes a holder count, nor reachable.
3. **Flat weights.** Every candidate above the bar weighs the same. A node up all the time and
   seen for a year weighs no more than one up 85% and seen for a week, so a patient party's
   always-on nodes gain nothing over honest ones.
4. **Places** are Warmth's (`hoi_sync::warmth::places_of`: one network, one declared operator), so
   N is a count of places, not nodes; a place's weight is one however many nodes it has. Leave out
   the places of this object's forwarders.
5. **Caps per generation.** At most ⌈n/8⌉ indexes in one IPv4 /16 or IPv6 /32, and at most
   ⌈n/4⌉ among candidates first learned through the same introducer.
6. **Pick** n places uniformly at random from the CSPRNG among those that satisfy 5.5 rule 2 and
   the caps; one node per place, at random among its candidates.
7. **Pair** each piece with a forwarder among this device's own links, in a place other than its
   holder's, at most k−1 pieces per forwarder, at least two forwarder places per object. A holder
   must be in the forwarder's own node list (`nodes`), so nobody makes a public node dial an
   address of their choosing (a rule the protocol already has). If no forwarder lists a chosen
   holder, choose another holder, never another forwarder in the holder's place.
   *Built in S3:* a device's forwarders are its links that say `fw` (every public node); the
   holders it can use are those they vouch for in `nodes.h`, with the holder's signed key (`k`)
   and routes (`r`). A holder is placed by its own link, by routes this device saw itself, or by
   the first route every forwarder vouching for it gives alike; vouchers that disagree, or give
   none, leave it out. The forwarders' places are left out of N. Forwarder places are fixed when
   an operation starts; each carries at most k−1 *distinct indexes* of the generation (a retry of
   an index through the same place adds nothing, through another place adds one), so with three
   forwarders and 5 of 12 there is no room for retries: a fourth link gives it. A holder whose
   requests went nowhere through two forwarder places is left out of new placements for six hours.

The consent gate applies where the device connects: its forwarders. Holders are reached
only through forwarders and never see the device's address, so a browser is not asked about every
holder's notice; a node the person declined is never used as a holder or forwarder.

### 5.8 Forwarding (two hops, sealed envelopes)
Every node with `scatter` publishes an X25519 key in its hello (`xk`, with an expiry), signed by
its endpoint key. Envelopes use HPKE (RFC 9180, base mode, DHKEM(X25519, HKDF-SHA256),
HKDF-SHA256, ChaCha20-Poly1305; `hpke-rs` is already in the tree through OpenMLS), `info` naming
the message (`"hoi scatter v1 put"`, `… get`, `… has`, `… drop`, and `stored`, `got`, `had`,
`dropped` for the answers), the AAD binding the holder's endpoint id and key id (answers: the
locator asked about). Every request carries a one-time X25519 key its answer is sealed to, so a
forwarder can neither read an answer nor replay an old one: it opens under no new request.

- **put**: author → forwarder: `put { to: holder, env }`; `env` is sealed to the holder and holds
  `{ loc, reply key, expiry day, drop hash, drop key id, stamp, piece }`. The forwarder learns the
  author's address, the holder and a size; the holder learns the piece and the forwarder, not the
  author. The holder answers `stored` sealed to the reply key.
- **get**: reader → forwarder: `get { to: holder, env }`, `env = { loc, reply key, wanted length }`;
  the holder answers `got { env' }` sealed to the reply key, back along the same link, padded to the
  wanted length whether or not it has the piece. The forwarder learns the reader's address and the
  holder; the holder learns the forwarder.
- Forwarders keep nothing but rate counters. `get`, `has` and `drop` envelopes are one size
  (256 bytes, enough for a drop signed by the drop key); `stored`, `had` and `dropped` 64 bytes;
  `got` the piece length plus 54. Padding requests to the piece size, as first drafted, would
  double read traffic and hide nothing that direction and volume do not already show. The exact
  layouts are PROTOCOL.md §21.
- *Built in S3:* a forwarder passes a request on only to a holder it is linked to now, or to one it
  saw itself (its own vouches), which it then dials and holds the request for up to 20 seconds
  (at most 128 requests, 32 MiB); anything else gets `gone` at once, an unsealed one-byte answer
  that says the request went nowhere. Answers go back by the forwarder's own request id, so the
  asker's id never reaches the holder. A forwarder rate-limits each asking link as a holder does;
  holders give links that forward ten times the limits and do not blame them for envelopes that
  will not open. The asker treats `gone`, silence and an unopenable answer alike: the same index
  goes to the same holder through another forwarder, which places nothing new, and only with
  none left does a fresh index go elsewhere.

### 5.9 Fetching and rebuilding
On opening a post (or ahead of time for the first screen): derive the locators of the newest
generation with at least k reachable pieces, pick k+2 of its live indexes at random (so popular
posts spread load over all holders), send them through at least two forwarders in different places,
at most k−1 each, check each piece's hash as it arrives, ask for another index when one fails or
is slow, decode as soon as k good pieces are in, undo the transform (the canary must be zero),
open the seal, check the record as today, show it. The rebuilt post is kept only in the device's
encrypted store. "Use less data" fetches k+1.

### 5.10 Repair
Members' durable devices (home nodes, desktops) of established members run repair for recent
objects every 6 hours, jittered:

1. **Probe**: `has { loc }` through forwarders, one per piece. A piece counts as lost only after
   two failed probes through forwarders in different places, at least 12 hours apart, so neither a
   forwarder dropping probes nor a holder that is briefly away triggers repair.
2. **One repairer at a time**: wait a random 0–6 hours before acting; stand down on seeing an
   update for the object.
3. **Repair within the budget**: once a third of the spare pieces are gone, when reachable pieces
   fall to `k + ⌈2(n−k)/3⌉` (24 of 28 at the default), rebuild from k, make fresh indexes
   up to the budget, place them (5.7 and 5.5 rule 2), and publish a pointer update naming the new
   indexes, their hashes and holders. The first draft's `k + ⌈k/3⌉` (20 of 28) lost about one post
   in fifty over a month at 10% churn a day in the lifetime simulation and let the readable share
   fall to 97%; the earlier trigger keeps every post readable there (section 15), at about twice
   the repair rounds when churn is low.
4. **Regenerate** when the budget is spent (5.5 rule 3).
5. **Hot posts**: when fetches of an object pass a threshold (holders answering `full` or slow), a
   member's device regenerates it early with n at least doubled, sized by 5.4 for that n (a larger
   k at the same risk budget): more holders share the load and the bound tightens rather than
   loosens.
6. **Updates are checked by everyone who reads them**: the signer is the author or an established
   member; the object id and the sealed object's hash are those of generation 0 (and a rebuild that
   does not match is refused and remembered against the signer); new indexes are fresh and within
   the budget; holders are distinct endpoints and, as far as the reader can tell, distinct places;
   at most one regeneration per object per 24 hours is accepted (the earliest by `created_at`, then
   by id). Readers keep the previous generation as a fallback until the new one has rebuilt once
   on this device.

Holders never repair: they cannot. A post can always be repaired from any member's own copy
(the author's, or any reader's encrypted store), not only from its pieces.

*Built in S3* (`hoi-core/src/keep.rs`): every object a device sent or received a pointer for is
kept track of with its newest generation and the one before it (the fallback, and what a
competing new generation is checked against). Durable devices (desktops, nodes) probe young
objects every six hours, older ones daily, then every three days; a probe of an index that
failed before goes through another forwarder place if it can. A holder's own sealed "not kept"
loses an index at once. A repair rebuilds from k pieces (a rebuilt sealed object must match the
manifest's hash), places `min(n − reachable, budget left)` fresh indexes, and otherwise makes the
next generation once a day, or at once when fewer than k are left. An object is hot when 16
answers in an hour of its fetches failed or stalled; it is then regenerated once, at least a day
apart, with at least twice the pieces (`size::size_at_least`). Updates are checked as rule 6 says
(`Manifest::update_to` and `size::reading`); a generation whose k pieces rebuild into another
sealed object, or whose every answered piece fails its hash, is dropped and its sender not heard
again for that object. Since S5, outside private groups (where MLS admission is the check) an update
counts only from the pointer's first author or an established member, and must name what the
first named. A durable device that may update the object (in an open community
its author or an established member, in a private group a member) makes it again from its own
copy when probes leave fewer than k pieces or neither generation rebuilds: it seals what it keeps
(the record, or the media slice of the manifest's class) and, once that gives the very bytes the
manifest's hash names, starts the next generation from it; a copy that seals otherwise is never
published, and the device does not try again. A reader that finds an object unreadable reads its
pointer records again, so a new generation it refused for coming within a day counts then.

### 5.11 Retention, deletion, abuse
- Pieces expire with the post's lifespan, at most 400 days; posts kept longer are regenerated
  before their pieces expire.
- `drop { loc, D }` with the creator's drop secret, or `drop { loc, sig }` signed by the community
  drop key whose id the piece was stored with, removes one early. The author's devices send drops
  for their own deleted posts; moderators' devices send drops for removed posts. Neither readers
  nor members can drop: they hold neither secret. A drop that no ledger removal or author delete
  explains is a loss to repair, so drops cannot silence a post the community kept.
- A `put` carries a stamp (proof of work, as records have), bound to `loc`, size, expiry and the
  holder's endpoint id (so it cannot be replayed at another holder), its cost growing with size:
  8 bits up to 1 KiB and one more for each doubling (18 bits for a full 1 MiB piece).
  Forwarders rate-limit per link (default 1 MB a minute from a browser), holders per forwarder and
  in total; a holder that is full answers `full` and is skipped. Holders evict expired pieces
  first, then pieces nobody probed or fetched for 30 days.
- *Built in S3*: a private group's posts, comments and edits go in pieces one object
  each, so a drop is one post's. The group's drop key is made by the owner's device, which keeps
  its secret; its public part travels with the mailbox list in the MLS group context (`d`), so
  every member's pieces are stored with its id. Once everything an object carries is deleted by
  its author, removed by the community, or no longer kept, every device lets the object go and
  nobody repairs it; the device that made a generation drops it with its own secret, and the
  owner's device drops a removed post's generations with the drop key. Since S5 a
  media object names the posts it was sent for and goes with them, dropped with a removed post
  even if other posts use the same bytes: theirs are objects of their own.
- *Built in S4*: every committer's device holds the group's drop key. Each device in
  the line announces a seal key in the group (X25519, derived from its device key, for nothing
  else); a device that holds the listed key seals it to those the list does not name as holders
  and lists them, after the records that make them committers. When a holder leaves the line, a
  committer makes a new key for those still in it, and the one that left forgets its keys; a
  device that lacks the key waits until each holder had its turn to give it, then makes one.
  Pieces keep the key they were stored with; devices keep a replaced key 31 days, until those
  pieces have expired, and drop with every key they hold. Drops follow the line: the first at
  once, the one of rank *r* after *r* hours, and only if the one index it asks first is still
  kept.
- *Built in S5*: an open community's drop key is in its ledger: `drop-key` entries by
  owners, the secret sealed to the seal key each owner's and moderator's device carries in its
  delegation; a new key once a device it was sealed to leaves the line, the same one again for
  a device in line that lacks it; replaced keys kept as long as the community's posts may live.
- Operators cannot inspect what they hold; removing abuse is the community's moderation, which
  ends in drops. Blind storage can be abused like any encrypted storage: stamps, quotas and expiry
  bound the volume, not the purpose (Rate-Limiting Nullifiers, M5, would bound it per person). The
  join notice gets a fixed sentence (`hoi_types::notice::PIECES`, notice key `s`), set by the node
  from how it runs, never from the operator's file: "This node also keeps encrypted pieces of posts
  and encrypted messages of private communities for other people's devices, and hands them back
  when asked. It cannot read them. Each is deleted when it expires, after 400 days at most."

**Durable author work (RFC-0001).** Deletion is bounded and eventual: rejection is guaranteed
after a compliant holder learns the authorized drop, not before an uninformed holder rejoins.
Expiry is the outer validity boundary. Neither a successful sync nor a chosen number of peers
proves that every deletion has arrived.

The requesting device captures known generations and per-locator capabilities before discarding
manifests, and persists its deletion journal with local suppression and the signed delete before
dispatch. Restart resumes the same work. Scheduling is bounded; transient failures back off,
and queue exhaustion refuses new work without evicting unfinished targets. Missing capabilities
and unknown generations remain explicit coverage gaps. The journal keeps object IDs for late
repair, routing targets and expiry days for retry, tokens for authorization, and per-target
outcomes/attempt times for progress; it does not retain deleted content or its decryption key.

Progress separates local removal, signed-record submission, queued/submitted targets, durable
acknowledgements, legacy removal observations, unknown/refused/unreachable targets and expiry.
Only a matching sealed reply from a holder with a verified, unexpired `drop-journal` capability
counts as a durable acknowledgement. The holder signs the capability with its endpoint key;
vouches preserve that signature (PROTOCOL §21.1). An old holder's success is a legacy observation.
Timeout, unknown and expiry are never evidence of erasure; even every known holder's
acknowledgement does not prove complete inventory or delivery of the delete to every member.

New clients do not start legacy home-delegated placements, whose format cannot recover the
origin's placement correlation and deletion authority after restart. They scatter locally when
eligible or use the existing whole-record fallback. Legacy receivers remain compatible. A local
delete waits retryably while an existing legacy delegated placement is unresolved, before any
irreversible mutation; unresolved legacy inventory is not silently counted as deleted.

Holders commit payload removal and `{loc, expiry_day, drop_hash, drop_key_id}` in one durable
transaction before acknowledging. The locator blocks replay; the original day limits retention;
the hash and key ID allow authenticated duplicate requests without retaining raw tokens or
signatures. The existing correlation between pieces using the same drop-key ID remains.
Get/has report absence, and puts at that locator are refused through its inclusive
expiry day. Invalid drops create no marker; unknown locators remain unknown. Admitted pieces
reserve marker slots, and live markers are never evicted for new data. Repeated drops never
extend them. The original stored day matters because scatter placement rounds expiry upward.

Accepted deletion cancels unsent repair and suppresses fetched/placed results already in flight.
Known late placements enter the same cleanup work instead of publishing replacement content.
Valid moderator removal retains its separate authorization; unrelated live posts and their own
media objects are preserved. On restart, persisted deletion state is loaded before network work.
Supported holder restoration uses `SqlitePieces::restore`, discarding stale payloads while
keeping available markers; turning holding off also preserves live markers. Neither direct disk
rollback nor a malicious operator's private copy is covered. The holder schema migrates legacy
payloads without inventing acknowledgements and prevents old software reopening it without
marker enforcement; details are in PROTOCOL §21.1.

The Rust `Node::open_restored` member-snapshot API quarantines public content during ordinary
catch-up. Private snapshot content and its queued ciphertext/repair cache are discarded before
serving, while deletion work survives. Private output remains quarantined until an accepted
fresh join; an active restored MLS group has no general reset/rejoin path. Recover on a fresh
device with an authorized private join in that case. The applications do not expose a full
database snapshot import flow; identity-key recovery is separate (PROTOCOL §21.1).

### 5.12 The home node as the person's own end
A paired home node is the person's device. It may fetch and rebuild for its own phone and browser,
and hand them rebuilt posts over the pairing link, which cuts a phone's data and links to one.
It never serves rebuilt posts to anyone else.

*Built in S3*: a home node says `rb` in its hello to its own devices only, and answers
`rebuild { r, m: manifest }` only from them with `rebuilt { r, c? }`, fetching through its own
forwarders. A device whose home node said `rb` fetches the objects it keeps track of through it,
and through its own forwarders when the home node fails or does not answer within two minutes
(then for ten minutes). "Use less data" (You → Network, browsers and phones,
off at first) keeps a paired device on its home node alone: it asks the home node again
whenever it is linked, with no pause after a failure, and fetches no piece itself, waiting
while the home node is away; a device with no home node asks k+1 pieces. A home node that did
not answer says nothing about the generation, with the setting or without: only a home node's
"could not rebuild" moves the next try to the generation before.

### 5.13 Mailboxes: private groups without a relay (S2, built)
A private group's devices had found new messages through a relay's listing of each epoch's room.
Without a relay they need another way to learn what is new, without anyone holding a list.

- **The group agrees on its mailboxes.** The owner's device picks up to four holders, one per
  place (5.7's caps), first those it saw long and steadily, any linked holder when there are too
  few, at least two; it writes them with their routes into the MLS group context (extension
  0xff44, required of every member). A commit changes the list; a Welcome hands it to newcomers.
  The owner's device replaces a mailbox it has not seen for two days. Nobody sets anything up.
- **Each member writes into its own numbered slots.** Slot `seq` of member `leaf` in an epoch is
  at `BLAKE3_keyed(derive_key("hoi mailbox slot v1", epoch relay secret), u32(leaf) ‖ u32(seq))`.
  Only that epoch's members can compute it; every membership change starts fresh addresses; a
  holder sees random locators and never a listing. A slot holds one of the same sealed, padded
  objects the relay carried, written with the ordinary sealed `put` and stamp, at most 30 days. A
  message counts as sent once two mailboxes confirm it.
- **A bell tells members something is new.** Each epoch has a bell: numbered slots no member
  owns (leaf `u32::MAX`). A writer whose message is stored rings it on every mailbox, a small
  random object in the next bell slot that mailbox does not hold (two ringing at once take two).
  Each member listens to one mailbox's bell, chosen by its own leaf, backing off to every two
  minutes; a ring sends it over every member's next slot (a `has`, 64-byte answer carrying the
  length, then a `get` of exactly that length), and whoever posted is followed until it stops.
  Every ten minutes it looks at everyone anyway, and one past the bell's next slot, so a lost ring
  or slot stalls nothing. An idle member of a group of eight sends about a dozen probes in ten
  minutes instead of sixty; a post arrives within two and a half minutes.
- **Membership changes go to the epoch they end.** Slots are given out when a message is queued,
  so a commit lands in the old epoch's slots after everything queued before it; members who read
  it move on to the new epoch's addresses. A holder never overwrites a slot: it answers `taken`,
  and a device that lost its place after a restart moves on to its next slot.
- **Hosts tick every five seconds.** The core does all of this in `scatter_tick` (native hosts,
  the browser worker); a device owning a group still without mailboxes dials holders it met and
  public nodes it knows of, and keeps a few more links, until it has two.
- **Every group, from its creation.** Every private group requires the mailbox capability
  (0xff44) of every leaf, with the others: there is no group a member's app keeps on
  a relay. A relay still carries a group while it has no mailboxes yet, where the host has one.

What a holder learns: that someone stored a sealed object of one size class at a random
locator, and that someone fetched it, as the relay saw before. Since S3 a member
reaches a mailbox through a forwarder in another place whenever one vouches for it, and does not
dial mailboxes a forwarder reaches, so a mailbox sees the forwarder's address and not the
member's; only a mailbox no forwarder reaches is reached over a direct link, as before. The
owner chooses mailboxes among holders it reaches either way.

**Posts in pieces (S3, built).** A private group sends each post, comment and
edit as an object of its own, and its media in objects of up to eleven 256 KiB chunks, in pieces
when the device can place a generation now; everything else (votes, deletions, profiles, a
device's own authorization records) goes whole, so a renewed member is recognised before any
pointer from it. A vote, edit, deletion or reply that arrives before the post it is about,
while that group has objects still being fetched, waits and is tried again for an hour. The MLS message
that goes to the mailboxes is a `pointer` payload with up to 64 pointers; members fetch each
object through their own forwarders and apply its payloads as if they had come whole. When the
pieces cannot be placed, the same payloads go whole, as before.

---

## 6. The community's setting: how much of the network a party would need

Communities already choose how much identity work a key must carry (`ladder.identity_bits`: the
protocol has a floor, 8 is the default, up to 20, changed only by consent). Scattering gets one
setting of the same shape.

```
Limits.scatter = { resist: 10,      // percent of the places pieces can go to that a party may run
                                    // without rebuilding or withholding a post: 10, 15 or 20
                   open:   false }  // open communities only: scatter post bodies (S5)
```

**What a community chooses**, by a limits proposal the community decides:

| Level | In the app | A big network needs | Piece requests per read | Piece operations at a million people | Traffic, storage |
|---|---|---|---|---|---|
| **10%** (default) | "a tenth" | 28 places per post | 17 | 136,000/s | ≈ 450 MB/s; 1.87× |
| 15% | "15%" | 44 | 26 | 208,000/s | ≈ 450 MB/s; 1.83× |
| 20% | "a fifth" | 71 | 41 | 328,000/s | ≈ 470 MB/s; 1.82× |

Higher levels cost places and operations, not bytes: a phone downloads about the same for a
picture at any level, and makes more, smaller requests. In a network with fewer places every level
is still met (section 5.4, small networks), so a community never has to lower its level for a young
network; it pays in more places per post.

**What is the same everywhere**, and why it is not a setting:

- **The floor (10%) and the risk budget (one in a million over a life).** The app's sentence "a
  party would need more than a tenth of the places" must mean the same in every community, or it
  means nothing. Going lower would save operations and places, not bandwidth or storage (both stay
  near 1.9×): little to gain for a weaker sentence.
- **The machinery**: AONT-RS, the HPKE suite, padding classes, locators, generations and budgets,
  one piece per place, at most k−1 pieces per forwarder, two forwarder places, candidate rules,
  drop rules, the exact arithmetic. These are what make the level true; a community choosing them
  would choose whether its own setting is honest.
- **The words** come from the achieved reading of each post (its k, n, M, N and generation), never
  from the setting alone.

**What a device chooses**: "Use less data" (k+1 spares instead of k+2, and rebuilding on the home
node). Never fewer pieces or places.

**Not offered**: below 10%, for the reasons above; above 20%, which needs more than 100 pieces per
object and more places than a browser's links and a young network can reasonably place.

**How it is checked and changed.**
- `scatter` is written in the genesis or a limits proposal only when it differs from the default,
  so communities on the default stay readable by older releases (decoders refuse unknown fields).
  `resist` must be 10, 15 or 20; anything else is refused.
- **The limits of a record's day** (PROTOCOL.md §4) apply: a post scattered at 10% is not refused
  after the community moves to 15%; repairers make the next generation at today's level.
- Receivers check every manifest against the floors: 3 ≤ k ≤ n−2, n ≤ M ≤ n + ⌈n/4⌉, M ≤ N,
  n ≤ 160, generation ≤ 255; and compute the post's reading (the largest level whose bound its
  generation meets for its N) with the shared function. A post below its community's level is
  shown, labelled ("spread thinner than this community asks"), and repaired up to it.
- The Limits page explains the choice in one line per level, with what it costs on phones.

---

## 7. Wire protocol

| Message | From → to | Carries | Limits |
|---|---|---|---|
| `hello.sk` | holder | X25519 key, expiry, signature by the endpoint key; its presence says "I hold pieces" | derived from the endpoint key for each 30-day period; the previous period's key still opens envelopes for 7 days; expired keys refused. (Not an `fx` name: releases before S2 refuse a hello with more than 8, and a public node already sends 8.) |
| `hello.fw` | any node that forwards (every public node) | `true`; written only when true | older peers ignore it; a peer without it is never asked to forward (S3) |
| `nodes.h[].k`, `.r` | forwarder → its links | a vouched holder's signed key and up to two routes | receivers check the key against the holder's endpoint; routes only merge places (S3) |
| `put` | sender → forwarder | `to`, sealed envelope | piece ≤ 1 MiB; per-link rate; ≤ k−1 per generation per link |
| `stored` | holder → forwarder → sender | sealed `{ ok \| full \| bad stamp \| refused }` | 64 bytes |
| `get` | reader → forwarder | `to`, sealed `{ loc, reply key, wanted length }` | 256 bytes; per-link rate; misses count against the link |
| `got` | holder → forwarder → reader | sealed piece or `{ none }` | padded to class |
| `has` / `had` | repairer → forwarder, and back | `to`, sealed `{ loc, reply key }`; sealed yes or no | 256 and 64 bytes; members' durable devices |
| `drop` / `dropped` | author or moderator → forwarder, and back | `to`, sealed `{ loc, reply key, D }` or `{ loc, reply key, key, sig }`; sealed status | 256 and 64 bytes |
| `gone` | forwarder → asker | one unsealed byte: the request went nowhere | only to a peer that asked with `to` (S3) |
| `hello.rb`, `rebuild` / `rebuilt` | home node ↔ its own devices | a manifest; the rebuilt content or none | `rb` only to own devices; a home node answers only them (S3) |
| mailbox list `d` | MLS group context 0xff44 | the group's drop key (public) | written by the owner's device; older readers ignore it (S3) |
| pointer record | community sync | sealed manifest of generation 0 | ≈ 1.8 KB at the default; private groups: an MLS `pointer` payload (S3) |
| pointer update | community sync | sealed: new indexes or a new generation | signed by an established member; the same `pointer` payload in private groups |
| `drop-key` | ledger | the community drop key's public part; its secret sealed to each owner's and moderator's device (S5) | any one owner; the key drops only what a removal explains |

```
manifest = { v, object id, sealed hash, size class, generation g, level ℓ, places N,
             k, n, M, pieces: [ BLAKE3-128(piece_i) ], holders: [ endpoint id ], S_g, K_obj, ttl }
```

About 1.8 KB at 15 of 28 (16 bytes of hash and 32 of endpoint id per index, budget included).
Catch-ups move pointers, not bodies, which is the "partial sync" planned for big communities. A Merkle root instead of the hash list would save about 0.5 KB if pointer
gossip turns out to matter.

Senders never send `put`, `get`, `has` or `drop` to a peer without `scatter`. A forwarder refuses
a `to` outside its own node list. PROTOCOL.md gets a §21 with these, and vectors in
`core/vectors/scatter.json` (pieces, locators, manifests, envelopes, drops, and the sizing
function's answers for every level, generation and a spread of N) for every implementation.

---

## 8. Security requirements for the implementation

What the code must do, beyond the design, before any phase ships:

**Cryptography**
- No primitive is implemented here: HPKE from `hpke-rs` with the RustCrypto provider (already
  vetted through OpenMLS), XChaCha20-Poly1305 and ChaCha20 from the crates in the tree, BLAKE3 for
  hashes, keyed hashes and `derive_key` with the context strings above, each used for one thing.
- Every key, nonce and secret comes from `getrandom`; `K_obj`, `K_a,g`, `S_g`, one-time reply keys
  and HPKE contexts are zeroized after use (`zeroize`, as `sealed.rs` does).
- Open in this order: piece hashes against the manifest, then decode, then the AONT canary, then
  the AEAD, then the record's signature and checks; any failure drops the object without partial
  output, and a failing piece's holder is not asked again for that object.
- The erasure code is ours (`hoi_crypto::scatter::rs`, S1): systematic Reed–Solomon over GF(2⁸)
  in Backblaze's construction, about 200 lines of table arithmetic on ciphertext, no `unsafe`,
  tested on every k-subset of small codes and by property tests to 256 indexes. The crates
  considered were `reed-solomon-erasure` (no release since 2022, and it pulls an old `lru` line
  with a soundness advisory) and `reed-solomon-simd` (a GF(2¹⁶) FFT code whose exact codeword
  is much harder to specify for other implementations); the codeword is part of the protocol
  anyway (PROTOCOL.md §21) and the vectors fix it.

**Decoding and resources**
- Every message, envelope and manifest decodes strictly (unknown fields refused, as
  `Fields::done` does), with lengths checked before any allocation: k, n, M, piece size, list
  lengths, generation.
- Erasure decoding runs only after k hashes matched, in the worker or a bounded task, never on the
  network thread; at most a fixed number of decodes in flight per device.
- Forwarders and holders bound everything per link and in total: requests in flight, bytes a
  minute, misses, stored bytes, open holder links (a pool, idle links closed); excess answers
  `full` or closes the link, never queues without bound.
- No amplification: a `get` is 256 bytes and its answer a piece that the requester's own link
  limit paid for; nothing answers a stranger with more than it was sent before the stranger proved
  its address.

**Exact arithmetic**
- The sizing and reading functions use integers only (binomial coefficients as big integers,
  shares as rationals); results are checked against `scatter-params.mjs` and the vectors in native
  and wasm builds.

**What nodes keep and log**
- A holder stores a piece with its locator, expiry, size, drop hash and drop key id, and nothing
  about who sent it. Forwarders keep counters, not pairs of addresses and holders. Logs never
  carry locators, holder ids next to addresses, or piece sizes per link; `admin pieces` shows
  totals.

**Keys and identities of nodes**
- `xk` is accepted only with a valid endpoint signature and an unexpired date; senders seal only to
  keys of holders whose endpoint id the forwarder will dial (the transport authenticates it), so a
  forwarder cannot substitute a holder.
- Candidate lists, rosters and reported uptimes are claims: they may only remove candidates or
  lower figures, never add a place by themselves (rule 2 of 5.7).

**Tests that gate each phase**: section 14.

---

## 9. Code: what changes where

| Area | Where | Work | Size |
|---|---|---|---|
| Pieces (S1, built) | `hoi-crypto/src/scatter/{mod,rs,aont}.rs` | object seal, AONT-RS, Reed–Solomon, generations, locators, piece hashes, drop secrets; property tests (any k rebuild, k−1 do not, generations never combine, any bytes never panic) | M |
| Sizing (S1, built) | `hoi-crypto/src/scatter/size.rs` | the exact sizing and reading functions of 5.4; agree with `scatter-params.mjs` on every case in the vectors | S |
| Envelopes (S1, built) | `hoi-crypto/src/envelope.rs` | HPKE seal and open through `hpke-rs`, one-time answer keys, published holder keys, padding, strict plaintext layouts | S |
| Manifest (S1, built) | `hoi-types/src/scatter.rs` | size classes, the manifest, its floors; strict CBOR | S |
| Vectors and fuzzing (S1, built) | `core/vectors/scatter.json`, `core/fuzz` | the generator in `hoi-validate`, conformance test, four cargo-fuzz targets (manifest, envelope plaintexts, rebuild, sizing) | S |
| Types | `hoi-types` | pointer and pointer update bodies, `Limits.scatter` (optional, default omitted), `drop-key` ledger entry, an X25519 key in moderator devices' delegations, size classes and limits | S |
| Validation | `hoi-validate` | `scatter` limits range, manifest floors, update rules of 5.10 | S |
| Messages | `hoi-sync` | the messages of section 7, decode limits, vectors | M |
| Store | `hoi-store` | a `pieces` table (locator, bytes, expiry, size, drop hash, drop key id), budget and eviction | S |
| Holder, forwarder (S2, S3, built) | `hoi-core/src/scatter.rs`, `hoi-core/src/forward.rs` | answer `put`, `get`, `has`, `drop`; rate limits; node-list rule; forwarding with a short queue while a known holder is dialed (at most eight temporary outgoing holder links within the 32-link transport limit, closed after 120 idle seconds) | M |
| Sender, reader, repair (S3, built but home-node rebuild) | `hoi-core/src/scatter.rs`, `hoi-core/src/keep.rs`, `hoi-sync/src/place.rs` | candidates and corroboration, placement on Warmth places with caps, forwarder pairing, retries, the object registry and update checks, probes, budgeted repair and regeneration, hot regeneration; home-node rebuild left | L |
| Moderation | `hoi-core` | the community drop key: made by owners, sealed to moderators, rotated on `mod-remove`; drops for removed posts | S |
| Warmth | `hoi-sync/src/warmth.rs`, `hoi-core/src/warmth.rs` | a pieces reading per community (section 13) | S |
| hoi-node, desktop | `--hold-pieces GB` (built; on by default, 2 GB, since S6 on every node that serves strangers), `admin pieces` totals (built), the notice sentence (built), `--rtc-bind` (built); the desktop's *Keep mailboxes for others* while a public node (`holdPiecesGb`, `hoi_net::sync_piece_store`, built) | S |
| Apps | `apps/web` | post status, "how this reached you", the level on the Limits page and in proposals, warmth words, the data-saving setting | M |
| Simulation (S2, S3, built but load) | `hoi-sim/src/{scatter,life}.rs` | a `scatter` scenario: churn, colluding and Sybil shares, candidate poisoning, forced repairs, malicious updates (built); load from 10,000 to 200,000 online (S6) | M |
| Docs | PROTOCOL §21, `relay/README.md`, the notices | | S |

---

## 10. Phases, and what may be said after each

| Phase | Builds | Done when | What the app may then say |
|---|---|---|---|
| **S0** (shipped) | Warmth counts places and checked links; desktops stop redialing declined nodes | shipped | "Kept in 3 independent places", "every link checked" |
| **S1** (built) | the pieces library, sizing, envelopes, vectors, property tests, fuzzing; no network | vectors pass in native and wasm builds; the sizing function matches the reference for every level, generation and N in the vectors | nothing new |
| **S2** (in progress) | holders on public nodes, candidates and placement (5.7), one-hop `put`/`get` for private groups, `Limits.scatter`, replacing the single relay when enough places exist (the relay stays as fallback). **Built:** `Limits.scatter`; `hello.sk` and `piece` messages; piece stamps; the piece store (memory, SQLite) with budget and eviction; the holder role and `hoi-node --hold-pieces`, `admin pieces`; placement with caps over a generation's life; one-hop `scatter_store`/`scatter_load` with replacement by fresh indexes; `hoi-sim scatter`; mailboxes for private groups (5.13), so they sync with no relay; first-hand uptime before a holder takes pieces (5.7); the notice sentence (5.11); the app's words; the installers' and setup app's mailbox option (on by default there); holders vouched for in node lists by the person's own devices or two places (5.7); holder keys bound to the WebRTC certificate, so browsers can check them (without it browsers saw no holder at all); desktops as holders by choice; a bell per epoch so polling no longer grows with the group (5.13); `hoi-node --rtc-bind`; `scripts/e2e-mailboxes.mjs`, real `hoi-node` holders and browsers with no relay. **Then:** rosters, opt-in, from established members' durable devices, a second source of vouches weighed by voices and capped through one introducer. **Left:** testing the setup app as the built desktop app. | a private group syncs with the relay switched off; in the simulation a party at the level's share rebuilds and withholds no more often than section 3 says, and never in the hard regime | "Nodes that keep this private community's messages can't read them, and only members can find them." (Mailboxes hold whole sealed messages, a few copies each, not pieces; one hop: holders see the writing device's address, as the relay does today.) |
| **S3** (built) | two-hop forwarding for upload and fetch, budgeted repair and regeneration, hot regeneration, update checks, home-node rebuild, drop keys. **Built:** forwarding by every public node (`hello.fw`, `gone`, a short queue while a known holder is dialed); holders reached through forwarders' vouches with signed keys and routes; two-hop `scatter_store`/`scatter_load` with forwarder places left out of N and at most k−1 distinct indexes of a generation through one forwarder place; retries through other forwarders before a fresh index; the object registry with checked pointers and updates (one regeneration a day, the earliest of rivals, the previous generation as fallback, senders of mismatching generations refused); probes with the two-place, half-a-day loss rule; repair within the budget, then the next generation; hot regeneration with twice the pieces; private groups' posts and media in pieces, pointers through the mailboxes (leaf capability 0xff45); the app's words from each group's achieved level; `hoi-sim scatter --life`. Then: one object per post; the group drop key in the mailbox list, author drops and the owner's drops of removed posts, objects let go once their posts are gone; mailboxes reached through forwarders; home-node rebuild (`hello.rb`, `rebuild`/`rebuilt`). **Completed by S5:** established-member repair and open-community drop keys | the simulation keeps 99.9% of posts readable at 10% churn a day; no path carries k pieces; forced repairs and malicious updates leave the lifetime bound intact (all three met in `hoi-sim scatter --life`, with the earlier repair trigger) | "No single node hands you a whole post, and the nodes that store pieces don't know who posted. Someone would need more than a tenth of the places to rebuild or stop one." (Private groups now say the first sentence, and "stopping one would take more than [their level] of the places its pieces go to", only while their posts actually go in pieces.) |
| **S4** (built) | members-only communities become MLS groups, then scattered. **Built (first step):** a private community admitted by invitation or approval is an MLS group whose membership follows its own records: the owner's device adds the people its vouches and `admit` entries admit and removes those it bans or whose identity or device is gone, sending the records that back each change ahead of it; every member refuses a commit those records do not back; any member passes a join request on (vouching for its person where admission is by invitation) and the invitation comes back to the group to hand over; an owner of a members-only community whose records travel in the clear is offered a private copy with the same name, rules and limits. Posts in pieces come with S3. **Built (second step):** several committers in an order: the owner's and moderators' devices, by how long their person has held the role (signed time), then by device age; the first makes each change at once and each next one after an hour more of silence; a committer back from silence catches up before committing; of two commits of one epoch the one earlier in line counts, and members who followed the other keep that epoch a day and go back to the earlier one if it turns up; invitations from moderators' devices carry what makes them moderators. **Built (third step):** a drop key every committer's device holds: each announces a seal key in the group, a device holding the listed key seals it to the others and the list names them as holders, a new key once a holder leaves the line, replaced keys kept a month so older pieces stay droppable, drops in the same line and turns (a later one first asks one index); join requests passed on in an epoch a commit ended go again in the next. **Then:** the clean break: every private group requires every capability (0xff42–0xff46) of every leaf from its creation, requests without them are refused, and the paths for older leaves, the relay v1 state and the old drop-key field are gone; a commit found ahead of a writer's slot waits for the slots before it. (Moving members of an old community to its copy, renewing old groups' leaves and groups made before mailboxes are dropped: there are none.) **Then:** newcomers pick up their own invitations: a join request carries a one-time X25519 key and up to four holders the newcomer reaches; the committer that adds them stores the invitation sealed to that key on those holders, through its forwarders; the newcomer's app looks there and joins by itself. The hand-over by a member stays as a fallback. | old members-only communities offer "make a private copy" (built) | the S2 and S3 sentences for members-only communities, while their posts go in pieces; "Who can read follows this community's own records" for those that follow them |
| **S5** (built) | open communities: pointer records and scattered bodies, `scatter.open` on by default for new communities. **Built:** `pointer` records sealed under the community's read key; a device's own posts, comments and edits kept in pieces and never offered whole, readers rebuilding them on their devices and never re-serving them, carriers taking what refers to them by id; whole again when too few places are around; updates only from a pointer's author or an established member; the community's drop key in `drop-key` ledger entries made by owners, sealed to a seal key in each owner's and moderator's delegation, a new one on `mod-remove`, drops in the line's turns; media objects per post, dropped with their post though others use the bytes | a post in an open community reaches its readers in pieces through carriers that hold only its pointer; a removed post's pieces are dropped by a moderator's device with the community's key (`tests/open.rs`) | "No node that stores this community's posts knows who posted them, and no single node can stop a post." (Not "can't read": it is public. Not "nobody sees who posted": the node a device connects through does.) |
| **S6** (built, review open) | review and scale: an outside cryptography review, the simulation at 200,000 online, default on. **Built:** `hoi-sim scatter --load` from 10,000 to 200,000 online; what it showed fixed: one forwarder hop more for holders a reader's forwarders do not vouch for (manifests list holders' keys, holders open asks to their earlier keys, a new hello at each key period), media in raw slices fitted to size classes; private groups' resends after a commit bounded to the epoch it ended (and own records on an add); holders on by default (2 GB) on every node that serves strangers and on desktops that are public nodes; the notices' new words (version 1.2). **Open:** the outside review | review findings closed | the Privacy Notice and Network & Safety texts change (a new notice version) |
| **S7** (built) | address privacy: three-hop uploads (nested envelopes, the first forwarder never sees the holders) and an injector for all of a device's own records (it never offers its own new records to its own links before they come back from elsewhere). **Built:** `wrap` envelopes sealed to an exit forwarder, sent through an entry that vouches for it, exits alone counting against a place's k−1; `inject` envelopes to a linked public node carrying the community, which passes the records on after a random moment; own records kept from the device's links until they come back, or two minutes; `hoi-sim scatter --address`. **Then:** nothing kept in pieces goes whole to an injector; a person's identity and blessings go first hand only to their own devices and reach others with injected records; a home node injects what its devices post; the hold is saved; a post's checks say the node a device connects through can't tell it is its person's, only while that holds for it | the simulation shows no single node linking a device's address to its records (`hoi-sim scatter --address`: none since S7; a party needs two nodes of one path, 1% at a 10% share) | "The node you connect through can't tell which posts are yours." Linking then needs the first two hops of the same upload together (about 1% of posts at a 10% share), or an observer of all traffic: a VPN still matters. |

---

## 11. Threat model

| Who | What they can do | What stops them | What is left |
|---|---|---|---|
| One malicious holder | read, withhold or swap the piece it holds | AONT and the seal (P1), n−k spare pieces (P6), piece hashes (P7) | it knows a piece of some size exists until it expires |
| One malicious forwarder | see the author's or reader's address, the holders, sizes, times; drop | envelopes sealed to holders; at most k−1 pieces of a generation per forwarder; retries elsewhere; two-probe loss rule | the address of its own link's device, as any first hop sees today (S7, a VPN) |
| One operator with many nodes in one network or under one name | look like many | one place per network and per declared operator: one piece per generation | an operator who hides behind many networks and no name is a Sybil (next row) |
| A Sybil across many networks holding a share f of places | gather k pieces of some generations, or withhold | the level's sizing: at f = 10%, under 10⁻⁶ per object over its life (a hard bound in networks up to about 90 places); flat weights; corroborated candidates; caps per /16 and per introducer; unlinkable locators; for sealed tiers the members' key | a party that buys more than the level's share gets more; in a young network that share is a handful of nodes |
| A poisoner of node lists | fill a device's candidates with its own nodes | candidates first hand or listed by two introducers in different places; at most ⌈n/4⌉ per introducer; claims only lower figures | two of a browser's three links colluding (about 3% at f = 10%) can still tilt that browser's choice within the caps |
| Established Sybils publishing rosters | list the party's holders so they count where honest ones would not | rosters of two established people with the hiding quorum's voice; one dialed link must still vouch; every roster-only holder counted against one introducer, ⌈n/4⌉ of a generation (`hoi-sim`: a party at half the places gets no more, and never k) | the quarter of a generation rosters may fill can lean towards the party |
| A party forcing repairs (withholding to make honest devices re-place) | collect more draws of the same object | fresh indexes within a fixed budget; places never reused in a generation; fresh keys per generation; generations a day apart; the halving risk budget | more repair traffic, not a better chance |
| A member publishing bad updates | move a post onto its own holders, or to garbage | updates only from established members, same sealed object, plausible placement, one regeneration a day, old generation as fallback, refused rebuilds remembered | a member reads by design; it can delay a post's repair, not take it |
| A holder trying to collect siblings | ask for other pieces | locators need the manifest's secret; no listings; misses count against the link | none |
| Readers or members dropping pieces | silence a post | drops need the creator's secret or the moderators' drop key; unexplained drops are repaired | none |
| A DoS against the holders named in a public manifest (open communities) | make one post slow or unavailable | spare pieces, repair moves it, members' copies | a large enough DoS delays one public post for a while |
| A member | read everything | nothing: members read by design | the same as today |
| Someone who sees all traffic (an ISP-scale observer) | correlate uploads and fetches by timing | jitter and padding make it costlier, not impossible | no anonymity network: a VPN still matters |
| Storage spammers | fill holders | stamps bound to size, expiry and holder, per-link and per-forwarder quotas, `full`, eviction | cost grows with their volume |
| Abusive posts | hide from operators | community moderation and drops; expiry | operators cannot inspect: the trade made on purpose |

---

## 12. How it feels for people

The aim is that nobody has to learn a new idea. The first run already rehearses "3 of 5 pieces"
with the key's picture (`Shards.svelte`); posts travel the same way, and the app can say so in one
line there.

| Moment | What a person sees | Words (examples) |
|---|---|---|
| Writing | the composer is unchanged | |
| Just sent | the post appears at once, with a small status that settles in a second or two | "sending in pieces…" → "kept in 28 places" |
| Not enough places around | the post is kept as today; Warmth reads cooler | "kept on members' devices for now; it spreads into pieces when more nodes are around" |
| Reading | titles and the feed appear as now; a post's text 0.1–0.3 s later on first open; pictures fill in as pieces arrive, often faster than from one node | |
| Details of a post | an optional "how this reached you" line | "Rebuilt on this phone from 15 pieces that came through 2 places. No node had it whole." |
| A slow or missing piece | nothing, while the reader asks another place; after a few seconds a calm line | "Fetching this post from other places…" then, if it fails, "Too few places are online for this post right now. It is still kept; try again later." |
| Offline | posts already opened stay readable (the device's encrypted store) | |
| Phones | a text post uploads about 12 KB and reads about 9 KB; a picture uploads about 1.9× its size and reads about 1.15×; with a home node the phone fetches through it | You → Network: "Use less data" (one spare piece fewer, and the home node rebuilds) |
| Warmth | one more reason under the word, never a new score to learn | "Posts here are kept in pieces in 28 independent places. Someone would need more than a tenth of them to rebuild or stop one." |
| The community's rules | the level beside the other joining rules | "Posts are kept in pieces. Rebuilding or stopping one would take more than a tenth of the places they go to." |
| Proposing a change | the Limits page offers three choices with their cost | "A tenth (usual) · 15% · a fifth — more places per post, a little slower on phones" |
| Trust, in the notices | only what the phase allows (section 10), per community kind | open: "No node that stores this community's posts knows who posted them, and no single node can stop a post." private: "No node can read this community, and none holds more than one piece of a post." |
| Operators | the setup app and Manage show pieces held, never content | "Pieces held: 12,400 · 3.1 GB · none readable here" |
| Moderators | removing a post works as today; the app sends the drops | "Removed. Its pieces are dropped from the places that held them." |

What it changes for trust, in the person's own terms: they no longer have to trust whichever node
they happened to connect to. The question "who runs this node?" matters less, because no node
alone gets a readable post, a whole post, or the power to stop one, and even a group running a
tenth of the network does not. The join notice still says who runs a node, and that it carries
pieces it cannot read.

---

## 13. At a million people, 200,000 online

**Assumptions** (one peak hour, deliberately heavy): 1,000,000 people, 400,000 active a day,
200,000 online at once (70% browsers and phones, 30% desktops). Online, each writes one text post
or reply an hour (4 KiB after padding) and a picture every ten hours (300 KB); each opens 60 posts
and 10 pictures an hour. Default level: 15 of 28 for text and pictures, k+2 fetched. Two hops each
way, envelopes and fixed-size requests counted. Today's comparison: one hop from a carrier, three
whole copies on write.

| | Scattered (default) | First draft | Today | |
|---|---|---|---|---|
| Writes | 56 posts/s, 5.6 pictures/s | same | same | |
| Reads | 3,333 posts/s, 556 pictures/s | same | same | |
| Network traffic at peak | **≈ 450 MB/s (3.6 Gbit/s)** | ≈ 470 MB/s | 186 MB/s | 2.4×: two hops; fewer spare bytes offset more envelopes |
| …of which pictures read | 383 MB/s | 418 MB/s | | the dominant cost |
| Piece operations | 136,000/s | 33,000/s | | 4×: more, smaller pieces |
| Per node at 2,000 public nodes | 1.8 Mbit/s average, 68 piece operations/s, ~8 GB/day | 16 operations/s | | over a home node's default 5 GB/day |
| Per node at 10,000 | 0.36 Mbit/s, 14 operations/s, ~1.7 GB/day | 3 operations/s | | fits a home node |
| Storage, 90-day lifespan | 127 GB/day, 11.4 TB in all | 12.2 TB | 18.4 TB (three whole copies) | 1.87× instead of 3× |
| …per node | 5.7 GB at 2,000 nodes, 1.1 GB at 10,000 | | | |
| Sessions, browsers keeping 3 links | 420,000 | | up to 420,000 | the same limit as today |
| …nodes needed | 13,125 at 32 sessions; 1,400 at 300 | | | the first thing that breaks |
| Repair | 1.0 TB/day at 2% of pieces lost a day, 2.5 TB/day at 5% | 1.5 TB/day at 5% (without the lifetime bound) | none (copies just shrink) | 4.3 bytes moved per byte lost: the price of P10. With the earlier repair trigger, repairs come about twice as often at low churn, each moving fewer pieces but rebuilding from k: expect up to about twice these figures |
| A viral picture, 100,000 reads in an hour | 17 requests/s and 0.34 MB/s per holder | 1.3 MB/s per holder | 100,000 full reads from 3 carriers | spread over all n holders by random choice of pieces |
| Pointer gossip | ≈ 1.8 KB per post | 300 bytes | whole posts | still less than syncing bodies |
| Extra time to open a post | +100–300 ms for text | +80–260 ms | | the 15th of 17 parallel answers; pictures often faster |
| CPU | Reed–Solomon over 300 KB and 28 HPKE seals per picture: a few milliseconds on a phone, tens in wasm | | | |

At 15% or 20% traffic and storage stay about the same; piece operations grow to 208,000/s and
328,000/s (section 6).

**Measured (S6).** `hoi-sim scatter --load --seed 1` (one public node per hundred online,
each linked to 64 and vouching for 32, three forwarder links per browser and six per desktop, a
peak hour as above, per public node; "reach" is the share of reads that find k pieces):

```
  online  nodes  relay  media    k/n   reach  ops/s (busiest)  MB/s  GB stored  sessions  picture stored/read
   10000    100         before   8/17  0.984     40 (    53)   0.77       50.1       390    7.43× / 4.37×
   20000    200         before   8/17  0.464     40 (    63)   0.77       50.1       390    7.43× / 4.37×
   50000    500         before  10/20  0.035     48 (    98)   0.74       47.1       390    6.99× / 4.20×
  200000   2000         before  12/23  0.005     56 (   187)   0.72       45.1       390    6.70× / 4.08×
   10000    100    yes  fitted   8/17  1.000     63 (    84)   0.25       15.9       390    2.15× / 1.27×
  200000   2000    yes  fitted  12/23  1.000     87 (   294)   0.24       14.3       390    1.94× / 1.19×
```

Without the extra hop a reader's forwarders reach too few of an author's holders once there are
more than a couple of hundred public nodes; with it every read finds k pieces. Devices see about
190 places through their forwarders' vouches, so they size 12 of 23 rather than the 15 of 28 a
whole network of 2,000 would give. Fitting media to classes cuts traffic and storage to a third
and costs more piece operations (a picture is two objects). Sessions are the first limit, as
below.

**Where it breaks, in order, and what to build:**
1. **Sessions on public nodes**, as today: 200,000 online need 1,400 nodes at 300 sessions each, or
   13,000 at today's 32. The server kit's `--max-sessions` comes first;
   home-node rebuild (5.12) lets a phone keep one link instead of three.
2. **Bandwidth on home nodes**: at 2,000 nodes each carries about 8 GB a day. Either 5,000+ nodes,
   or server-class caps of 20 GB a day; "carriers follow demand" applies
   to holders too: a holder near its cap answers `full` and sheds new pieces.
3. **Holder links on forwarders**: implemented with the existing 32-connection global bound,
   at most 8 temporary outgoing connections, and a 120-second idle timeout. Temporary holder
   routes are not saved as bootstrap hints. Pairing pieces with forwarders already linked to
   their holders remains an optional optimization, outside this completion scope.
4. **Piece operations**: 68 a second per node at 2,000 nodes is light on servers and noticeable on
   a Raspberry Pi; batching several gets for one holder in one envelope is the first saving.
5. **Hot posts**: random k+2 of n spreads load; hot regeneration doubles n for the few that need it.
6. **Repair traffic under churn**: budgeted repair, longer repair intervals for old posts, and
   placing only on holders above the uptime bar keep it near 1–2.5 TB a day.
7. **Pointer gossip**: 1.8 KB per post; set reconciliation keeps
   catch-ups small, and a Merkle root saves a third if needed.
8. **Fewer than 12 places** in a young or small network: objects stay whole (fallback), and Warmth
   says so. Scattering needs a network of at least a few dozen independent places before a tenth of
   it is more than a few nodes.

---

## 14. Warmth and scattering

Warmth already counts by place and by checked links, the same keys placement uses.
Once pieces exist, a community's reading adds one input, still shown as warmth:

```
pieces reading = over a sample of recent objects:
                 share with ≥ k pieces of their newest generation reachable in distinct places,
                 the median margin (reachable − k), and the share meeting the community's level,
                 each probe through forwarders (has), never all from one place
level          = today's level, capped at the pieces reading's level for scattered communities
                 (cold: some posts unrebuildable · cool: margin 0, or posts below the community's
                 level · warm: margin 1–4 · hot: ≥ 5 and every sampled post at the level)
```

And *your connections* tells a person whether their own fetches can be split at all: cool (one
place) means every piece would pass through one party, so the reader falls back to fewer, slower
paths and says so.

---

## 15. Testing and review

- Vectors for every byte format (pieces, locators, manifests, updates, envelopes, drops) and for the
  sizing function (every level, generations 0–16, N from 12 to 5,000), checked in Rust and the wasm
  build against `scatter-params.mjs`.
- Property tests: any k of n rebuild; k−1 fail; pieces of different generations never combine; a
  swapped piece is caught by its hash; padding hides sizes within a class; no index is ever placed
  twice in a generation; no place ever holds two indexes of one generation.
- Fuzzing: piece decode, manifest and update decode, envelopes, `xk` entries.
- Simulation (`hoi-sim scatter`): churn 1–20% a day; colluding and Sybil shares 1–25% with and
  without candidate poisoning; parties forcing repairs; members publishing bad updates; load from
  10,000 to 200,000 online. Scored on readability, the share of objects any one party could ever
  rebuild or withhold (it must stay under the level's bound), path counts (no path ever carries k
  pieces) and traffic. *Built:* placement against shares and poisoning (S2), and objects over
  their life (`--life`, S3): places leaving for good at 1–20% a day and transient downtime,
  six-hourly probes with the loss rule, repairs and generations as members' devices make them,
  a party forcing repairs or a member of it publishing a generation onto its own places every
  day, three forwarder places per placement. Not built: load and traffic at scale (S6).
  `hoi-sim scatter --life --objects 200 --days 30 --seed 3` (level 10%, holders reachable 90% of
  the time at any probe, readable = the object rebuilds from its newest generation or the one
  before at a six-hourly round; "over k" counts placements where a forwarder place would carry k):

  ```
  places  party  churn/day  forced  bad-updates  readable     lost  rebuilt (expected)   withheld  gens/object    over k
    1000    10%         1%                        1.00000        0        0 ( 4.66e-5)         0          1.4       0
    1000    10%         5%                        0.99996        0        0 ( 8.93e-5)         0          4.0       0
    1000    10%        10%                        0.99979        0        0 ( 9.64e-5)         0          6.7       0
    1000    10%        20%                        0.99746        1        0 ( 9.78e-5)         0         11.2       0
    1000    10%        10%     yes                0.99962        0        0 ( 9.76e-5)         0          9.1       0
    1000    10%        10%                  yes   1.00000        0        0 ( 4.66e-5)         0         61.0       0
      80    10%        10%     yes                0.99625        1        0 (  0.00e0)         0          8.5       0
    1000    30%        10%     yes                0.94729       10       90 (  1.38e2)         0         22.4       0
  ```

  At the acceptance churn (10% a day) every post stays readable and the party at the level's share
  rebuilds and withholds nothing, forced repairs included; far above the level (30%) forcing
  repairs gets it fewer objects than the generations' exact tails add up to (90 against 138). What
  it also shows: at 20% churn a day one post in two hundred is lost within a month; in a network of
  80 places a party forcing repairs at 10% while a tenth of the places leave every day brings the
  readable share to 99.6%; and a member publishing withheld generations makes repairers follow
  every day (61 generations per object in 30 days; section 16). *Built in S6:* load at scale,
  `hoi-sim scatter --load` (section 13).
- Browser tests: a post sent and rebuilt through three pretend nodes, a missing piece retried
  elsewhere, a regeneration picked up, the words of section 12, the Limits choice.
- Real network (`scripts/e2e-mailboxes.mjs`, built in S2): two `hoi-node --hold-pieces` holders,
  one on loopback and one on the machine's network address (two places as a browser counts
  them), two browsers with the WASM core and no relay: the owner's browser picks both as
  mailboxes, a member joins by invitation alone, posts go both ways, the holders keep only sealed
  objects and their notices say so. S3 adds forwarders to it.
- An outside review of the key schedule, AONT-RS, HPKE use, the sizing function and the candidate
  rules before S6, as for every new cryptographic path.

## 16. Open questions

- **Uptime without trusting claims.** Placement needs nodes seen up for long; a browser without a
  home node measures little first hand and leans on corroboration. Rosters from
  members' durable devices help, weighed by voices, opt-in, and capped at ⌈n/4⌉ of a generation
  through one introducer; in `hoi-sim scatter` a party with half the places, all of them coming
  in through rosters, holds at most that quarter and never rebuilds or withholds. Open: how much
  they help in a real network (few people may turn them on), and that publishing one tells the
  publishing device's links whose it is.
- **Members-only to MLS: who commits.** Settled for now: the records decide
  who is admitted or banned, and members refuse commits they do not back; the owner's and
  moderators' devices commit in a line ordered by how long their person has held the role, then
  by device age, each next one stepping in after an hour more of silence; of two commits of one
  epoch the one earlier in line counts, and members who followed the other go back within a day.
  Open: a committer ahead that commits more than a day after a later one's change stays alone on
  its branch (it catches up before committing, so only a device that commits by hand without
  reading would); the line is only as live as the moderators' devices; and the research's
  decentralised MLS proposals may still give a rule that needs no window.
- **Moderation versus blind storage.** Operators cannot inspect pieces; some operators will want
  to, for legal reasons. The notice sentence tells people; the choice to run a holder stays the
  operator's.
- **Drop key custody.** The community drop key is one secret shared by the committers' devices.
  A moderator removed later can still drop pieces stored under the old key until they
  expire, at most a month; a drop no removal explains is a loss, repaired under the new key (P8).
  A threshold signature among moderators would remove that, at a cost in complexity. A device
  that misses the message giving it the key makes a new one after its wait, a commit more than
  needed.
- **Anonymity.** Since S7 no single node links a device's address to its uploads or
  its own records: the first hop sees an envelope for another forwarder. Two nodes of one path
  together, or a global observer timing the entry's envelope against the injector's passing on,
  still can; a mix network is out of scope; a VPN stays the advice. A device's
  own post says "The node you connect through can't tell this post is yours" while that holds:
  its pointer records came back through an injector, its puts went in three hops, and the device
  never offered its person's records first. Completed 2026-11-06: an eligible home-only public post is placed by its paired home,
  verified and signed as pointers by the originating device. Authenticated home reports distinguish
  pending, injected and fallback forwarding. The home always knows its sender. Author drops now
  use three hops when a suitable path exists; shorter routes produce an explicit limitation notice.
- **Reaching a manifest's holders.** Settled: the simulation showed reach collapsing
  beyond a few hundred public nodes and wider vouch lists not helping; manifests list holders'
  keys and a forwarder passes a request one hop on to a link that vouches for its holder. Open:
  whether three hops for every read at scale is the cost to keep, or placement should prefer
  holders the reader population's forwarders know.
- **Generations spent by bad updates.** One regeneration a day is taken, and a member who
  publishes a generation onto its own places and withholds it makes honest repairers follow with
  another at once: about two a day, so an object's 255 generations last about four months under
  such a member, and each generation is larger than the last. Remembering a member whose
  generation never rebuilt anywhere, as mismatches already are, would end it; it needs a signal
  that the generation was withheld rather than unlucky.
- **Drop keys with several moderators.** Settled for private groups and open
  communities: the secret goes sealed to each committer's or moderator's device, and a
  new key comes when one leaves the line. Author capabilities now transfer per generation/locator, HPKE-encrypted to active devices
  of the same person and signed by an active device, through existing authenticated own-device
  links. The global device drop secret never transfers. Saved capabilities survive restart;
  the original device may then be offline. Revoked devices receive no new capabilities;
  already-copied capabilities cannot be made forgotten. New generations have fresh locators.

---

## Appendix: the arithmetic

With a party holding A of N places and a generation using M places in total, of which n are live:

```
rebuild   P(X ≥ k),       X ~ Hypergeometric(N, A, M)       (≈ Binomial(M, ℓ) for large N)
withhold  P(Y ≥ n−k+1),   Y ~ Hypergeometric(N, A, n)
readable  P(Z ≥ k),       Z ~ Binomial(n, uptime)
```

The rebuild bound uses M, not n, because a party keeps every piece it was ever given in a
generation, including those whose honest siblings were lost and re-made. That is why repair
re-placing the same indexes, as first drafted, broke the bound: with each of 3-of-6's indexes
placed t times, a 10% party holds it with probability 1 − 0.9ᵗ, and the chance of a rebuildable set
rises from 1.6% (t = 1) to 8.7% (2), 20% (3) and 48% (5). Fresh indexes, a fixed budget and fresh
generations keep it at the table's figures. `scatter-params.mjs` computes all of it exactly.


## Completion follow-ups · 2026-11-06

`home_scatter.rs` uses a bounded optional `home-scatter` message on paired links only.
`scatter-store` advertises actual placement capacity; ordinary homes without it retain the
existing whole-post path. Store operations validate size/expiry and are capped at 16 per
client under the global operation bound, with a two-minute timeout and ten-minute failure
backoff. The origin verifies the returned manifest against its own content before signing.
Receipts are reports from the trusted home, not anonymity proofs or guarantees of erasure.
Private group delivery continues to use its existing MLS/mailbox path.

`drop_share.rs` exchanges only generation-specific tokens with active, unrevoked,
unexpired same-person delegations. Payload signatures bind both devices and ciphertext;
the home cannot decrypt capability relays. Normal authorization validation applies to
periodic device-delegation announcements. Capability persistence is bounded and encrypted
by the existing vault. Holders still validate each token, and may retain copies.

Temporary forwarding dials reserve eight slots inside the global 32-peer bound; activity
and pending replies retain a link, and promoted community links release their temporary
slot. Setup now enables, inspects and disables mailboxes using the installed program,
retaining existing data and explaining the restart and repair consequences.
