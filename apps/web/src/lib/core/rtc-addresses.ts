// Browser-only observations of existing connections. They never enter the core, storage,
// signaling, or logs. A selected reflexive candidate is a browser report, not a VPN check.
export interface RtcAddress {
  id: number;
  kind: 'node' | 'browser';
  connected: boolean;
  address: string | null;
}

function numericAddress(value: unknown): value is string {
  if (typeof value !== 'string' || !value) return false;
  if (/^(0|[1-9]\d{0,2})(\.(0|[1-9]\d{0,2})){3}$/.test(value)) {
    return value !== '0.0.0.0' && value.split('.').every((part) => Number(part) <= 255);
  }
  if (!value.includes(':') || !/^[\da-f:.]+$/i.test(value)) return false;
  try { return new URL(`http://[${value}]/`).hostname !== '[::]'; }
  catch { return false; }
}

/** Only the transport's selected pair is evidence of a current route. Other gathered or
 * nominated candidates may be unused, and a host candidate does not reveal a NAT mapping. */
export function selectedAddress(report: RTCStatsReport): string | null {
  for (const transport of report.values()) {
    if (transport.type !== 'transport' || typeof transport.selectedCandidatePairId !== 'string') continue;
    const pair = report.get(transport.selectedCandidatePairId);
    if (pair?.type !== 'candidate-pair' || pair.state !== 'succeeded' || typeof pair.localCandidateId !== 'string') continue;
    const candidate = report.get(pair.localCandidateId);
    if (candidate?.type === 'local-candidate' && (candidate.candidateType === 'srflx' || candidate.candidateType === 'prflx') && numericAddress(candidate.address)) {
      return candidate.address;
    }
  }
  return null;
}

export function createRtcAddresses() {
  const rows = new Map<number, RtcAddress>();
  const subscribers = new Set<(rows: RtcAddress[]) => void>();
  const observers = new Set<() => void>();
  let next = 1;
  const snapshot = () => Array.from(rows.values(), (row) => ({ ...row }));
  const publish = () => { for (const fn of subscribers) fn(snapshot()); };

  return {
    subscribe(fn: (rows: RtcAddress[]) => void) {
      const first = subscribers.size === 0;
      subscribers.add(fn);
      if (first) for (const changed of observers) changed();
      fn(snapshot());
      return () => {
        subscribers.delete(fn);
        if (!subscribers.size) for (const changed of observers) changed();
      };
    },
    track(pc: RTCPeerConnection, kind: RtcAddress['kind']) {
      const id = next++;
      let stopped = false;
      let generation = 0;
      let interval: ReturnType<typeof setInterval> | undefined;
      let ice: RTCIceTransport | undefined;
      const deadlines = new Set<ReturnType<typeof setTimeout>>();
      const connected = () => pc.connectionState === 'connected' && (pc.iceConnectionState === 'connected' || pc.iceConnectionState === 'completed');

      function update(address: string | null) {
        const live = connected();
        const previous = rows.get(id);
        const nextAddress = live ? address : null;
        if (previous?.connected === live && previous.address === nextAddress) return;
        rows.set(id, { id, kind, connected: live, address: nextAddress });
        publish();
      }

      async function refresh() {
        const current = ++generation;
        if (stopped || !connected() || !subscribers.size) { if (!stopped) update(null); return; }
        // A stalled stats read must not leave an old address looking current forever.
        const deadline = setTimeout(() => {
          deadlines.delete(deadline);
          if (!stopped && current === generation) { ++generation; update(null); }
        }, 1000);
        deadlines.add(deadline);
        try {
          const report = await pc.getStats();
          if (!stopped && current === generation) update(selectedAddress(report));
        } catch {
          if (!stopped && current === generation) update(null);
        } finally {
          clearTimeout(deadline);
          deadlines.delete(deadline);
        }
      }

      function changed() {
        if (stopped) return;
        ++generation;
        update(null);
        const currentIce = pc.sctp?.transport.iceTransport;
        if (ice !== currentIce) {
          ice?.removeEventListener('selectedcandidatepairchange', changed);
          ice = currentIce;
          ice?.addEventListener('selectedcandidatepairchange', changed);
        }
        clearInterval(interval);
        interval = undefined;
        for (const deadline of deadlines) clearTimeout(deadline);
        deadlines.clear();
        if (connected() && subscribers.size) {
          void refresh();
          // Some browsers omit the selected-pair event; refresh without sending requests.
          interval = setInterval(() => void refresh(), 2000);
        }
      }

      pc.addEventListener('connectionstatechange', changed);
      pc.addEventListener('iceconnectionstatechange', changed);
      pc.addEventListener('icegatheringstatechange', changed);
      observers.add(changed);
      changed();
      return () => {
        if (stopped) return;
        stopped = true;
        ++generation;
        clearInterval(interval);
        for (const deadline of deadlines) clearTimeout(deadline);
        deadlines.clear();
        pc.removeEventListener('connectionstatechange', changed);
        pc.removeEventListener('iceconnectionstatechange', changed);
        pc.removeEventListener('icegatheringstatechange', changed);
        ice?.removeEventListener('selectedcandidatepairchange', changed);
        observers.delete(changed);
        rows.delete(id);
        publish();
      };
    },
  };
}

export const rtcAddresses = createRtcAddresses();
