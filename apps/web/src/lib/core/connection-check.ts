// A voluntary check contacts only the route the person selected. It never enters the sync
// worker, so joining, pairing credentials, profile and community state stay out of the probe.
import { isDesktopShell } from '../host';
import { numericAddress } from './rtc-addresses';
import { openRtc, parseRoute } from './rtc-link';

type Transport = 'websocket' | 'webrtc' | 'udp' | 'quic';
type Observation = { address: string; transport: Transport };
const request = new TextEncoder().encode('hoi/address-check/1');
const prefix = 'hoi/address/1 ';

const failures = {
  unsupported: 'This device cannot check the selected route.',
  browser: 'WebRTC is unavailable in this browser.',
  unreachable: 'This device could not open a connection to the selected node.',
  timeout: 'The selected node did not answer in time.',
  closed: 'The node closed the connection before returning an address. Its version or settings may not support anonymous checks.',
  unexpected: 'The node replied, but the reply contained no valid IP address.',
  withheld: 'The node answered without an address. This can happen when a secure WebSocket is behind a proxy.',
} as const;

export class ConnectionCheckError extends Error {
  constructor(readonly kind: keyof typeof failures, detail = '') {
    super(detail ? failures[kind] + ' ' + detail : failures[kind]);
    this.name = 'ConnectionCheckError';
  }
}

function socketRoute(route: string): boolean {
  try {
    const url = new URL(route);
    return !url.username && !url.password && !url.hash && !url.search
      && (url.protocol === 'wss:' || (url.protocol === 'ws:' && ['127.0.0.1', '[::1]', 'localhost'].includes(url.hostname)));
  } catch { return false; }
}

function quicRoute(route: string): boolean {
  const parts = /^[0-9a-f]{64}@(?:\[([0-9a-f:.]+)\]|([0-9.]+)):(\d{1,5})$/i.exec(route);
  if (!parts) return false;
  const port = Number(parts[3]);
  return port > 0 && port <= 65535 && numericAddress(parts[1] || parts[2]);
}

export function canCheckRoute(route: string): boolean {
  const rtc = parseRoute(route);
  return (rtc !== null && numericAddress(rtc.ip)) || socketRoute(route) || (isDesktopShell() && quicRoute(route));
}

export function readObservation(bytes: Uint8Array): string | null {
  if (bytes.length > prefix.length + 45) return null;
  const reply = new TextDecoder().decode(bytes);
  if (!reply.startsWith(prefix)) return null;
  const ip = reply.slice(prefix.length);
  return numericAddress(ip) ? ip : null;
}

function directCheck(route: string, transport: 'webrtc' | 'websocket', signal: AbortSignal): Promise<Observation> {
  return new Promise((resolve, reject) => {
    const controller = new AbortController();
    let connected = false;
    let close = () => {};
    let settled = false;
    const finish = (result: string | ConnectionCheckError) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      signal.removeEventListener('abort', abort);
      controller.abort();
      close();
      if (signal.aborted) reject(signal.reason);
      else if (result instanceof ConnectionCheckError) reject(result);
      else resolve({ address: result, transport });
    };
    const received = (bytes: Uint8Array) => {
      const address = readObservation(bytes);
      if (address) finish(address);
      else {
        const empty = bytes.length === prefix.length && new TextDecoder().decode(bytes) === prefix;
        finish(new ConnectionCheckError(empty ? 'withheld' : 'unexpected'));
      }
    };
    const closed = () => finish(new ConnectionCheckError(connected ? 'closed' : 'unreachable'));
    const abort = () => finish(new ConnectionCheckError('closed'));
    const timer = setTimeout(() => finish(new ConnectionCheckError('timeout')), 14000);
    signal.addEventListener('abort', abort, { once: true });
    if (transport === 'webrtc') {
      // Client mode is silent until the first application frame. The remote DTLS
      // certificate is pinned to the route, and this probe never sends a sync hello.
      void openRtc([route], 'c', {
        message: received,
        close: reason => reason === 'timeout' ? finish(new ConnectionCheckError('timeout')) : closed(),
      }, controller.signal).then(link => {
        if (settled) { link.close(); return; }
        connected = true;
        close = () => link.close();
        link.send(request);
      }).catch(closed);
    } else {
      try {
        const socket = new WebSocket(route);
        close = () => socket.close();
        socket.binaryType = 'arraybuffer';
        socket.onopen = () => {
          if (!settled) {
            connected = true;
            try { socket.send(request); } catch { closed(); }
          }
        };
        socket.onmessage = e => {
          if (e.data instanceof ArrayBuffer) received(new Uint8Array(e.data));
          else finish(new ConnectionCheckError('unexpected'));
        };
        socket.onerror = socket.onclose = closed;
      } catch { closed(); }
    }
  });
}

/** Older nodes with a public RTC listener already answer ordinary STUN. Gather on a separate
 * socket with that exact numeric destination as its only server. A reflexive UDP candidate
 * is evidence of that STUN path; host, relay and peer-reflexive candidates are not. */
async function sameSocketStun(rtc: NonNullable<ReturnType<typeof parseRoute>>, signal: AbortSignal): Promise<string | null> {
  signal.throwIfAborted();
  const server = `stun:${rtc.v6 ? '[' + rtc.ip + ']' : rtc.ip}:${rtc.port}`;
  const pc = new RTCPeerConnection({ iceServers: [{ urls: server }] });
  try {
    return await new Promise<string | null>((resolve, reject) => {
      const finish = (address: string | null) => {
        clearTimeout(timer);
        signal.removeEventListener('abort', abort);
        pc.onicecandidate = null;
        if (signal.aborted) reject(signal.reason);
        else resolve(address);
      };
      const abort = () => finish(null);
      const timer = setTimeout(abort, 6000);
      signal.addEventListener('abort', abort, { once: true });
      pc.onicecandidate = ({ candidate }) => {
        if (!candidate) { finish(null); return; }
        const reportedServer = (candidate as RTCIceCandidate & { url?: string | null }).url;
        if (reportedServer && reportedServer !== server) return;
        if (candidate.type === 'srflx' && candidate.protocol === 'udp' && numericAddress(candidate.address)) finish(candidate.address);
      };
      void (async () => {
        pc.createDataChannel('check');
        const offer = await pc.createOffer();
        if (pc.signalingState !== 'closed') await pc.setLocalDescription(offer);
      })().catch(abort);
    });
  } finally { pc.close(); }
}

export async function checkConnection(route: string, signal: AbortSignal): Promise<Observation> {
  signal.throwIfAborted();
  if (!canCheckRoute(route)) throw new ConnectionCheckError('unsupported');
  if (isDesktopShell()) {
    const { invoke } = await import('@tauri-apps/api/core');
    signal.throwIfAborted();
    const result = await invoke<Observation>('hoi_check_connection', { route });
    signal.throwIfAborted();
    return result;
  }

  const rtc = parseRoute(route);
  if (!rtc) return directCheck(route, 'websocket', signal);
  if (typeof RTCPeerConnection === 'undefined') throw new ConnectionCheckError('browser');
  try {
    return await directCheck(route, 'webrtc', signal);
  } catch (error) {
    signal.throwIfAborted();
    if (!(error instanceof ConnectionCheckError)) throw error;
    const address = await sameSocketStun(rtc, signal).catch(() => null);
    signal.throwIfAborted();
    if (address) return { address, transport: 'udp' };
    throw new ConnectionCheckError(error.kind, 'The browser also received no usable UDP address from that same node.');
  }
}
