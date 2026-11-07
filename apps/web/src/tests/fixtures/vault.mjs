// Synchronous authenticated cipher for exercising IDB transactions. The production
// Argon2id/XChaCha implementation is tested in Rust and in browser integration tests.
import { createCipheriv, createDecipheriv, createHash, randomBytes } from 'node:crypto';

export function testVault() {
  const key = randomBytes(32);
  const header = randomBytes(80);
  return {
    header: () => new Uint8Array(header),
    fingerprint: value => createHash('sha256').update(value).digest('hex'),
    seal(context, value) {
      const nonce = randomBytes(12);
      const cipher = createCipheriv('aes-256-gcm', key, nonce);
      cipher.setAAD(Buffer.from(context));
      return new Uint8Array(Buffer.concat([nonce, cipher.update(value), cipher.final(), cipher.getAuthTag()]));
    },
    unseal(context, value) {
      const cipher = createDecipheriv('aes-256-gcm', key, value.slice(0, 12));
      cipher.setAAD(Buffer.from(context));
      cipher.setAuthTag(value.slice(-16));
      return new Uint8Array(Buffer.concat([cipher.update(value.slice(12, -16)), cipher.final()]));
    },
  };
}
