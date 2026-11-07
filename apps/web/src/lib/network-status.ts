import type { NetStatus } from '@hoi/core-api';

export interface SyncStatus {
  state: 'checking' | 'offline' | 'degraded' | 'connected';
  title: string;
  detail: string;
  action: string;
}

/** App posting policy only. A matching live peer is not a delivery receipt. */
export function canPublishPublic(net: NetStatus | null, community?: string): boolean {
  return !!community && !!net && net.peers > 0 && !!net.communities?.some(item => item.community === community && item.connectedPeers > 0);
}

/** A browser or phone with no home node of its own: nobody can reach it, so what it posts is
 * readable by others only once a home node carrying the community has it. */
export function postsThroughOthers(net: NetStatus | null, platform: string): boolean {
  return (platform === 'web' || platform === 'mobile') && !net?.homeNode;
}

/** Nodes connected now that offered this exact community: where a post from here can go. */
export function carriersOf(net: NetStatus | null, community?: string): number {
  return (community && net?.communities?.find(item => item.community === community)?.connectedPeers) || 0;
}

/** Connectivity is not a receipt: a socket can be open before any community has synced. */
export function syncStatus(net: NetStatus | null, community?: string): SyncStatus {
  if (!net) return {
    state: 'checking', title: 'Checking connections',
    detail: "Saved posts can be read offline. Posting and new updates need a home node that carries this community: yours, a member's or a public one.",
    action: 'Connection settings',
  };
  const error = net.homeNode?.error || net.transport?.error;
  const homeUnavailable = !!net.homeNode && !net.homeNode.connected;
  if (net.peers === 0) return {
    state: 'offline', title: 'Reading saved posts',
    detail: `${error ? `${error} ` : ''}Saved posts are readable offline. New updates cannot be sent or received until a home node carrying this community connects: yours, a member's or a public one.${homeUnavailable ? ' To use your own, check its address and browser permissions.' : ''}`,
    action: 'Fix connection',
  };
  const scope = community ? net.communities?.find((item) => item.community === community) : undefined;
  if (community && net.communities && !scope?.connectedPeers) return {
    state: 'degraded', title: 'Connected, but not to this community',
    detail: `${error ? `${error} ` : ''}Saved posts remain readable. Posting and new updates need a node with this exact community ID. Open the same community invite, or a node link from someone who runs a node that carries it.`,
    action: 'Check connection',
  };
  if (community && !net.communities) return {
    state: 'degraded', title: 'Community sync not confirmed',
    detail: `${error ? `${error} ` : ''}Other devices are connected, but this node has not reported whether any share this community's ID. Use the same community invite on both devices. Delivery is not confirmed.`,
    action: 'Connection details',
  };
  if (error || homeUnavailable) return {
    state: 'degraded', title: 'Some connections are unavailable',
    detail: `${error || 'Your home node is not connected.'} ${scope ? `${scope.connectedPeers} ${scope.connectedPeers === 1 ? 'peer has' : 'peers have'} offered this community for sync` : `${net.peers} ${net.peers === 1 ? 'other peer is' : 'other peers are'} connected`}. Delivery is not confirmed.`,
    action: 'Check connection',
  };
  return {
    state: 'connected', title: scope ? 'Connected to this community' : 'Connected for sync',
    detail: scope
      ? `${scope.connectedPeers} ${scope.connectedPeers === 1 ? 'peer has' : 'peers have'} offered this community's exact ID for sync. A connection is not a delivery receipt.`
      : `${net.peers} ${net.peers === 1 ? 'peer is' : 'peers are'} connected. Only matching community IDs sync; a connection is not a delivery receipt.`,
    action: 'Connection details',
  };
}

/** Used only after the core accepted a signed action locally. A browser or phone without its own
 * home node says who can read it: nobody, until a home node carrying the community has it. */
export function savedForSync(net: NetStatus | null, community?: string, platform = 'desktop'): string {
  const others = postsThroughOthers(net, platform) ? ' Nobody else can read it until a home node carrying this community has it.' : '';
  if (!net || net.peers === 0) return `Saved on this device. Waiting for a connection to sync.${others}`;
  if (community) {
    if (!net.communities) return `Saved on this device. This community's sync connection is not confirmed.${others}`;
    if (!net.communities.some((item) => item.community === community && item.connectedPeers > 0)) {
      return `Saved on this device. Waiting for a peer that carries this community.${others}`;
    }
  }
  if (others) return 'Sent to the home nodes carrying this community; others read it from there. Delivery is not confirmed.';
  return 'Saved on this device. Available for peer sync; delivery is not confirmed.';
}
