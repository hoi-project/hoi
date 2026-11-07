import { describe, expect, it } from 'vitest';
import { RECOVERY_KEY_PREFIX, formatRecoveryKey, looksLikeRecoveryKey, recoveryKeyBody, zbase32Decode, zbase32Encode } from '@hoi/core-api';
import { createMockCore } from '../lib/core/mock-core';

describe('recovery keys', () => {
  it('encodes and decodes z-base-32 like the core', () => {
    // the alphabet avoids 0, l, v and 2; every byte string round-trips
    const bytes = Uint8Array.from({ length: 35 }, (_, i) => (i * 37) & 255);
    const text = zbase32Encode(bytes);
    expect(text).toHaveLength(56);
    expect(text).toMatch(/^[ybndrfg8ejkmcpqxot1uwisza345h769]+$/);
    expect(zbase32Decode(text)).toEqual(bytes);
    expect(zbase32Decode('abc0')).toBeNull();
    expect(zbase32Encode(new Uint8Array([0xff]))).toBe('9h');
  });

  it('tells a recovery key from a passphrase, a link or a half-pasted line', () => {
    const body = 'y'.repeat(56);
    const line = formatRecoveryKey(body);
    expect(line).toBe(`${RECOVERY_KEY_PREFIX}yyyyyyyy-yyyyyyyy-yyyyyyyy-yyyyyyyy-yyyyyyyy-yyyyyyyy-yyyyyyyy`);
    expect(looksLikeRecoveryKey(line)).toBe(true);
    expect(looksLikeRecoveryKey(` ${line.toUpperCase()}\n`)).toBe(true);
    expect(looksLikeRecoveryKey(line.replace(/-/g, ' '))).toBe(true);
    expect(recoveryKeyBody(`hoikey1${body}`)).toBe(body);
    expect(recoveryKeyBody('hoi-key-1-' + 'o'.repeat(28) + '0'.repeat(28))).toBe('o'.repeat(56));
    expect(looksLikeRecoveryKey(line.slice(0, -1))).toBe(false);
    expect(looksLikeRecoveryKey(`${line}y`)).toBe(false);
    expect(looksLikeRecoveryKey('correct horse battery staple')).toBe(false);
    expect(looksLikeRecoveryKey('hoi://join?c=abc')).toBe(false);
    expect(looksLikeRecoveryKey('hoi-key-2-' + body)).toBe(false);
  });

  it('brings the same key back on a fresh mock device, and refuses typos and second keys', async () => {
    const a = createMockCore();
    await a.boot();
    const me = await a.createIdentity('mira');
    const line = await a.recoveryKey();
    expect(line.startsWith(RECOVERY_KEY_PREFIX)).toBe(true);
    expect(looksLikeRecoveryKey(line)).toBe(true);
    expect(await a.recoveryKey()).toBe(line);
    await expect(a.restoreIdentity(line, 'mira')).rejects.toMatchObject({ code: 'not-allowed' });

    const b = createMockCore();
    await b.boot();
    await expect(b.restoreIdentity('correct horse battery staple', 'mira')).rejects.toMatchObject({ code: 'invalid' });
    const wrong = line.slice(0, -1) + (line.endsWith('y') ? 'b' : 'y');
    await expect(b.restoreIdentity(wrong, 'mira')).rejects.toMatchObject({ code: 'bad-passphrase' });
    await expect(b.restoreIdentity(line, '')).rejects.toMatchObject({ code: 'invalid' });
    expect(await b.me()).toBeNull();
    const tries: number[] = [];
    const restored = await b.restoreIdentity(line.toUpperCase().replace(/-/g, ' '), 'mira again', { onTry: (t) => tries.push(t.bits) });
    expect(restored.rootKey).toBe(me.rootKey);
    expect(restored.name).toBe('mira again');
    expect(restored.hasRoot).toBe(true);
    expect(restored.device).not.toBe(me.device);
    expect(tries.length).toBeGreaterThan(0);
    expect(await b.recoveryKey()).toBe(line);
  });

  it('holds the encrypted file to the vault passphrase standard', async () => {
    const a = createMockCore();
    await a.boot();
    await a.createIdentity('mira');
    await expect(a.exportIdentity('too short')).rejects.toMatchObject({ code: 'invalid' });
    expect((await a.exportIdentity('fifteen chars ok')).length).toBeGreaterThan(0);
  });
});
