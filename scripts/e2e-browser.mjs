// Which browser an end-to-end script drives: HOI_E2E_BROWSER=chromium (the default), firefox or
// webkit (Safari's engine, close to Safari but not Safari itself). The WebRTC runs go through
// all three in the major-release suites (docs/CI.md).
import { chromium, firefox, webkit } from 'playwright';

export const BROWSER = process.env.HOI_E2E_BROWSER || 'chromium';

// Firefox leaves loopback and private host candidates out of ICE by default; the tests dial
// nodes on 127.0.0.1. Data channels only: nothing asks for a camera or a microphone.
const FIREFOX_PREFS = {
  'media.peerconnection.ice.loopback': true,
  'media.peerconnection.ice.obfuscate_host_addresses': false,
  'media.peerconnection.ice.no_host': false,
  'media.navigator.permission.disabled': true,
};

// Waits for proof-of-work. Identity keys and post stamps are real Argon2id work with a random
// number of tries, so the time varies from run to run; on a 2-core CI runner, in Firefox, an
// unlucky key took longer than 90 s and a stamp longer than 30 s. These are ceilings for a stuck
// page, not expected times: a passing run finishes as fast as before.
export const KEY_WORK_TIMEOUT = 300_000;
export const STAMP_TIMEOUT = 120_000;

/** Launches the chosen browser with Playwright's launch options. */
export function launch(options = {}) {
  if (BROWSER === 'firefox') return firefox.launch({ ...options, firefoxUserPrefs: FIREFOX_PREFS });
  if (BROWSER === 'webkit') return webkit.launch(options);
  if (BROWSER !== 'chromium') throw new Error(`HOI_E2E_BROWSER is chromium, firefox or webkit, not ${BROWSER}`);
  return chromium.launch({ ...options, executablePath: process.env.CHROMIUM_PATH || undefined });
}

/** A Playwright device descriptor this browser can take: Firefox has no mobile emulation. */
export function device(descriptor) {
  if (BROWSER !== 'firefox') return descriptor;
  const { isMobile: _mobile, hasTouch: _touch, defaultBrowserType: _type, ...rest } = descriptor;
  return rest;
}
