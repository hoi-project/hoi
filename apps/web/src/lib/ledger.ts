// How ledger entries read in the UI. Pure, so it is unit-tested.
import type { LedgerEntryView, Limits, ProposalStatus } from '@hoi/core-api';
import { duration } from '@hoi/core-api';
import { admissionLabel } from './util';

export type TagColor = 'mint' | 'sky' | 'lilac' | 'peach' | 'sun' | 'coral' | 'soft';

export function ledgerTag(e: LedgerEntryView, rulesVersion?: number): { text: string; color: TagColor } {
  switch (e.action) {
    case 'remove': return { text: 'removed', color: 'peach' };
    case 'ban': return { text: e.auto ? 'bounced' : 'banned', color: 'peach' };
    case 'unban': return { text: 'unbanned', color: 'mint' };
    case 'label': return { text: 'labelled', color: 'sky' };
    case 'rules': return { text: rulesVersion ? `rules v${rulesVersion}` : 'rules', color: 'mint' };
    case 'limits': return { text: 'limits', color: 'sun' };
    case 'mod-add': return { text: 'new mod', color: 'lilac' };
    case 'mod-remove': return { text: 'mod left', color: 'soft' };
    case 'pin': return { text: 'pinned', color: 'sun' };
    case 'unpin': return { text: 'unpinned', color: 'soft' };
    case 'admit': return { text: 'admitted', color: 'mint' };
    case 'drop-key': return { text: 'drop key', color: 'soft' };
    case 'retire': return { text: 'left for good', color: 'soft' };
  }
}

/** Who signed it, in words. Automation is "spam shield". */
export function ledgerWho(e: LedgerEntryView): string {
  if (e.by.length === 0) return e.auto ? 'spam shield' : 'unknown key';
  return e.by.map((p) => p.name).join(', ');
}

/** The Signed column: '88c1', 'auto', '3 sigs'. The ✓ is added by the table. */
export function ledgerSigned(e: LedgerEntryView): string {
  if (e.auto) return 'auto';
  if (e.by.length > 1) return `${e.by.length} sigs`;
  return e.by[0]?.short.slice(0, 4) ?? '—';
}

/** Versions for rules entries: the newest one is the current rules version, older ones count down. */
export function rulesVersions(entries: LedgerEntryView[], current?: number): Map<string, number> {
  const rules = entries.filter((e) => e.action === 'rules').sort((a, b) => b.seq - a.seq);
  let v = current ?? rules.length;
  const out = new Map<string, number>();
  for (const e of rules) out.set(e.id, Math.max(1, v--));
  return out;
}

/** Where a rules or limits proposal stands, for the ledger row. */
export function proposalNote(s: ProposalStatus): string {
  switch (s) {
    case 'open': return 'proposed · the community is deciding';
    case 'in-effect': return 'in effect · answers still open';
    case 'passed': return 'agreed by the community';
    case 'lapsed': return "didn't pass";
  }
}

export function limitsLine(l: Limits): string {
  return `${l.powBits}-bit stamp · posts fade after ${duration(l.maxRetention)} · newcomers ${l.newcomerPostsPerDay}/day · joining: ${admissionLabel(l.admission).toLowerCase()} · auto-bounce at ${l.autobanThreshold} flags`;
}

export function truncate(s: string, n: number): string {
  return s.length > n ? s.slice(0, n - 1).trimEnd() + '…' : s;
}
