// Handing a secret to the person's password manager.
//
// Two ways, used together: a plain form with a `username` and a `password` field, submitted
// (managers can recognize it), and the Credential Management API,
// where a browser has it (Chrome, Edge), which opens the browser's own "save password?" bubble
// without any navigation. Manager support varies; an API result does not prove a saved backup.
import { recoveryKeyBody } from '@hoi/core-api';

export const VAULT_LOGIN_KEY = 'hoi:vault-login';

/** Labels are non-secret. Each new installation gets its own manager entry. */
export function vaultLoginLabel(desktop: boolean, creating: boolean): string {
  try {
    const saved = localStorage.getItem(VAULT_LOGIN_KEY);
    if (saved) return saved;
  } catch { /* A manager can still fill the visible username when preferences are blocked. */ }
  const legacy = desktop ? 'hoi — desktop vault' : 'hoi — browser vault';
  return creating ? `${legacy} · ${crypto.randomUUID().slice(0, 8)}` : legacy;
}

export function rememberVaultLogin(label: string): void {
  try { localStorage.setItem(VAULT_LOGIN_KEY, label.slice(0, 200)); } catch { /* Optional label only. */ }
}

export function recoveryLogin(name: string, short: string): string {
  return `hoi key · ${name} · ${short}`;
}

export function backupLogin(filename: string): string {
  return `hoi key file · ${filename}`;
}

/** Read the submitted DOM, including autofill that did not dispatch an input event. */
export function credentialValue(form: HTMLFormElement, name = 'password'): string {
  const value = new FormData(form).get(name);
  return typeof value === 'string' ? value : '';
}

/** Verify retrieval of the saved recovery line without restoring or rotating an identity. */
export function sameRecoveryKey(saved: string, expected: string): boolean {
  const body = recoveryKeyBody(saved);
  return !!body && body === recoveryKeyBody(expected);
}

/** Whether this browser can be asked to save a password outright. */
export function canStoreCredential(): boolean {
  return typeof window !== 'undefined' && 'PasswordCredential' in window && !!navigator.credentials?.store;
}

/**
 * Asks the browser to save `password` under `id` for this origin. Resolves `'stored'` when the
 * browser took it (it may still have asked the person first), `'unsupported'` where only the
 * form-submit route exists, `'declined'` when the browser refused.
 */
export async function storeCredential(id: string, password: string, name: string): Promise<'stored' | 'unsupported' | 'declined'> {
  if (!canStoreCredential()) return 'unsupported';
  try {
    const Ctor = (window as unknown as { PasswordCredential: new (init: { id: string; password: string; name?: string }) => Credential }).PasswordCredential;
    await navigator.credentials.store(new Ctor({ id, password, name }));
    // Disable silent credential mediation where supported. This is not protection against
    // a compromised origin, nor evidence that a particular password manager saved the key.
    await navigator.credentials.preventSilentAccess?.().catch(() => undefined);
    return 'stored';
  } catch {
    return 'declined';
  }
}
