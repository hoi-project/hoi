// Real Ed25519 fixtures for browser verifier tests; no runtime dependency.
import { generateKeyPairSync, sign } from 'node:crypto';
import { RELEASE } from '../../lib/preview';

export const pair = generateKeyPairSync('ed25519');
export const otherKey = () => generateKeyPairSync('ed25519');
export function claim(name = 'bikes', key = pair, version = 1, protocolVersion = RELEASE.protocolVersion, indexVersion = RELEASE.indexVersion) {
  const community = key.publicKey.export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex');
  const statement = version === 2 ? `hoi community index v2\n${name}\n${community}\n${protocolVersion}\n${indexVersion}` : `hoi community name v1\n${name}\n${community}`;
  return { name, community, ...(version === 2 ? { protocolVersion, indexVersion } : {}), signature: sign(null, Buffer.from(statement), key.privateKey).toString('hex') };
}
