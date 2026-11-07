// The reactive side of the Network & Safety acknowledgement: the app starts nothing that
// connects until `accepted`, and shows the notice again when its version changed.
import { CONSENT_KEY, type ConsentRecord, consentCurrent, consentRecord, parseConsent } from './consent';

function read(): ConsentRecord | null {
  try {
    return parseConsent(localStorage.getItem(CONSENT_KEY));
  } catch {
    return null;
  }
}

class NetworkConsent {
  record: ConsentRecord | null = $state.raw(read());
  /** The current notice was read and acknowledged on this device. */
  readonly accepted = $derived(consentCurrent(this.record));
  /** An older version was acknowledged before: the words changed since. */
  readonly changed = $derived(!!this.record && !consentCurrent(this.record));

  accept(): void {
    const record = consentRecord();
    try {
      localStorage.setItem(CONSENT_KEY, JSON.stringify(record));
    } catch {
      /* private mode: the acknowledgement lasts for this visit */
    }
    this.record = record;
  }

  /** Ask again at the next start, and now: after a wipe, or to read it again. */
  reset(): void {
    try {
      localStorage.removeItem(CONSENT_KEY);
    } catch {
      /* nothing stored, nothing to remove */
    }
    this.record = null;
  }
}

export const networkConsent = new NetworkConsent();
