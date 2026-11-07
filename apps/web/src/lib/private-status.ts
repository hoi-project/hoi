import type { PrivateGroup } from '@hoi/core-api';
import type { PrivateSyncStatus } from './private-relay';

/** How a private group syncs right now, in the words the app shows. */
export interface PrivateLine {
  /** One or two sentences for the group's panel and the sync notice. */
  text: string;
  /** Short label for the status button in the top bar. */
  label: string;
  connected: boolean;
  trouble: boolean;
  /** The group syncs through an encrypted relay (the "Sync" button applies). */
  relay: boolean;
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/**
 * Mailboxes sync on their own inside the core; a relay only while they are still being chosen,
 * on a build that has one.
 */
export function privateLine(group: PrivateGroup, sync: PrivateSyncStatus | undefined, relays: number, device: string): PrivateLine {
  if (!group.active) return { text: 'This device is no longer a member. Messages it already received can remain here.', label: 'Former private member', connected: false, trouble: false, relay: false };
  if (group.transport === 'mailboxes') {
    const total = group.mailboxes.length;
    const linked = group.mailboxes.filter(m => m.linked).length;
    const waiting = group.pending ? ` ${plural(group.pending, 'update')} still going out.` : ' Nothing waiting to go out.';
    if (!linked) return { text: `None of the group’s ${plural(total, 'mailbox', 'mailboxes')} can be reached right now. Updates wait on this ${device} and go out once one can.`, label: 'Private mailboxes unreachable', connected: false, trouble: true, relay: false };
    return { text: `Encrypted mailboxes: ${linked} of ${total} reachable.${waiting}`, label: 'Private mailboxes reachable', connected: true, trouble: false, relay: false };
  }
  const relayLine = (): PrivateLine => {
    const trouble = !!sync?.error;
    const connected = !!sync?.lastSync && !trouble;
    return {
      text: sync?.error ?? (sync?.lastSync ? `Encrypted updates checked with ${sync.replicas} of ${sync.total} relays.` : 'Saved on this device. Waiting for the encrypted relay.'),
      label: trouble ? 'Private sync needs attention' : connected ? 'Private relay checked' : 'Private sync pending',
      connected, trouble, relay: true,
    };
  };
  if (relays && sync?.lastSync) return relayLine();
  return group.isOwner
    ? { text: `Choosing mailboxes. Your ${device} picks public nodes in different places that keep sealed messages, as soon as it reaches two. Keep hoi open.`, label: 'Choosing private mailboxes', connected: false, trouble: false, relay: false }
    : { text: `Waiting for the owner’s device to choose the group’s mailboxes. Your updates stay on this ${device} until then.`, label: 'Waiting for private mailboxes', connected: false, trouble: false, relay: false };
}

/** How a share of places reads in a sentence. */
function share(level: number | undefined): string {
  return level === 20 ? 'a fifth' : level === 15 ? '15%' : 'a tenth';
}

/**
 * What an open community whose posts go in pieces may say (SCATTER.md S5): not that nobody
 * can read them (it is public), nor that nobody sees who posted (the node a device connects
 * through does), only what holds for the places its pieces go to.
 */
export function openPiecesLine(level: number): string {
  return `No node that stores this community’s posts knows who posted them, and no single node can stop a post. Stopping one would take more than ${share(level)} of the places its pieces go to.`;
}

/** The line under a group's status: who can read it and where its messages wait. */
export function privateHelp(group: PrivateGroup): string {
  const devices = plural(group.members.length, 'invited device');
  if (group.transport === 'mailboxes' && group.pieces === 'pieces') {
    return `${devices}. Posts go out in pieces through other public nodes: no single node holds or hands over a whole post, and the nodes that keep the pieces don’t know who posted. Stopping one would take more than ${share(group.piecesLevel)} of the places its pieces go to. Small sealed pointers wait in mailboxes on ${plural(group.mailboxes.length, 'public node')}, which can’t read them.`;
  }
  if (group.transport === 'mailboxes') return `${devices}. Messages wait in mailboxes on ${plural(group.mailboxes.length, 'public node')} the group agreed on. Those nodes keep sealed messages they can’t read, for up to 30 days, and only members can find them.`;
  return `${devices}. Messages will wait in mailboxes on public nodes the group agrees on. Those nodes keep sealed messages they can’t read.`;
}
