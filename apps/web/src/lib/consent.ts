// The Network & Safety acknowledgement: kept in localStorage with the version read
// and the date, so it is readable before the vault is unlocked, and asked again when the words
// change. Pure helpers here; the reactive store is consent.svelte.ts.
import { CONSENT_KEY, NETWORK_NOTICE_VERSION } from './legal/version.mjs';

export interface ConsentRecord {
  version: string;
  date: string;
}

export { CONSENT_KEY, NETWORK_NOTICE_VERSION };

/** What is stored, or null when nothing is or it is unreadable. */
export function parseConsent(raw: string | null | undefined): ConsentRecord | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as Partial<ConsentRecord>;
    if (typeof v?.version !== 'string' || typeof v?.date !== 'string') return null;
    return { version: v.version, date: v.date };
  } catch {
    return null;
  }
}

/** Whether a stored acknowledgement is the current notice's. */
export function consentCurrent(record: ConsentRecord | null, version = NETWORK_NOTICE_VERSION): boolean {
  return !!record && record.version === version;
}

export function consentRecord(now = new Date(), version = NETWORK_NOTICE_VERSION): ConsentRecord {
  return { version, date: now.toISOString().slice(0, 10) };
}
