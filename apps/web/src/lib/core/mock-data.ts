// Seed data for the dev mock: people, communities, posts and ledger.
// Ids here are labels; mock-core.ts turns them into stable hex keys. 'me' is whoever made the key.
import type { CommunityColor, Limits, Napping, LedgerAction, TrustLevel } from '@hoi/core-api';

export interface SeedPerson { id: string; name: string; prefix: string; trust: TrustLevel; ageDays: number }
export interface SeedCommunity {
  id: string;
  name: string;
  description: string;
  color: CommunityColor;
  sinceDays: number;
  owners: string[];
  threshold: number;
  mods: string[];
  limits?: Partial<Limits>;
  rules?: { text: string; version: number; hoursAgo: number; signedBy: string[] };
  pending?: string[];
  banned?: { key: string; reason: string; hoursAgo: number; auto: boolean }[];
  people: number;
  pinning: boolean;
}
export interface SeedPost {
  id: string;
  c: string;
  by: string;
  title: string;
  text: string;
  excerpt?: string;
  link?: string;
  hoursAgo: number;
  score: number;
  pinned?: boolean;
  napping?: Napping;
  labels?: string[];
  waiting?: boolean;
  receivedFrom?: number;
  /** Up-votes from your friends and friends of friends (why Hot puts it where it is). */
  friendsUp?: number;
  fofUp?: number;
}
export interface SeedComment {
  id: string;
  post: string;
  parent?: string;
  by: string;
  text: string;
  minutesAgo: number;
  score: number;
  removed?: { by: string; reason: string; ledger: string };
  napping?: Napping;
}
export interface SeedEntry {
  id: string;
  c: string;
  action: LedgerAction;
  by: string[];
  auto?: boolean;
  target?: { record: string } | { key: string };
  reason?: string;
  text?: string;
  hoursAgo: number;
  evidence?: number;
}
export interface SeedFlag {
  c: string;
  target: { record: string } | { key: string };
  reasons: string[];
  count: number;
  weight: number;
  hoursAgo: number;
}
export interface SeedProposal { code: string; c: string; patch: Partial<Limits>; signers: string[]; need: number }

export const PEOPLE: SeedPerson[] = [
  { id: 'mira', name: 'mira', prefix: 'a7f291c0', trust: 'friend', ageDays: 420 },
  { id: 'jonas', name: 'jonas.w', prefix: '88c10b3e', trust: 'fof', ageDays: 610 },
  { id: 'tobi', name: 'tobi_k', prefix: '3e1b70aa', trust: 'fof', ageDays: 300 },
  { id: 'lea', name: 'lea.h', prefix: 'b02e5d77', trust: 'friend', ageDays: 380 },
  { id: 'ptran', name: 'p.tran', prefix: '5a0db1e9', trust: 'fof', ageDays: 200 },
  { id: 'anon', name: 'anon_4c9', prefix: '4c9de2f1', trust: 'newcomer', ageDays: 3 },
  { id: 'skeller', name: 's.keller', prefix: 'd41e3c9a', trust: 'member', ageDays: 500 },
  { id: 'rav', name: 'rav', prefix: '02bb6f14', trust: 'member', ageDays: 340 },
  { id: 'spam', name: '', prefix: 'f30ac812', trust: 'newcomer', ageDays: 1 },
  { id: 'vendor', name: '', prefix: '9c21e0b4', trust: 'member', ageDays: 90 },
  { id: 'appeal', name: '', prefix: '7781aa02', trust: 'member', ageDays: 120 },
  { id: 'kmueller', name: 'k.mueller', prefix: '61d0a9e3', trust: 'newcomer', ageDays: 2 },
  { id: 'deals', name: 'cheap_vpn_deals', prefix: 'e7a1f00d', trust: 'newcomer', ageDays: 4 },
  { id: 'drill', name: 'drill_seller', prefix: '0f9e44b1', trust: 'newcomer', ageDays: 6 },
  { id: 'newbie', name: '', prefix: 'c3d8217f', trust: 'newcomer', ageDays: 0 },
];

const DAY_H = 24;

export const COMMUNITIES: SeedCommunity[] = [
  {
    id: 'privacy',
    name: 'privacy',
    description: 'Practical privacy for normal people. Threat models, tools, laws, and the occasional rant. Moderated by its members, carried by its peers.',
    color: 'mint',
    sinceDays: 291,
    owners: ['mira', 'jonas', 'rav', 'me'],
    threshold: 3,
    mods: ['mira', 'jonas', 'skeller', 'rav'],
    rules: {
      version: 7,
      hoursAgo: 6 * DAY_H,
      signedBy: ['mira', 'jonas', 'rav'],
      text: '1. Be kind. Argue with ideas, not people.\n2. No doxxing, ever. Not even "public" info.\n3. Sources for claims about specific apps or laws.\n4. Vendors must disclose. Astroturfing gets the key banned.\n5. Threat models beat hot takes.',
    },
    banned: [{ key: 'spam', reason: 'hit the rate limit 14× in an hour · 19 members flagged it', hoursAgo: 2, auto: true }],
    people: 4100,
    pinning: true,
  },
  {
    id: 'selfhosting',
    name: 'selfhosting',
    description: 'Run it yourself. Relays, home nodes, backups and the odd blinking LED.',
    color: 'sky',
    sinceDays: 230,
    owners: ['tobi'],
    threshold: 1,
    mods: ['tobi', 'lea'],
    limits: { maxRetention: 90 * 86_400 },
    rules: {
      version: 2,
      hoursAgo: 80 * DAY_H,
      signedBy: ['tobi'],
      text: '1. Share configs, not credentials.\n2. Say what hardware you run it on.\n3. No hosting-provider ads.',
    },
    people: 860,
    pinning: false,
  },
  {
    id: 'cryptography',
    name: 'cryptography',
    description: 'Primitives, protocols and patient explanations. ELI5 questions welcome.',
    color: 'lilac',
    sinceDays: 262,
    owners: ['skeller'],
    threshold: 1,
    mods: ['skeller'],
    limits: { powBits: 23 },
    rules: {
      version: 3,
      hoursAgo: 40 * DAY_H,
      signedBy: ['skeller'],
      text: "1. Explain, don't gatekeep.\n2. Cite papers for strong claims.\n3. Don't roll your own crypto in production. Do it here, for fun.",
    },
    people: 1900,
    pinning: false,
  },
  {
    id: 'hackspace',
    name: 'hackspace',
    description: "Our hackspace's corner: meetups, tools to borrow, who's fixing what.",
    color: 'peach',
    sinceDays: 170,
    owners: ['lea'],
    threshold: 1,
    mods: ['lea', 'me'],
    limits: { maxRetention: 30 * 86_400, admission: 'approve', newcomerPostsPerDay: 1 },
    rules: {
      version: 1,
      hoursAgo: 160 * DAY_H,
      signedBy: ['lea'],
      text: "1. Return what you borrow.\n2. Meetups are Thursdays, back room.\n3. Ask a mod before posting other people's photos.",
    },
    pending: ['kmueller', 'newbie'],
    people: 64,
    pinning: true,
  },
  {
    id: 'opsec',
    name: 'opsec',
    description: 'Operational security for activists, journalists and the merely curious.',
    color: 'sun',
    sinceDays: 70,
    owners: ['me'],
    threshold: 1,
    mods: ['me', 'rav'],
    limits: { powBits: 24, maxRetention: 30 * 86_400 },
    rules: {
      version: 1,
      hoursAgo: 68 * DAY_H,
      signedBy: ['me'],
      text: '1. Assume your adversary reads this.\n2. No operational details about real, ongoing actions.\n3. Be generous with beginners.',
    },
    people: 210,
    pinning: true,
  },
];

export const POSTS: SeedPost[] = [
  {
    id: 'p-start', c: 'privacy', by: 'mira', hoursAgo: 30 * DAY_H, score: 540, pinned: true,
    title: 'Start here: threat modelling in 10 minutes, and the rules explained',
    text: 'Welcome. Before you ask "which app is the most private", answer three questions: who might want your data, what they could do with it, and how much effort they would spend.\n\nWrite the answers down. That is your threat model. Most of the arguments in here happen because two people have different ones and don\'t say so.\n\nThe rules are on the Rules tab. Rule 4 is new: if you sell something, say so.',
  },
  {
    id: 'p-messengers', c: 'privacy', by: 'mira', hoursAgo: 2, score: 312, receivedFrom: 63, friendsUp: 2, fofUp: 3,
    title: 'Which messengers actually survived the 2026 metadata audit? A field summary.',
    excerpt: 'Only three apps close the gap between "encrypted" and "doesn\'t know who you talk to". Details and the scoring inside, please argue with it.',
    text: 'Spent the last month going through every published audit I could find. Short version: the gap between "end-to-end encrypted" and "doesn\'t know who you talk to" is enormous, and only three apps close it.\n\nMethodology. I only counted audits with a public scope document and a named firm. Marketing pages were ignored. Where an app changed architecture mid-year, I scored the version shipping on 1 October.\n\nWhat "survived" means. No social graph server-side, sealed sender on by default, and push notifications that carry no message or sender. Miss one and it didn\'t survive.\n\nFull table and the raw notes are in the comments. Please argue with the scoring. That\'s the point.',
  },
  {
    id: 'p-cosign', c: 'privacy', by: 'mira', hoursAgo: 3, score: 14,
    title: 'Owners: cosign the newcomer-limit change?',
    text: "We've had a wave of new keys from the audit thread. I'd like to raise the newcomer limit from 2 to 3 posts a day so people can answer follow-ups.\n\njonas.w and I have signed. Owners, paste this under Manage → Cosign a proposal:\n\nhoi://cosign?p=7a1e44c0b9d2f3a8\n\nIt needs 3 of 4.",
  },
  {
    id: 'p-bill', c: 'privacy', by: 'jonas', hoursAgo: 6, score: 128,
    title: 'Data-retention bill, second draft: what it would mean for people running relays',
    text: 'The second draft dropped the worst part (logging by default for anyone forwarding traffic), but the definition of "communication service" is still wide enough to catch a relay with more than 10,000 people on it.\n\nRelays keep nothing, so there would be nothing to hand over. The question is whether running one would come with paperwork. I wrote to the committee; the template is in the comments.',
  },
  {
    id: 'p-spam', c: 'privacy', by: 'spam', hoursAgo: 8, score: -4,
    napping: { label: 'likely spam', by: 'Spamwatch' },
    title: 'Buy the only phone that keeps you 100% private, link in bio',
    text: 'DM for a discount code. Ships worldwide. 100% private, guaranteed.',
  },
  {
    id: 'p-age', c: 'privacy', by: 'ptran', hoursAgo: 24, score: 76, labels: ['needs a threat model'],
    title: 'Audit results for an age-verification app: what it proves and what it leaks',
    text: "The app proves you're over 18 without telling the site who you are. The audit says the proof itself holds up. What leaks is timing: the wallet phones home when it refreshes credentials, and that call carries a stable identifier.\n\nNot a dealbreaker, but worth knowing before a community turns on age-proof admission.",
  },
  {
    id: 'p-relay', c: 'selfhosting', by: 'tobi', hoursAgo: 5, score: 97, link: 'https://notes.tobik.example/relay-month',
    title: "My relay passed along 2 TB this month. Here's what it cost, and what it saw: nothing.",
    text: 'Numbers first: 2.1 TB passed along, about 3 kWh of electricity on a little ARM box, zero bytes on disk.\n\nWhat it saw: node ids, packet sizes and timing. Never a post, never who wrote what. The routing table lives in memory and is gone when the process restarts.\n\nWriteup with graphs at the link.',
  },
  {
    id: 'p-vps', c: 'selfhosting', by: 'lea', hoursAgo: 50, score: 33,
    title: 'hoi-node on the cheapest VPS: my systemd unit and backup cron',
    text: 'Sharing my setup for a home node that keeps c/hackspace warm for our phones. The backup is just the database file, encrypted, once a night.\n\n[Service]\nExecStart=/usr/local/bin/hoi-node --pin hackspace\nRestart=always',
  },
  {
    id: 'p-eli5', c: 'cryptography', by: 'anon', hoursAgo: 9, score: 64,
    title: "ELI5: how can a relay pass my post along if it can't read it?",
    text: "Everyone says relays can't read anything. But it's literally sending my post along. How?",
  },
  {
    id: 'p-blake', c: 'cryptography', by: 'skeller', hoursAgo: 70, score: 45,
    title: 'Why BLAKE3 for the stamp and not SHA-256?',
    text: "Short answer: speed on small inputs, and it's already what ids use, so there is one hash function to audit instead of two.",
  },
  {
    id: 'p-meetup', c: 'hackspace', by: 'lea', hoursAgo: 12, score: 41,
    title: "Meetup Thursday: bring a laptop, we'll set up relays together (back room, 7pm)",
    text: "Bring a laptop and a charger. We'll set up the desktop app, turn on relaying and pair everyone's phones to it. Pizza if more than six people show up.",
  },
  {
    id: 'p-borrow', c: 'hackspace', by: 'kmueller', hoursAgo: 1, score: 0, waiting: true,
    title: 'Hi! Can I borrow the soldering station on Saturday?',
    text: "New here, found you through lea's poster. I'd bring it back the same evening.",
  },
  {
    id: 'p-drills', c: 'hackspace', by: 'drill', hoursAgo: 4, score: -2,
    title: 'Selling cordless drills, best price, DM',
    text: 'Best price in town. DM me.',
  },
  {
    id: 'p-border', c: 'opsec', by: 'rav', hoursAgo: 26, score: 58,
    title: 'Travel checklist: what I wipe before crossing a border',
    text: "Phone: log out of everything, move my key to the desktop, revoke the phone's device key. Laptop: full-disk encryption on, powered off, not asleep.\n\nThe nice part is the revoke: the phone can't sign as me while I'm on the road, and I bless it again when I'm home.",
  },
  {
    id: 'p-family', c: 'opsec', by: 'mira', hoursAgo: 60, score: 23,
    title: 'Is a password manager on a shared family laptop worth it?',
    text: 'Asking for my parents. One laptop, three people, everyone knows the login password.',
  },
];

export const COMMENTS: SeedComment[] = [
  { id: 'c-jonas', post: 'p-messengers', by: 'jonas', minutesAgo: 60, score: 96,
    text: 'Great work. One pushback: "push carries no sender" is doable with a self-hosted push relay, which two of the apps you failed support but don\'t default to. Should defaults count, or capability?' },
  { id: 'c-mira', post: 'p-messengers', parent: 'c-jonas', by: 'mira', minutesAgo: 52, score: 71,
    text: "Defaults. The audience for this list is my dad, not us. If it takes a config flag, 99% of people are unprotected and the app shouldn't get credit." },
  { id: 'c-tobi', post: 'p-messengers', parent: 'c-mira', by: 'tobi', minutesAgo: 40, score: 38,
    text: 'Agree with defaults, but add a second column: "survives with a self-hosted relay". That\'s the exact crowd of c/selfhosting next door.' },
  { id: 'c-ptran', post: 'p-messengers', by: 'ptran', minutesAgo: 61, score: 54,
    text: 'Would be useful to know which audits were paid for by the vendor. Not disqualifying, but it changes how I read "no findings".' },
  { id: 'c-vendor', post: 'p-messengers', by: 'vendor', minutesAgo: 50, score: 3,
    text: 'Honestly the best one is the one I work on, here is a link.',
    removed: { by: 'jonas', reason: 'rule 4 (undisclosed vendor)', ledger: 'e-remove' } },
  { id: 'c-lea', post: 'p-messengers', by: 'lea', minutesAgo: 120, score: 22,
    text: 'Can we pin this to the wiki once the scoring settles? Would save answering the same question every week.' },
  { id: 'c-first', post: 'p-messengers', by: 'anon', minutesAgo: 20, score: -1,
    napping: { label: 'off-topic', by: 'the mods' },
    text: 'first!! also does anyone have an invite to c/opsec' },
  { id: 'c-envelope', post: 'p-eli5', by: 'skeller', minutesAgo: 8 * 60, score: 31,
    text: 'Think of a sealed envelope inside another sealed envelope. The relay reads the outer one (which device to hand it to) and passes it on. Only the device at the end can open the inner one.' },
  { id: 'c-thanks', post: 'p-eli5', parent: 'c-envelope', by: 'anon', minutesAgo: 7 * 60, score: 6,
    text: "oh. that's actually simple. thanks!" },
  { id: 'c-pis', post: 'p-meetup', by: 'tobi', minutesAgo: 10 * 60, score: 9,
    text: "I'll bring two spare Raspberry Pis for anyone without a laptop." },
  { id: 'c-graphs', post: 'p-relay', by: 'lea', minutesAgo: 4 * 60, score: 12, text: 'What did you use for the graphs?' },
  { id: 'c-gnuplot', post: 'p-relay', parent: 'c-graphs', by: 'tobi', minutesAgo: 3 * 60, score: 8,
    text: "Plain gnuplot reading the relay's in-memory counters once a minute. Nothing hits the disk." },
  { id: 'c-family', post: 'p-start', by: 'rav', minutesAgo: 25 * 24 * 60, score: 40,
    text: 'Bookmarking. The three-question version is the one I send to my family.' },
  { id: 'c-template', post: 'p-bill', by: 'mira', minutesAgo: 5 * 60, score: 17,
    text: 'Thanks for writing to them. Can you post the template?' },
  { id: 'c-fade', post: 'p-border', by: 'jonas', minutesAgo: 20 * 60, score: 11,
    text: "Add: check which of your communities have short fade windows, so you know what's still around when you're back." },
];

export const LEDGER: SeedEntry[] = [
  { id: 'e-unban', c: 'privacy', action: 'unban', by: ['rav'], target: { key: 'appeal' }, hoursAgo: 31 * DAY_H,
    reason: 'appeal accepted, the duplicate was a client bug. Sorry!' },
  { id: 'e-modadd', c: 'privacy', action: 'mod-add', by: ['mira', 'jonas'], target: { key: 'rav' }, hoursAgo: 24 * DAY_H,
    reason: 'Welcomed rav as a moderator' },
  { id: 'e-rules', c: 'privacy', action: 'rules', by: ['mira', 'jonas', 'rav'], hoursAgo: 6 * DAY_H,
    reason: 'Added rule 4, vendor disclosure' },
  { id: 'e-label', c: 'privacy', action: 'label', by: ['skeller'], target: { record: 'p-age' }, hoursAgo: 5,
    reason: 'needs a threat model · shown as a note, not hidden' },
  { id: 'e-autoban', c: 'privacy', action: 'ban', by: [], auto: true, target: { key: 'spam' }, hoursAgo: 2, evidence: 19,
    reason: 'hit the rate limit 14× in an hour · 19 members flagged it' },
  { id: 'e-remove', c: 'privacy', action: 'remove', by: ['jonas'], target: { record: 'c-vendor' }, hoursAgo: 35 / 60,
    reason: 'rule 4, undisclosed vendor' },
  { id: 'e-sh-rules', c: 'selfhosting', action: 'rules', by: ['tobi'], hoursAgo: 80 * DAY_H, reason: 'No hosting-provider ads' },
  { id: 'e-cr-rules', c: 'cryptography', action: 'rules', by: ['skeller'], hoursAgo: 40 * DAY_H, reason: 'Rule 3, for fun' },
  { id: 'e-hs-rules', c: 'hackspace', action: 'rules', by: ['lea'], hoursAgo: 160 * DAY_H, reason: 'First rules' },
  { id: 'e-hs-mod', c: 'hackspace', action: 'mod-add', by: ['lea'], target: { key: 'me' }, hoursAgo: 120 * DAY_H, reason: 'Welcome aboard' },
  { id: 'e-hs-admit', c: 'hackspace', action: 'admit', by: ['lea'], target: { key: 'drill' }, hoursAgo: 6 * DAY_H },
  { id: 'e-op-rules', c: 'opsec', action: 'rules', by: ['me'], hoursAgo: 68 * DAY_H, reason: 'First rules' },
  { id: 'e-op-mod', c: 'opsec', action: 'mod-add', by: ['me'], target: { key: 'rav' }, hoursAgo: 50 * DAY_H, reason: 'rav helps with the queue' },
];

export const FLAGS: SeedFlag[] = [
  { c: 'privacy', target: { record: 'c-first' }, reasons: ['spam'], count: 3, weight: 0.6, hoursAgo: 0.25 },
  { c: 'privacy', target: { key: 'deals' }, reasons: ['spam', 'rule:4'], count: 7, weight: 4.25, hoursAgo: 3 },
  { c: 'privacy', target: { record: 'p-age' }, reasons: ['rule:5'], count: 2, weight: 1.5, hoursAgo: 20 },
  { c: 'hackspace', target: { record: 'p-drills' }, reasons: ['spam'], count: 4, weight: 2.2, hoursAgo: 2 },
];

/** Rules and limits changes the community is deciding (PROTOCOL.md §6.1). */
export interface SeedConsent {
  id: string;
  c: string;
  by: string[];
  hoursAgo: number;
  patch: Partial<Omit<Limits, 'ladder'>>;
  ladder: Partial<Limits['ladder']>;
  agreed: string[];
}

export const CONSENTS: SeedConsent[] = [
  { id: 'e-sh-fade', c: 'selfhosting', by: ['tobi'], hoursAgo: 30, patch: { maxRetention: 60 * 86_400 }, ladder: { hideQuorum: 4 }, agreed: ['lea'] },
];

export const PROPOSALS: SeedProposal[] = [
  { code: 'hoi://cosign?p=7a1e44c0b9d2f3a8', c: 'privacy', patch: { newcomerPostsPerDay: 3 }, signers: ['mira', 'jonas'], need: 3 },
];
