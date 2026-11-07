// The sync notice above a feed is one line: what the connection is, and Warmth. Its sentences
// open on request, and the × hides it until the reading changes: a new problem always shows
// again. What is hidden stays on the You page, under Connection. A per-device convenience,
// kept in this browser's storage (theme and motion live there too), never sent anywhere.
import type { SyncStatus } from './network-status';

const KEY = 'hoi:sync-hidden';

/** What the × hides: this exact reading. Another state or another problem shows again. */
export function noticeKey(status: Pick<SyncStatus, 'state' | 'title'>): string {
  return `${status.state}:${status.title}`;
}

function read(): string {
  try {
    return localStorage.getItem(KEY) ?? '';
  } catch {
    return '';
  }
}

class SyncNoticeState {
  /** The reading the person hid, or '' when nothing is hidden. */
  hidden = $state(read());
  /** Details are open for this visit only. */
  open = $state(false);

  isHidden(status: Pick<SyncStatus, 'state' | 'title'>): boolean {
    return !!this.hidden && this.hidden === noticeKey(status);
  }

  hide(status: Pick<SyncStatus, 'state' | 'title'>): void {
    this.hidden = noticeKey(status);
    this.open = false;
    try {
      localStorage.setItem(KEY, this.hidden);
    } catch {
      /* private mode: hidden for this visit only */
    }
  }

  show(): void {
    this.hidden = '';
    try {
      localStorage.removeItem(KEY);
    } catch {
      /* ignore */
    }
  }
}

export const syncNotice = new SyncNoticeState();
