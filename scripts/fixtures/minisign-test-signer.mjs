// A throwaway minisign signer for tests: a fresh Ed25519 key per call, never written to disk.
// Its signatures verify with scripts/release-verify.mjs exactly as a real release key's would.
import { generateKeyPairSync, sign } from 'node:crypto';
import { blake2b } from '@noble/hashes/blake2.js';

export function signer() {
  const { publicKey, privateKey } = generateKeyPairSync('ed25519');
  const id = Buffer.from('01234567');
  const key = Buffer.concat([Buffer.from('Ed'), id, publicKey.export({ format: 'der', type: 'spki' }).subarray(-32)]).toString('base64');
  return { key, sign(bytes) {
    const sig = sign(null, blake2b(bytes), privateKey);
    const comment = Buffer.from('timestamp:1793960000\tversion:2.0.0');
    return `untrusted comment: isolated test key\n${Buffer.concat([Buffer.from('ED'), id, sig]).toString('base64')}\ntrusted comment: ${comment}\n${sign(null, Buffer.concat([sig, comment]), privateKey).toString('base64')}\n`;
  } };
}
