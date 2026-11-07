// WebRTC to a native hoi node with no signaling server (core/crates/hoi-net/src/rtc.rs).
// A route is `rtc:<base64url sha-256 of the node's DTLS certificate>@<ip:port>`. The browser
// writes the node's half of the session description itself: the node is an ICE-lite peer that
// serves binding requests whose username carries our certificate's fingerprint, once this
// browser's ICE agent has answered the node's own binding request (proof that it receives at its
// address), and the browser pins the node's certificate by the fingerprint in the route. Nothing
// else is contacted: no STUN, no TURN, no signaling.
//
// Lives on the page: Web Workers can't create RTCPeerConnection. The core worker asks for links
// and exchanges whole sync messages with them (core.worker.ts).

import { frame } from './rtc-frames';

export type RtcMode = 'c' | 'p';

export interface RtcLink {
  send(bytes: Uint8Array): void;
  close(): void;
  /** What the node binds to its key: both certificate fingerprints (base64url) and the
   * ICE username fragment of this channel. */
  binding: { node: string; browser: string; ufrag: string };
}

export interface RtcHandlers {
  message(bytes: Uint8Array): void;
  close(reason: string): void;
}

interface Route {
  fp: Uint8Array;
  ip: string;
  port: number;
  v6: boolean;
}

const OPEN_TIMEOUT_MS = 12_000;

function unb64url(s: string): Uint8Array {
  const b = atob(s.replace(/-/g, '+').replace(/_/g, '/') + '==='.slice((s.length + 3) % 4));
  return Uint8Array.from(b, (c) => c.charCodeAt(0));
}

export function parseRoute(route: string): Route | null {
  const m = /^rtc:([A-Za-z0-9_-]{43})@(?:\[([0-9a-fA-F:.]+)\]|([0-9.]+)):(\d{1,5})$/.exec(route);
  if (!m) return null;
  const port = Number(m[4]);
  if (!port || port > 65535) return null;
  const fp = unb64url(m[1]);
  if (fp.length !== 32) return null;
  return m[2] ? { fp, ip: m[2], port, v6: true } : { fp, ip: m[3], port, v6: false };
}

/** Routes grouped by node (one certificate = one node), keeping their order. */
export function groupRoutes(routes: string[]): string[][] {
  const groups = new Map<string, string[]>();
  for (const r of routes) {
    const fp = /^rtc:([A-Za-z0-9_-]{43})@/.exec(r)?.[1];
    if (!fp || !parseRoute(r)) continue;
    const g = groups.get(fp) ?? [];
    g.push(r);
    groups.set(fp, g);
  }
  return [...groups.values()];
}

const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, '0').toUpperCase()).join(':');
const b64 = (b: Uint8Array) => btoa(String.fromCharCode(...b)).replace(/=+$/, '');

function randomIceChars(n: number): string {
  const abc = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
  return Array.from(crypto.getRandomValues(new Uint8Array(n)), (x) => abc[x % abc.length]).join('');
}

/** The node's half of the session: ICE-lite, passive DTLS, its certificate, its addresses. */
export function answerSdp(routes: Route[], ufrag: string, mid: string): string {
  const first = routes[0];
  const fam = (r: Route) => (r.v6 ? 'IP6' : 'IP4');
  const lines = [
    'v=0',
    `o=- 0 0 IN ${fam(first)} ${first.ip}`,
    's=-',
    `c=IN ${fam(first)} ${first.ip}`,
    't=0 0',
    'a=ice-lite',
    `m=application ${first.port} UDP/DTLS/SCTP webrtc-datachannel`,
    `a=mid:${mid}`,
    'a=setup:passive',
    `a=ice-ufrag:${ufrag}`,
    `a=ice-pwd:${ufrag}`,
    `a=fingerprint:sha-256 ${hex(first.fp)}`,
    'a=sctp-port:5000',
    'a=max-message-size:262144',
    ...routes.map((r, i) => `a=candidate:${i + 1} 1 UDP ${2130706431 - i} ${r.ip} ${r.port} typ host`),
    'a=end-of-candidates',
    '',
  ];
  return lines.join('\r\n');
}

/** Opens a data channel to the node behind `routes` (all with the same fingerprint). */
export async function openRtc(routes: string[], mode: RtcMode, on: RtcHandlers): Promise<RtcLink> {
  if (typeof RTCPeerConnection === 'undefined') throw new Error('This browser has no WebRTC.');
  const parsed = routes.map(parseRoute).filter((r): r is Route => !!r);
  if (!parsed.length) throw new Error('No usable route.');
  const node = parsed.filter((r) => hex(r.fp) === hex(parsed[0].fp));

  const cert = await RTCPeerConnection.generateCertificate({ name: 'ECDSA', namedCurve: 'P-256' } as EcKeyGenParams);
  const mine = cert.getFingerprints().find((f) => f.algorithm === 'sha-256');
  if (!mine?.value) throw new Error('No certificate fingerprint.');
  const fp = Uint8Array.from(mine.value.split(':'), (h) => parseInt(h, 16));
  const ufrag = `h1${mode}${b64(fp)}${randomIceChars(10)}`;

  const pc = new RTCPeerConnection({ certificates: [cert], iceServers: [] });
  const dc = pc.createDataChannel('hoi', { negotiated: true, id: 0, ordered: true });
  const ch = frame(pc, dc, on);

  const offer = await pc.createOffer();
  const sdp = (offer.sdp ?? '').replace(/^a=ice-ufrag:.*$/gm, `a=ice-ufrag:${ufrag}`).replace(/^a=ice-pwd:.*$/gm, `a=ice-pwd:${ufrag}`);
  const mid = /^a=mid:(\S+)/m.exec(sdp)?.[1] ?? '0';
  await pc.setLocalDescription({ type: 'offer', sdp });
  await pc.setRemoteDescription({ type: 'answer', sdp: answerSdp(node, ufrag, mid) });

  await new Promise<void>((resolve, reject) => {
    const t = setTimeout(() => {
      ch.shut('timeout');
      reject(new Error('No answer over WebRTC.'));
    }, OPEN_TIMEOUT_MS);
    dc.addEventListener('open', () => {
      clearTimeout(t);
      resolve();
    });
    pc.addEventListener('connectionstatechange', () => {
      if (ch.closed()) {
        clearTimeout(t);
        reject(new Error('The WebRTC connection failed.'));
      }
    });
  });

  const b64url = (u: Uint8Array) => btoa(String.fromCharCode(...u)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  return {
    binding: { node: b64url(node[0].fp), browser: b64url(fp), ufrag },
    send: ch.send,
    close() {
      ch.shut('closed here');
    },
  };
}
