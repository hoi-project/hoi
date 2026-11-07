// Whole sync messages over a WebRTC data channel: each one is a 4-byte length and the bytes, cut
// into 16 KiB data channel messages. Shared by links to nodes (rtc-link.ts) and between
// browsers (rtc-direct.ts).

/** Data channel messages we send; every browser and the node take at least this much. */
const CHUNK = 16 * 1024;
/** Largest sync message (hoi-sync MAX_MESSAGE). */
const MAX_MESSAGE = 16 * 1024 * 1024 + 64 * 1024;
/** Above this much unsent, wait for the channel to drain. */
const HIGH_WATER = 4 * 1024 * 1024;

export interface FrameHandlers {
  message(bytes: Uint8Array): void;
  close(reason: string): void;
}

export interface Framed {
  send(bytes: Uint8Array): void;
  shut(reason: string): void;
  closed(): boolean;
}

/** Wires framing onto `dc`; closing either side closes both and calls `on.close` once. */
export function frame(pc: RTCPeerConnection, dc: RTCDataChannel, on: FrameHandlers): Framed {
  dc.binaryType = 'arraybuffer';
  dc.bufferedAmountLowThreshold = 1024 * 1024;
  let closed = false;
  const queue: Uint8Array<ArrayBuffer>[] = [];
  /** The length header as it arrives (chunks may split it), then the frame being filled. */
  const head = new Uint8Array(4);
  let headLen = 0;
  let buf: Uint8Array | null = null;
  let filled = 0;

  function shut(reason: string) {
    if (closed) return;
    closed = true;
    try {
      dc.close();
      pc.close();
    } catch {
      /* already closed */
    }
    on.close(reason);
  }

  function pump() {
    while (!closed && queue.length && dc.readyState === 'open' && dc.bufferedAmount < HIGH_WATER) {
      dc.send(queue.shift()!);
    }
  }

  dc.onbufferedamountlow = pump;
  dc.addEventListener('open', pump);
  // Each frame is allocated once, at its announced size, and chunks are copied in at a running
  // offset: a peer trickling a 16 MiB frame in 16 KiB pieces costs one copy per byte, not one
  // copy of everything so far per piece.
  dc.onmessage = (e: MessageEvent<ArrayBuffer>) => {
    const chunk = new Uint8Array(e.data);
    let pos = 0;
    while (pos < chunk.length) {
      if (!buf) {
        const take = Math.min(4 - headLen, chunk.length - pos);
        head.set(chunk.subarray(pos, pos + take), headLen);
        headLen += take;
        pos += take;
        if (headLen < 4) break;
        const len = new DataView(head.buffer).getUint32(0);
        headLen = 0;
        if (len === 0 || len > MAX_MESSAGE) return shut('bad frame');
        buf = new Uint8Array(len);
        filled = 0;
      }
      const take = Math.min(buf.length - filled, chunk.length - pos);
      buf.set(chunk.subarray(pos, pos + take), filled);
      filled += take;
      pos += take;
      if (filled === buf.length) {
        const done = buf;
        buf = null;
        on.message(done);
      }
    }
  };
  dc.onclose = () => shut('closed');
  pc.addEventListener('connectionstatechange', () => {
    if (pc.connectionState === 'failed' || pc.connectionState === 'closed') shut(pc.connectionState);
  });

  return {
    send(bytes) {
      if (closed) return;
      const f = new Uint8Array(4 + bytes.length);
      new DataView(f.buffer).setUint32(0, bytes.length);
      f.set(bytes, 4);
      for (let i = 0; i < f.length; i += CHUNK) queue.push(f.subarray(i, Math.min(i + CHUNK, f.length)));
      pump();
    },
    shut,
    closed: () => closed,
  };
}
