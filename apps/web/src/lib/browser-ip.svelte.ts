// This is a browser HTTPS observation. Peer, relay and native VPN observations live separately.
import { fetchBrowserEgress } from './core/browser-egress';
import { locate } from './ip-location.svelte';

type Sample = { address: string; checkedAt: Date };
export const browserIp = $state<{ sample: Sample | null; pending: boolean; error: string }>({
  sample: null, pending: false, error: '',
});

let request: AbortController | undefined;
let expires: ReturnType<typeof setTimeout> | undefined;

export function clearBrowserIp() {
  const previous = request;
  request = undefined;
  previous?.abort();
  clearTimeout(expires);
  browserIp.sample = null;
  browserIp.error = '';
  browserIp.pending = false;
}

export async function checkBrowserIp() {
  if (browserIp.pending) return;
  clearBrowserIp();
  const current = new AbortController();
  request = current;
  browserIp.pending = true;
  try {
    const address = await fetchBrowserEgress(current.signal);
    if (request !== current) return;
    browserIp.sample = { address, checkedAt: new Date() };
    // Browsers do not reliably announce VPN route changes. Limit the life of each sample.
    expires = setTimeout(clearBrowserIp, 120_000);
    // The person asked for this check, so look up its country with the tables built into the app.
    void locate(address);
  } catch (error) {
    if (request === current) browserIp.error = error instanceof Error ? error.message : 'The public IP check failed. Try again.';
  } finally {
    if (request === current) {
      request = undefined;
      browserIp.pending = false;
    }
  }
}

/** Listening for network changes never starts a lookup. A new sample always takes another click. */
export function watchBrowserIpRoute(): () => void {
  const network = (navigator as Navigator & { connection?: EventTarget }).connection;
  const onVisibility = () => { if (document.hidden) clearBrowserIp(); };
  window.addEventListener('online', clearBrowserIp);
  window.addEventListener('offline', clearBrowserIp);
  window.addEventListener('pagehide', clearBrowserIp);
  document.addEventListener('visibilitychange', onVisibility);
  network?.addEventListener('change', clearBrowserIp);
  return () => {
    window.removeEventListener('online', clearBrowserIp);
    window.removeEventListener('offline', clearBrowserIp);
    window.removeEventListener('pagehide', clearBrowserIp);
    document.removeEventListener('visibilitychange', onVisibility);
    network?.removeEventListener('change', clearBrowserIp);
    clearBrowserIp();
  };
}
