// Toasts confirm every signed action: ink pill, bottom centre, one at a time.
import { HoiError } from '@hoi/core-api';

export interface Toast {
  id: number;
  text: string;
  kind: 'ok' | 'error';
}

class Toaster {
  current: Toast | null = $state(null);
  #seq = 0;
  #timer: ReturnType<typeof setTimeout> | null = null;

  show(text: string, kind: Toast['kind'] = 'ok', ms = kind === 'error' ? 4200 : 2600): void {
    this.current = { id: ++this.#seq, text, kind };
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = setTimeout(() => (this.current = null), ms);
  }

  dismiss(): void {
    this.current = null;
  }
}

export const toasts = new Toaster();

export function toast(text: string): void {
  toasts.show(text, 'ok');
}

/** Errors become toasts with the HoiError message (short, plain). */
export function toastError(e: unknown): void {
  console.warn(e);
  toasts.show(errorText(e), 'error');
}

export function errorText(e: unknown): string {
  if (e instanceof HoiError) return e.message;
  if (e && typeof e === 'object' && 'message' in e && typeof (e as Error).message === 'string') {
    // HoiError may cross a worker boundary as a plain object.
    return (e as Error).message;
  }
  return 'Something went wrong on this device.';
}
