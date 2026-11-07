// Service worker registration: production web builds only (never in dev, never inside Tauri).
// When a new worker is installed and waiting, the UI offers "A new version is ready · Reload".

class SwState {
  waiting: ServiceWorker | null = $state.raw(null);
  registration: ServiceWorkerRegistration | null = $state.raw(null);
  error = $state('');
}

export const sw = new SwState();

export function registerServiceWorker(): void {
  if (!import.meta.env.PROD || __HOI_DESKTOP__ || __HOI_SANDBOX__) return;
  if (typeof window === 'undefined' || '__TAURI_INTERNALS__' in window) return;
  if (!('serviceWorker' in navigator)) return;

  const start = async () => {
    try {
      const reg = await navigator.serviceWorker.register(new URL('sw.js', document.baseURI));
      sw.registration = reg;
      const track = (w: ServiceWorker | null) => {
        if (!w) return;
        w.addEventListener('statechange', () => {
          if (w.state === 'installed' && navigator.serviceWorker.controller) sw.waiting = w;
          if (w.state === 'redundant') sw.error = 'The browser update did not finish. Your previous cached app is still available; check again to retry.';
        });
      };
      if (reg.waiting && navigator.serviceWorker.controller) sw.waiting = reg.waiting;
      track(reg.installing);
      reg.addEventListener('updatefound', () => track(reg.installing));
      // Look for a new version now and then while the app stays open.
      setInterval(() => void checkWebUpdate(), 60 * 60 * 1000);
    } catch (e) {
      sw.error = 'Offline caching is unavailable in this browser session. Check storage permissions and retry.';
    }
  };
  let reloading = false;
  navigator.serviceWorker.addEventListener('controllerchange', () => {
    if (reloading || !sw.waiting) return;
    reloading = true;
    location.reload();
  });
  if (document.readyState === 'complete') void start();
  else window.addEventListener('load', () => void start(), { once: true });
}

export function applyUpdate(): void {
  sw.waiting?.postMessage({ type: 'skip-waiting' });
}

export async function checkWebUpdate(): Promise<void> {
  sw.error = '';
  if (!sw.registration) return;
  try { await sw.registration.update(); }
  catch { sw.error = 'The browser update check failed. Your cached app is unchanged; reconnect and retry.'; }
}
