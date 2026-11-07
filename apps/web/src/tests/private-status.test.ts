import { describe, expect, it } from 'vitest';
import type { PrivateGroup } from '@hoi/core-api';
import { openPiecesLine, privateHelp, privateLine } from '../lib/private-status';

const base: PrivateGroup = { community: 'c'.repeat(64), room: 'a'.repeat(64), epoch: 3, active: true, isOwner: true, members: [{ device: 'd'.repeat(64), root: 'r'.repeat(64) }], pending: 0, received: [], transport: 'mailboxes', mailboxes: [], pieces: 'whole', followsRecords: false, invitations: [], committers: 1 };
const boxes = (linked: boolean[]) => linked.map((l, i) => ({ endpoint: String(i).repeat(64), linked: l }));

describe('how a private group syncs, in the words the app shows', () => {
  it('describes mailboxes without a relay or a sync button', () => {
    const line = privateLine({ ...base, mailboxes: boxes([true, true, false, true]), pending: 1 }, undefined, 0, 'browser');
    expect(line).toMatchObject({ connected: true, trouble: false, relay: false, label: 'Private mailboxes reachable' });
    expect(line.text).toBe('Encrypted mailboxes: 3 of 4 reachable. 1 update still going out.');
    expect(privateHelp({ ...base, mailboxes: boxes([true, true, false, true]) })).toMatch(/^1 invited device\. Messages wait in mailboxes on 4 public nodes .* can’t read/);
  });

  it('says plainly when no mailbox can be reached, and that nothing is lost', () => {
    const line = privateLine({ ...base, mailboxes: boxes([false, false]) }, undefined, 0, 'phone');
    expect(line).toMatchObject({ connected: false, trouble: true, label: 'Private mailboxes unreachable' });
    expect(line.text).toContain('Updates wait on this phone');
  });

  it('says what pieces make true only when posts go in pieces, with the level they meet', () => {
    const inPieces = { ...base, pieces: 'pieces' as const, piecesLevel: 10, mailboxes: boxes([true, true]) };
    expect(privateHelp(inPieces)).toMatch(/no single node holds or hands over a whole post, and the nodes that keep the pieces don’t know who posted\. Stopping one would take more than a tenth of the places/);
    expect(privateHelp({ ...inPieces, piecesLevel: 20 })).toContain('more than a fifth');
    expect(privateHelp({ ...base, mailboxes: boxes([true, true]) })).not.toContain('pieces');
  });

  it('tells the owner their device chooses the mailboxes, and members to wait', () => {
    expect(privateLine({ ...base, transport: 'waiting' }, undefined, 0, 'computer').text).toMatch(/^Choosing mailboxes\. Your computer picks/);
    expect(privateLine({ ...base, transport: 'waiting', isOwner: false }, undefined, 0, 'computer').label).toBe('Waiting for private mailboxes');
  });

  it('says what the relay checked while mailboxes are still being chosen', () => {
    expect(privateLine({ ...base, transport: 'waiting' }, { busy: false, replicas: 2, total: 3, lastSync: 1 }, 3, 'browser')).toMatchObject({ relay: true, connected: true, text: 'Encrypted updates checked with 2 of 3 relays.' });
    expect(privateLine({ ...base, active: false }, undefined, 0, 'browser').label).toBe('Former private member');
  });

  it('says for an open community in pieces only what holds there', () => {
    expect(openPiecesLine(10)).toBe('No node that stores this community’s posts knows who posted them, and no single node can stop a post. Stopping one would take more than a tenth of the places its pieces go to.');
    expect(openPiecesLine(20)).not.toMatch(/can’t read|nobody sees/);
  });
});
