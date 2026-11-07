// Best-effort guess at the visitor's OS, used only to highlight the matching download.
// Nothing leaves the page; a wrong guess just highlights the wrong row.
export type OS = 'macos' | 'windows' | 'linux' | 'android' | 'ios';

export const OS_LABEL: Record<OS, string> = {
  macos: 'macOS',
  windows: 'Windows',
  linux: 'Linux',
  android: 'Android',
  ios: 'iOS',
};

export function detectOS(): OS | null {
  const nav = navigator as Navigator & { userAgentData?: { platform?: string } };
  const hint = (nav.userAgentData?.platform ?? '').toLowerCase();
  const ua = navigator.userAgent.toLowerCase();
  // iPadOS asks for desktop sites and says "Macintosh"; touch gives it away.
  if (/iphone|ipad|ipod/.test(ua) || (/macintosh/.test(ua) && navigator.maxTouchPoints > 1)) return 'ios';
  if (hint === 'android' || /android/.test(ua)) return 'android';
  if (hint === 'chrome os' || /\bcros\b/.test(ua)) return null;
  if (hint === 'macos' || /mac os x|macintosh/.test(ua)) return 'macos';
  if (hint === 'windows' || /windows/.test(ua)) return 'windows';
  if (hint === 'linux' || /linux|x11/.test(ua)) return 'linux';
  return null;
}
