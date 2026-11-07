import { describe, expect, it } from 'vitest';
import { CONSENT_KEY, NETWORK_NOTICE_VERSION, consentCurrent, consentRecord, parseConsent } from '../lib/consent';
import { parseHash, href, to } from '../lib/router';
import { NETWORK_SAFETY, OPERATOR_RESPONSIBILITY, PRIVACY_NOTICE } from '../lib/legal/texts';

describe('the Network & Safety acknowledgement', () => {
  it('is kept with the version read and the date, and asked again when the words change', () => {
    expect(CONSENT_KEY).toBe('hoi:consent:network');
    expect(parseConsent(null)).toBeNull();
    expect(parseConsent('not json')).toBeNull();
    expect(parseConsent('{"version":1}')).toBeNull();
    const record = consentRecord(new Date('2026-11-03T10:00:00Z'));
    expect(record).toEqual({ version: NETWORK_NOTICE_VERSION, date: '2026-11-03' });
    expect(consentCurrent(parseConsent(JSON.stringify(record)))).toBe(true);
    expect(consentCurrent({ version: '0.9', date: '2026-01-01' })).toBe(false);
    expect(consentCurrent(null)).toBe(false);
  });

  it('has a privacy route that works before anything is unlocked', () => {
    expect(parseHash('#/privacy')).toEqual({ name: 'privacy' });
    expect(parseHash('#/privacy/x').name).toBe('notfound');
    expect(href({ name: 'privacy' })).toBe('#/privacy');
    expect(to.privacy()).toBe('#/privacy');
  });

  it('ships the notices with their two required boxes', () => {
    expect(NETWORK_SAFETY.version).toBe(NETWORK_NOTICE_VERSION);
    expect(NETWORK_SAFETY.sections.map((s) => s.title)).toEqual([
      'You are joining a peer-to-peer network.', 'Your connections do not stay with one node.', 'Your posts travel in pieces.', 'Your IP address will be visible.', 'Not everyone may act in good faith.', 'Use a VPN at all times.',
    ]);
    expect(NETWORK_SAFETY.checks.ip).toMatch(/IP address and connection data/);
    expect(NETWORK_SAFETY.checks.vpn).toMatch(/VPN/);
    expect(PRIVACY_NOTICE.sections.map((s) => s.title)).toContain('Data stored on your device.');
    expect(PRIVACY_NOTICE.sections.map((s) => s.title)).toContain('What the nodes in between see.');
    // the words say what holds, never more: open communities stay public, the first node sees the address
    const pieces = NETWORK_SAFETY.sections.find((s) => s.title === 'Your posts travel in pieces.')!.text;
    expect(pieces).toMatch(/does see your address/);
    expect(pieces).toMatch(/anyone who joins can read/);
    expect(OPERATOR_RESPONSIBILITY.check).toBe('I understand that I am responsible for the data processed on my node.');
    expect(PRIVACY_NOTICE.sections.map((s) => s.title)).toContain('Where your device connects.');
  });
});
