// A voluntary check contacts only the route the person selected. It never enters the sync
// worker, so joining, pairing credentials, profile and community state stay out of the probe.
import { isDesktopShell } from '../host';
import { numericAddress } from './rtc-addresses';
import { openRtc, parseRoute } from './rtc-link';

type Transport = 'websocket' | 'webrtc' | 'udp';
type Observation = { address: string; transport: Transport };
const request = new TextEncoder().encode('hoi/address-check/1');
const prefix = 'hoi/address/1 ';
const unavailable = () => new Error('The node did not report an address for this connection.');

function socketRoute(route: string): boolean {
  try {
    const url = new URL(route);
    return !url.username && !url.password && !url.hash && !url.search
      && (url.protocol === 'wss:' || (url.protocol === 'ws:' && ['127.0.0.1', '[::1]', 'localhost'].includes(url.hostname)));
  } catch { return false; }
}

export function canCheckRoute(route: string): boolean {
  return !!parseRoute(route) || socketRoute(route);
}

export function readObservation(bytes: Uint8Array): string | null {
  if (bytes.length > prefix.length + 45) return null;
  const reply = new TextDecoder().decode(bytes);
  if (!reply.startsWith(prefix)) return null;
  const ip = reply.slice(prefix.length);
  return numericAddress(ip) ? ip : null;
}

export async function checkConnection(route: string, signal: AbortSignal): Promise<Observation> {
  signal.throwIfAborted();
  if (!canCheckRoute(route)) throw unavailable();
  if (isDesktopShell()) {
    const { invoke } = await import('@tauri-apps/api/core');
    signal.throwIfAborted();
    const result = await invoke<Observation>('hoi_check_connection', { route });
    signal.throwIfAborted();
    return result;
  }

  const transport = parseRoute(route) ? 'webrtc' : 'websocket';
  return new Promise<Observation>((resolve, reject) => {
    const controller = new AbortController();
    let close = () => {};
    let settled = false;
    const finish = (address: string | null) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      signal.removeEventListener('abort', abort);
      controller.abort();
      close();
      if (address) resolve({ address, transport });
      else reject(signal.aborted ? signal.reason : unavailable());
    };
    const abort = () => finish(null);
    const timer = setTimeout(abort, 14000);
    signal.addEventListener('abort', abort, { once: true });
    if (transport === 'webrtc') {
      // Client mode is silent until the first application frame. The remote DTLS
      // certificate is pinned to the route, and this probe never sends a sync hello.
      void openRtc([route], 'c', {
        message: bytes => finish(readObservation(bytes)), close: abort,
      }, controller.signal).then(link => {
        if (settled) { link.close(); return; }
        close = () => link.close();
        link.send(request);
      }).catch(abort);
    } else {
      try {
        const socket = new WebSocket(route);
        close = () => socket.close();
        socket.binaryType = 'arraybuffer';
        socket.onopen = () => { if (!settled) socket.send(request); };
        socket.onmessage = e => finish(e.data instanceof ArrayBuffer ? readObservation(new Uint8Array(e.data)) : null);
        socket.onerror = socket.onclose = abort;
      } catch { abort(); }
    }
  });
}
