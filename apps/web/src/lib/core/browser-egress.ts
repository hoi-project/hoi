import { numericAddress } from './rtc-addresses';

export const BROWSER_EGRESS_PROVIDER = 'api64.ipify.org';
export const BROWSER_EGRESS_URL = `https://${BROWSER_EGRESS_PROVIDER}/`;
const MAX_BYTES = 64;
const TIMEOUT_MS = 8_000;

function publicAddress(text: string): string | undefined {
  if (!numericAddress(text)) return;
  if (text.includes(':')) {
    const ip = new URL(`http://[${text}]/`).hostname.slice(1, -1);
    const [first, second] = ip.split(':').map(word => parseInt(word || '0', 16));
    // Accept global unicast, excluding protocol assignments, 6to4 and documentation ranges.
    if (first < 0x2000 || first >= 0x4000 || first === 0x2002
      || (first === 0x2001 && (second < 0x200 || second === 0xdb8))
      || (first === 0x3fff && second < 0x1000)) return;
    return ip;
  }
  const [a, b, c] = text.split('.').map(Number);
  if (a === 0 || a === 10 || a === 127 || a >= 224
    || (a === 100 && b >= 64 && b <= 127) || (a === 169 && b === 254)
    || (a === 172 && b >= 16 && b <= 31)
    || (a === 192 && (b === 168 || (b === 0 && (c === 0 || c === 2)) || (b === 88 && c === 99)))
    || (a === 198 && (b === 18 || b === 19 || (b === 51 && c === 100)))
    || (a === 203 && b === 0 && c === 113)) return;
  return text;
}

const invalidReply = () => new Error('The IP service returned an invalid or non-public address.');
const networkFailure = () => new Error('Could not reach the IP service. Check your internet connection and whether your browser or VPN blocks it.');

async function readAddress(response: Response, signal: AbortSignal): Promise<string> {
  if (!response.body || Number(response.headers.get('content-length')) > MAX_BYTES) throw invalidReply();
  const reader = response.body.getReader();
  const cancel = () => { void reader.cancel().catch(() => {}); };
  signal.addEventListener('abort', cancel, { once: true });
  const raw = new Uint8Array(MAX_BYTES);
  let used = 0;
  try {
    while (true) {
      signal.throwIfAborted();
      const { done, value } = await reader.read().catch(() => { throw networkFailure(); });
      if (done) break;
      if (used + value.byteLength > MAX_BYTES) {
        cancel();
        throw invalidReply();
      }
      raw.set(value, used);
      used += value.byteLength;
    }
    signal.throwIfAborted();
    const ip = publicAddress(new TextDecoder().decode(raw.subarray(0, used)).trim());
    if (!ip) throw invalidReply();
    return ip;
  } finally {
    signal.removeEventListener('abort', cancel);
    reader.releaseLock();
  }
}

/** Run only after the user chooses to contact the named HTTPS observer. */
export async function fetchBrowserEgress(signal: AbortSignal): Promise<string> {
  signal.throwIfAborted();
  const controller = new AbortController();
  const abort = () => controller.abort(signal.reason);
  signal.addEventListener('abort', abort, { once: true });
  const timer = setTimeout(() => controller.abort(new Error('The IP service timed out. Try again.')), TIMEOUT_MS);
  try {
    const response = await fetch(BROWSER_EGRESS_URL, {
      method: 'GET', mode: 'cors', credentials: 'omit', referrerPolicy: 'no-referrer',
      cache: 'no-store', redirect: 'error', signal: controller.signal,
    }).catch(() => { throw networkFailure(); });
    controller.signal.throwIfAborted();
    if (!response.ok) throw new Error(`The IP service returned HTTP ${response.status}. Try again later.`);
    return await readAddress(response, controller.signal);
  } catch (error) {
    controller.signal.throwIfAborted();
    throw error;
  } finally {
    clearTimeout(timer);
    signal.removeEventListener('abort', abort);
    // Release any unread body too (for example, an HTTP error or oversized Content-Length).
    controller.abort();
  }
}
