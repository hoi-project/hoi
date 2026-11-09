// A direct link with another member's browser. A node both browsers are linked to
// passes the offer and the answer (the core's `sig`), and its WebRTC port answers STUN, so each
// side learns the address it is seen from. Everything else is plain WebRTC: both sides make their
// own certificate, write their own half of the session, and gather their candidates before the
// description goes out (no trickling, one message each way). The data channel then carries the
// same sync messages as a link to a node, framed the same way (rtc-frames.ts).
//
// Lives on the page: Web Workers can't create RTCPeerConnection.

import { frame, type FrameHandlers } from './rtc-frames';

/** How long candidates are gathered before the description goes out without the rest. */
const GATHER_MS = 3000;
/** How long the channel may take to open once both descriptions are set. */
const OPEN_MS = 15_000;
/** An offer or an answer as it travels: hoi-sync MAX_SIG. */
const MAX_SIG = 8 * 1024;

export interface DirectLink {
  /** The other side's answer, for the side that offered. */
  accept(answer: Uint8Array): Promise<void>;
  send(bytes: Uint8Array): void;
  close(): void;
}

export interface DirectHandlers extends FrameHandlers {
  /** The channel is open. */
  open(): void;
}

const enc = new TextEncoder();
const dec = new TextDecoder();

/** `stun:` addresses the core gave, as ICE servers; nothing else is ever contacted. */
function servers(stun: string[]): RTCIceServer[] {
  const ok = stun.filter((s) => /^stun:(\[[0-9a-fA-F:.]+\]|[0-9.]+):\d{1,5}$/.test(s)).slice(0, 4);
  return ok.length ? [{ urls: ok }] : [];
}

/** The local description once gathering finished, or after GATHER_MS with what was found. */
async function gathered(pc: RTCPeerConnection): Promise<Uint8Array> {
  if (pc.iceGatheringState !== 'complete') {
    await new Promise<void>((resolve) => {
      const t = setTimeout(resolve, GATHER_MS);
      pc.addEventListener('icegatheringstatechange', () => {
        if (pc.iceGatheringState === 'complete') {
          clearTimeout(t);
          resolve();
        }
      });
    });
  }
  const d = pc.localDescription;
  if (!d) throw new Error('No session description.');
  const bytes = enc.encode(JSON.stringify({ type: d.type, sdp: d.sdp }));
  if (bytes.length > MAX_SIG) throw new Error('That session description is too large.');
  return bytes;
}

function parse(bytes: Uint8Array, type: 'offer' | 'answer'): RTCSessionDescriptionInit {
  const d = JSON.parse(dec.decode(bytes)) as { type?: unknown; sdp?: unknown };
  if (d.type !== type || typeof d.sdp !== 'string' || d.sdp.length > MAX_SIG) throw new Error(`Not an ${type}.`);
  return { type, sdp: d.sdp };
}

function start(stun: string[], on: DirectHandlers) {
  if (typeof RTCPeerConnection === 'undefined') throw new Error('This browser has no WebRTC.');
  const pc = new RTCPeerConnection({ iceServers: servers(stun), iceTransportPolicy: 'all', bundlePolicy: 'max-bundle' });
  const dc = pc.createDataChannel('hoi', { negotiated: true, id: 0, ordered: true });
  const ch = frame(pc, dc, on, 'browser');
  const timer = setTimeout(() => ch.shut('timeout'), GATHER_MS + OPEN_MS);
  dc.addEventListener('open', () => {
    clearTimeout(timer);
    on.open();
  });
  const link = (accept: DirectLink['accept']): DirectLink => ({
    accept,
    send: ch.send,
    close: () => ch.shut('closed here'),
  });
  return { pc, ch, link };
}

/** Starts a link as the side that offers: resolves with the offer to pass on, and the link. */
export async function offerDirect(stun: string[], on: DirectHandlers): Promise<{ offer: Uint8Array; link: DirectLink }> {
  const { pc, ch, link } = start(stun, on);
  try {
    await pc.setLocalDescription(await pc.createOffer());
    const offer = await gathered(pc);
    return {
      offer,
      link: link(async (answer) => {
        try {
          await pc.setRemoteDescription(parse(answer, 'answer'));
        } catch (e) {
          ch.shut('a bad answer');
          throw e;
        }
      }),
    };
  } catch (e) {
    ch.shut('could not offer');
    throw e;
  }
}

/** Answers another browser's offer: resolves with the answer to pass back, and the link. */
export async function answerDirect(stun: string[], offer: Uint8Array, on: DirectHandlers): Promise<{ answer: Uint8Array; link: DirectLink }> {
  const { pc, ch, link } = start(stun, on);
  try {
    await pc.setRemoteDescription(parse(offer, 'offer'));
    await pc.setLocalDescription(await pc.createAnswer());
    const answer = await gathered(pc);
    return { answer, link: link(async () => undefined) };
  } catch (e) {
    ch.shut('could not answer');
    throw e;
  }
}
