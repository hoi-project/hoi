import type { CommunitySummary, HoiCore, NetStatus, PrivateGroup } from '@hoi/core-api';
import { canPublishPublic } from './network-status';

const RETRY_MS = 5 * 60_000;
const MAX_BATCH = 16;
type DiscoveryCore = Pick<HoiCore, 'community' | 'publicJoin'>;
export interface PublicConnectionResult {
  checked: number;
  requested: number;
  routes: number;
  message: string;
  /** Whether the registry answered: this run or within the last five minutes. `unchecked` when
   * nothing needed it. Feeds the directory's Warmth reading, never the posting policy. */
  registry: 'unchecked' | 'reachable' | 'unreachable';
  /** Public node routes the registry advertised at that check, whatever device they fit. */
  announced: number;
  /** Communities whose own trackers gave routes to try this run. */
  tracked: number;
}

/** Asks one tracker which public nodes carry a community (`GET /v2/nodes?community=`). */
export type TrackerNodes = (tracker: string, community: string) => Promise<string[]>;

export function publicRoutes(routes: string[], platform: string): string[] {
  return routes.filter(route => platform === 'web' ? route.startsWith('rtc:') : !route.startsWith('rtc:'));
}

/** Refresh public infrastructure hints without changing membership or touching private groups.
 * The core still verifies routes, admission and every signed record. Transport retries use its
 * persisted hints; directory requests are bounded separately so net events cannot flood it.
 * A community that names trackers has them asked first, in any build: the community
 * chose them, the app didn't. An app built with a registry then asks it too; without one (the
 * published app), public nodes are otherwise found through the nodes this device reaches. */
export class PublicConnectionDiscovery {
  #attempts = new Map<string, number>();
  #applied = new Map<string, string>();
  #metadata: { at: number; publicNodes: string[] } | undefined;
  #generation = 0;

  constructor(private metadata: (() => Promise<{ publicNodes: string[] }>) | null, private now = Date.now, private trackerNodes: TrackerNodes | null = null) {}

  reset(): void { this.#generation++; this.#attempts.clear(); this.#applied.clear(); this.#metadata = undefined; }

  async connect(core: DiscoveryCore, communities: readonly CommunitySummary[], privateGroups: readonly PrivateGroup[], platform: string,
    net: NetStatus | null, options: { community?: string; force?: boolean; active?: () => boolean } = {}): Promise<PublicConnectionResult> {
    const generation = this.#generation;
    const active = () => generation === this.#generation && (options.active?.() ?? true);
    const result: PublicConnectionResult = {
      checked: 0, requested: 0, routes: 0, message: '',
      registry: this.#metadata ? 'reachable' : 'unchecked', announced: this.#metadata?.publicNodes.length ?? 0, tracked: 0,
    };
    const privateIds = new Set(privateGroups.map(group => group.community));
    const joined = new Set(communities.filter(c => c.joined && !privateIds.has(c.id)).map(c => c.id));
    for (const id of this.#attempts.keys()) if (!joined.has(id)) { this.#attempts.delete(id); this.#applied.delete(id); }
    const now = this.now();
    const candidates = communities.filter(c => c.joined && !privateIds.has(c.id) && (!options.community || options.community === c.id)
      && !canPublishPublic(net, c.id) && (options.force || now - (this.#attempts.get(c.id) ?? -Infinity) >= RETRY_MS)).slice(0, MAX_BATCH);
    if ((!this.metadata && !this.trackerNodes) || !candidates.length || !active()) return result;
    const eligible: CommunitySummary[] = [];
    const trackersOf = new Map<string, string[]>();
    for (const candidate of candidates) {
      if (!active()) return result;
      this.#attempts.set(candidate.id, now);
      result.checked++;
      try {
        const detail = await core.community(candidate.id);
        if (detail.joined && detail.limits.admission === 'open') {
          eligible.push(candidate);
          trackersOf.set(candidate.id, detail.trackers ?? []);
        }
      } catch { /* Leaving a community during discovery must not rejoin it. */ }
    }
    if (!eligible.length || !active()) return result;
    // the community's own trackers first: only asked because no route this device holds answered
    if (this.trackerNodes) {
      for (const candidate of eligible) {
        const trackers = trackersOf.get(candidate.id) ?? [];
        if (!trackers.length || !active()) continue;
        const routes: string[] = [];
        for (const tracker of trackers) {
          try {
            for (const r of publicRoutes(await this.trackerNodes(tracker, candidate.id), platform)) if (!routes.includes(r)) routes.push(r);
          } catch { /* a tracker that doesn't answer is one of two; the registry and node lists remain */ }
          if (!active()) return result;
        }
        const signature = `t:${routes.join('\n')}`;
        if (!routes.length || (!options.force && this.#applied.get(candidate.id) === signature)) continue;
        const current = await core.community(candidate.id);
        if (!active() || !current.joined || current.limits.admission !== 'open') continue;
        await core.publicJoin(candidate.id, routes);
        this.#applied.set(candidate.id, signature);
        result.tracked++;
        result.requested++;
      }
      if (result.tracked) result.message = 'Asking public nodes this community’s trackers named. Nobody can read what you post from here until one has it.';
    }
    if (!this.metadata) return result;
    const metadata = this.metadata;
    let asking = false;
    try {
      if (options.force || !this.#metadata || now - this.#metadata.at >= RETRY_MS) {
        asking = true;
        const answer = await metadata();
        if (!active()) return result;
        this.#metadata = { at: now, publicNodes: answer.publicNodes };
        asking = false;
      }
      result.registry = 'reachable';
      result.announced = this.#metadata.publicNodes.length;
      const routes = publicRoutes(this.#metadata.publicNodes, platform);
      result.routes = routes.length;
      if (!routes.length) {
        result.message = 'No public relays are advertised for this device yet. Saved posts remain readable. Ask a member for an invite with a reachable relay.';
        return result;
      }
      for (const candidate of eligible) {
        if (!active()) return result;
        const signature = routes.join('\n');
        // publicJoin is also an explicit transport retry. Do not reset backoff or evict
        // a working member connection merely because a periodic check saw the same routes.
        if (!options.force && this.#applied.get(candidate.id) === signature) continue;
        // Recheck after the directory request: a user may have left while it was in flight.
        const current = await core.community(candidate.id);
        if (!active() || !current.joined || current.limits.admission !== 'open') continue;
        await core.publicJoin(candidate.id, routes);
        if (!active()) return result;
        this.#applied.set(candidate.id, signature);
        result.requested++;
      }
      result.message = result.requested ? 'Looking for a public node carrying your community. Nobody can read what you post from here until one has it.' : '';
    } catch {
      if (active()) {
        result.message = 'Public relay discovery is unavailable. Saved posts and existing connections still work; try again or open a member’s invite.';
        // A fresh failure outranks a cached answer; a failed join says nothing about the registry.
        if (asking) result.registry = 'unreachable';
      }
    }
    return result;
  }
}
