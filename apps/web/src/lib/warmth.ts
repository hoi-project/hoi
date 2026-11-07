// Warmth in words (PROTOCOL.md §20). The core reads the numbers from this device's own links;
// this turns them into a badge, a sentence and a few reasons. Everything here describes copies
// this device could see besides its own, never whether a post reached anyone. Copies are
// counted once per independent place (one network, one operator), and copies or links that are
// not encrypted and checked count for less; the words say "places" and "checked links", never
// the technical terms behind them.
import type { CommunityWarmth, LinkWarmth, WarmthLevel } from '@hoi/core-api';
import { ago } from '@hoi/core-api';

export interface WarmthWords {
  word: WarmthLevel;
  /** A tag class: cold peach, cool sun, warm mint, hot cobalt. The word is always shown too. */
  tag: string;
  /** Availability elsewhere, followed by what is saved on this device. */
  summary: string;
  /** Up to three plain reasons behind the reading. */
  reasons: string[];
  /** What would warm it up, when something would. */
  advice?: string;
}

export const WARMTH_TAGS: Record<WarmthLevel, string> = { cold: 'tag-peach', cool: 'tag-sun', warm: 'tag-mint', hot: 'tag-trust' };

const n = (v: number, one: string, many = `${one}s`) => `${v} ${v === 1 ? one : many}`;
const copies = (v: number) => `${Number.isInteger(v) ? v : v.toFixed(1)} ${v === 1 ? 'copy' : 'copies'}`;
const percent = (v: number) => `${Math.round(v * 100)}%`;

/** Durable copies besides this device's own: what the level and score are read from. */
export const othersDurable = (w: CommunityWarmth): number => Math.max(0, Math.round((w.durableCopies - (w.ownDurable ? 1 : 0)) * 10) / 10);

export function warmthWords(w: CommunityWarmth, extras: { announced?: number; platformName?: string } = {}): WarmthWords {
  const device = extras.platformName ?? 'browser';
  const others = othersDurable(w);
  const reasons: string[] = [];
  let summary: string;
  let advice: string | undefined;
  if (w.pending) {
    summary = 'Waiting to download this community.';
    reasons.push('Its signed community details have not arrived yet.');
    if (!w.live) advice = 'Connect to a public node or open a member’s invite.';
  } else {
    const here = w.complete >= 1 ? `All known records are saved ${device === 'browser' ? 'in' : 'on'} this ${device}.` : `This ${device} has ${percent(w.complete)} of the known records.`;
    // Nodes that share a network or an operator are one place: say so where it matters.
    const together = w.places > 0 && w.places < w.holdersDurable ? ` They are in ${n(w.places, 'independent place')}.` : '';
    if (others === 0) {
      summary = `No ${w.ownDurable ? 'other ' : ''}node is currently known to have a copy. ${here}`;
    } else if (others === w.holdersDurable) {
      summary = `${n(w.holdersDurable, 'other node')} ${w.holdersDurable === 1 ? 'has' : 'have'} a copy.${together} ${here}`;
    } else {
      summary = `${n(w.holdersDurable, 'other node')} ${w.holdersDurable === 1 ? 'holds' : 'hold'} parts of this community (${copies(others)} in total).${together} ${here}`;
    }
    if (others < 1) advice = w.ownDurable
      ? 'Invite someone who runs a node, or ask a public node to keep a copy.'
      : 'Ask a public node to keep a copy, or run a node on your own computer.';
    else if (w.kept < 2 && w.places < w.holdersDurable) advice = 'A copy on a node somewhere else, run by someone else, would help more than another node in the same place.';
    else if (others < 2) advice = 'Another node with a full copy would help keep this community available if one goes offline.';
  }
  if (!w.pending && w.places > 0 && w.places < w.holdersDurable) {
    reasons.push('Some of these nodes share a network or an operator, so they count once: one outage or one person could take them together.');
  }
  if (!w.pending && others > 0 && w.guarded < 1) {
    reasons.push(`${percent(1 - w.guarded)} of these copies are reached over links that are not encrypted or whose other end was not checked, so they count for less.`);
  }
  if (!w.pending && w.missing > 0) reasons.push(`${n(w.missing, 'record')} still to download.`);
  if (w.media < 1) reasons.push(`${percent(w.media)} of the pictures referenced by saved posts are downloaded.`);
  if (!w.fresh) reasons.push(w.lastSync ? `No device sharing this community is connected. Last sync: ${ago(w.lastSync)} ago.` : 'This device has not finished syncing with another device yet.');
  if (w.holdersTransient > 0) reasons.push(`${n(w.holdersTransient, 'browser or phone', 'browsers or phones')} also ${w.holdersTransient === 1 ? 'has' : 'have'} records, but can only share while hoi is open.`);
  if (w.networkCopies > others) reasons.push(`A connected node reports ${copies(w.networkCopies)} on other nodes. These are not included in the warmth score.`);
  if (w.carried) reasons.push(w.carried === 'index' ? 'This device also shares the community because it is listed in a registry.' : 'This device also shares the community at someone’s request.');
  if (extras.announced) reasons.push(`The registry lists ${n(extras.announced, 'public node')} for this community; they may be offline.`);
  return { word: w.level, tag: WARMTH_TAGS[w.level], summary, reasons: reasons.slice(0, 3), advice };
}

export interface DirectoryFacts {
  registryConfigured: boolean;
  registryReachable: boolean;
  /** Public nodes the registry listed: leads, not connections. */
  announcedNodes: number;
  /** Connected nodes sharing a catalog of open communities. */
  catalogNodes: number;
  discoverable: number;
  carryNodes: number;
}

export interface DirectoryWords {
  level: WarmthLevel;
  word: WarmthLevel;
  tag: string;
  summary: string;
}

/** How findable communities are from here: a registry and connected nodes are separate sources,
 * and either can stand in for the other. A registry's list is a lead, not a
 * connection, so listings alone never read warmer than cool. */
export function directoryWarmth(f: DirectoryFacts): DirectoryWords {
  const registry = f.registryConfigured && f.registryReachable;
  let level: WarmthLevel;
  if (registry && f.announcedNodes >= 2 && f.catalogNodes >= 1) level = 'hot';
  else if (f.catalogNodes >= 1 && (registry || f.catalogNodes >= 2)) level = 'warm';
  else if (registry || f.catalogNodes >= 1) level = 'cool';
  else level = 'cold';
  const parts: string[] = [];
  if (!f.registryConfigured) parts.push('no registry in this build');
  else parts.push(f.registryReachable ? `the registry answers and lists ${n(f.announcedNodes, 'public node')}` : 'the registry is not reachable right now');
  parts.push(f.catalogNodes > 0 ? `${n(f.catalogNodes, 'connected node')} share${f.catalogNodes === 1 ? 's' : ''} ${n(f.discoverable, 'open community', 'open communities')}` : 'no connected node shares a directory');
  if (f.carryNodes > 0) parts.push(`${n(f.carryNodes, 'connected node')} take${f.carryNodes === 1 ? 's' : ''} requests to carry communities`);
  const summary = `${parts.join('; ')}.`.replace(/^./, (c) => c.toUpperCase());
  return { level, word: level, tag: WARMTH_TAGS[level], summary };
}

export interface LinkWords {
  word: WarmthLevel;
  tag: string;
  summary: string;
  advice?: string;
}

/** The warmth of this device's own connections, in plain words: how many independent places
 * it is linked through, whether one carries most of its links, how many links are checked. */
export function linkWarmthWords(l: LinkWarmth, platformName = 'browser'): LinkWords {
  if (l.links === 0) {
    return { word: l.level, tag: WARMTH_TAGS[l.level], summary: `This ${platformName} is not connected to anyone right now.` };
  }
  const parts = [`Connected through ${n(l.places, 'independent place')}${l.links > l.places ? ` (${n(l.links, 'link')})` : ''}.`];
  if (l.places === 1) parts.push(`Everything this ${platformName} sends goes through one place.`);
  else if (l.largest > 0.5) parts.push('One of them carries most of your links.');
  parts.push(l.guarded >= 1 ? 'Every link is encrypted and its other end checked.' : `${percent(l.guarded)} of links are encrypted and checked.`);
  let advice: string | undefined;
  if (l.guarded < 0.5) advice = 'Most links are not encrypted or not checked. Prefer nodes reached by their own link or invite.';
  else if (l.places === 1 || l.largest > 0.5) advice = 'A public node somewhere else, or a member’s invite, would spread your links over more places.';
  return { word: l.level, tag: WARMTH_TAGS[l.level], summary: parts.join(' '), advice };
}
