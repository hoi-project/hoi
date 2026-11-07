// Load data for a screen and refetch when its reactive inputs change (route params, app.rev).
// Reads inside `fetcher` before its first await are tracked. Old data stays on screen while refetching.
import { errorText } from './toast.svelte';

export interface Resource<T> {
  readonly value: T | undefined;
  readonly error: string;
  readonly loading: boolean;
  /** Replace the value locally (optimistic updates). */
  set(v: T): void;
  reload(): void;
}

export function resource<T>(fetcher: () => Promise<T>): Resource<T> {
  let value = $state.raw<T | undefined>(undefined);
  let error = $state('');
  let loading = $state(true);
  let nonce = $state(0);
  let token = 0;

  $effect(() => {
    void nonce;
    const mine = ++token;
    let p: Promise<T>;
    try {
      p = fetcher();
    } catch (e) {
      error = errorText(e);
      loading = false;
      return;
    }
    p.then(
      (v) => {
        if (mine !== token) return;
        value = v;
        error = '';
        loading = false;
      },
      (e) => {
        if (mine !== token) return;
        error = errorText(e);
        loading = false;
      },
    );
  });

  return {
    get value() {
      return value;
    },
    get error() {
      return error;
    },
    get loading() {
      return loading;
    },
    set(v: T) {
      value = v;
    },
    reload() {
      nonce++;
    },
  };
}
