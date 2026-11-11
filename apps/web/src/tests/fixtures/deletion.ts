// Deletion status as the core reports it (core/crates/hoi-core/src/dto.rs): typed against
// @hoi/core-api, so a DTO change breaks these fixtures at `pnpm check`.
import type { BulkDeletionStatus, BulkDeletionTarget, CleanupReadiness, DeletionProgress } from '@hoi/core-api';

export const T = 1_790_000_000;
export const DAY = 86_400;
const hex = (n: number) => n.toString(16).padStart(64, '0');
export const community = hex(0xc0);

/** One signed deletion's RFC-0001 progress: no holder targets, not sent, still in effect. */
export function progress(n: number, patch: Partial<DeletionProgress> = {}): DeletionProgress {
  return {
    id: hex(0xd000 + n), target: hex(n), localRemoved: true, recordSubmitted: false,
    queued: 0, submitted: 0, acknowledged: 0, legacyRemoval: 0, unknown: 0, refused: 0, unreachable: 0, expired: 0,
    outstanding: 0, coverageGaps: 0, expiresAt: T + 7 * DAY, requiresLocalState: true, inventoryComplete: false,
    ...patch,
  };
}

export function target(n: number, patch: Partial<BulkDeletionTarget> = {}): BulkDeletionTarget {
  return { id: hex(n), community, kind: 'post', state: 'selected', reason: null, deletion: null, nextTry: T, ...patch };
}

export function batch(targets: BulkDeletionTarget[], patch: Partial<BulkDeletionStatus> = {}): BulkDeletionStatus {
  const count = (state: BulkDeletionTarget['state']) => targets.filter((t) => t.state === state).length;
  return {
    snapshot: hex(0xb0), confirmedAt: T, cancelledAt: null, count: targets.length,
    selected: count('selected'), queued: count('queued'), signed: count('signed'), failed: count('failed'), expired: count('expired'), cancelled: count('cancelled'),
    targets, gaps: [], inventoryComplete: false, ...patch,
  };
}

/** A signed batch target linked to its progress entry. */
export const signed = (n: number) => target(n, { state: 'signed', deletion: hex(0xd000 + n), nextTry: null });

export function readiness(patch: Partial<CleanupReadiness> = {}): CleanupReadiness {
  return { ready: true, blocked: [], until: null, unlisted: 0, inventory: hex(0x1), deletions: hex(0x2), ...patch };
}
