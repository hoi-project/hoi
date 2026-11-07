import { afterEach, describe, expect, it, vi } from 'vitest';
import { formatRecoveryKey } from '@hoi/core-api';
import { backupLogin, recoveryLogin, rememberVaultLogin, sameRecoveryKey, storeCredential, vaultLoginLabel } from '../lib/password-manager';

afterEach(() => vi.unstubAllGlobals());

describe('password-manager credential purposes', () => {
  it('keeps new installations and files distinct, preserving legacy unlock names', () => {
    const values = new Map<string, string>();
    vi.stubGlobal('localStorage', { getItem: (key: string) => values.get(key), setItem: (key: string, value: string) => values.set(key, value) });
    expect(vaultLoginLabel(false, false)).toBe('hoi — browser vault');
    expect(vaultLoginLabel(true, false)).toBe('hoi — desktop vault');
    const created = vaultLoginLabel(false, true);
    expect(created).not.toBe(vaultLoginLabel(false, true));
    rememberVaultLogin(created);
    expect(vaultLoginLabel(false, false)).toBe(created);
    expect(backupLogin('lena-a.hoikey')).not.toBe(backupLogin('lena-b.hoikey'));
    expect(recoveryLogin('lena', 'aaaa')).not.toBe(recoveryLogin('lena', 'bbbb'));
    expect(recoveryLogin('lena', 'aaaa')).not.toBe(created);
  });

  it('verifies the retrieved recovery line without accepting other passwords or identities', () => {
    const expected = formatRecoveryKey('y'.repeat(56));
    expect(sameRecoveryKey(expected.toUpperCase().replaceAll('-', ' '), expected)).toBe(true);
    expect(sameRecoveryKey(formatRecoveryKey('b'.repeat(56)), expected)).toBe(false);
    expect(sameRecoveryKey('vault password', expected)).toBe(false);
    expect(sameRecoveryKey('', '')).toBe(false);
  });

  it('reports manager refusal and unsupported browsers without claiming a save', async () => {
    vi.stubGlobal('window', {});
    expect(await storeCredential('hoi key', 'secret', 'hoi')).toBe('unsupported');
    vi.stubGlobal('window', { PasswordCredential: class { constructor(public value: unknown) {} } });
    vi.stubGlobal('navigator', { credentials: { store: vi.fn().mockRejectedValue(new Error('declined')) } });
    expect(await storeCredential('hoi key', 'secret', 'hoi')).toBe('declined');
  });
});
