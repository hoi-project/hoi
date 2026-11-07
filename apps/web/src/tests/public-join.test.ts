import { describe, expect, it } from 'vitest';
import { publicTarget } from '../lib/public-join';
import { parseHash, to, screenKey } from '../lib/router';
import { RELEASE } from '../lib/preview';

describe('public key joining links', () => {
  const id = 'a'.repeat(64);
  it('round trips registry links without an invite, account or pairing credential', () => {
    const link = to.public(id, 'bikes', RELEASE.protocolVersion);
    const route = parseHash(link);
    expect(route.name).toBe('public');
    if (route.name !== 'public') throw new Error('Wrong route');
    expect(publicTarget(route.query)).toEqual({ id, name: 'bikes', protocol: RELEASE.protocolVersion, compatible: true });
    expect(screenKey(route)).toBe(`public:${route.query}`);
  });
  it('requires full keys and valid optional name/version hints', () => {
    for (const query of ['c=abc', `c=${id}&name=<script>`, `c=${id}&p=bad/version`]) expect(() => publicTarget(query)).toThrow('invalid');
    expect(publicTarget(`c=${id}`)).toMatchObject({ id, compatible: true });
  });
  it('does not pretend that a different protocol can be decoded', () => {
    expect(publicTarget(`c=${id}&p=future-unsupported`).compatible).toBe(false);
  });
});
