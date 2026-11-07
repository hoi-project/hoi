// What only the desktop shell can do for the UI, with a plain-web fallback: open a link in the
// person's own browser, start with the computer, size the window for the companion or the full
// app. Everything else goes through HoiCore.

const inTauri = () => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export const isDesktopShell = inTauri;

/** Opens `url` in the default browser (desktop) or a new tab (web). */
export async function openInBrowser(url: string): Promise<void> {
  if (inTauri()) {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('hoi_open_url', { url });
    return;
  }
  window.open(url, '_blank', 'noopener');
}

/** Whether the desktop app starts when you log in; null where that isn't a thing. */
export async function autostart(on?: boolean): Promise<boolean | null> {
  if (!inTauri()) return null;
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<boolean>('hoi_autostart', { on: on ?? null });
}

/** The desktop window takes the size of the face it shows: the companion, or the full app. */
export async function windowFace(full: boolean): Promise<void> {
  if (!inTauri()) return;
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('hoi_face', { full });
}
