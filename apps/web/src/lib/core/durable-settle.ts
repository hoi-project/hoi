import type { Change } from './idb';

interface JournalNode {
  take_changes(): Change[];
  take_events(): string;
}

/** The wasm journal is drained by the host, so retain it until IndexedDB commits it. */
export function createDurableSettler(
  node: JournalNode,
  persist: (changes: Change[]) => Promise<void>,
  flushOutbox: () => void,
  emit: (event: unknown) => void,
): () => Promise<void> {
  let pending: Change[] = [];
  return async () => {
    pending = pending.concat(node.take_changes());
    await persist(pending);
    pending = [];
    // Events and outgoing records remain in the core until storage has succeeded.
    flushOutbox();
    for (const event of JSON.parse(node.take_events()) as unknown[]) emit(event);
  };
}
