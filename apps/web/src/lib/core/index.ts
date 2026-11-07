// Picks the backend the UI talks to. The UI only ever sees `HoiCore`.
//   VITE_HOI_BACKEND=mock  → in-memory dev mock (never in production bundles: the branch is dead code there)
//   inside Tauri           → native hoi-core behind Tauri commands
//   otherwise              → hoi-wasm in a Web Worker
import type { HoiCore } from '@hoi/core-api';

let pending: Promise<HoiCore> | null = null;

export type VaultStatus = { state: 'create' | 'migrate' | 'locked' | 'ready' | 'blocked' };
export const isMock = import.meta.env.VITE_HOI_BACKEND === 'mock';
const isDesktop = () => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

async function vaultCall(command: 'status' | 'unlock' | 'lock' | 'takeOver' | 'reset', input?: string): Promise<unknown> {
  if (isMock) {
    if (command === 'takeOver') await (await getCore()).takeOver();
    return { state: 'ready' };
  }
  if (__HOI_DESKTOP__ || isDesktop()) {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke(`hoi_vault_${command}`, command === 'unlock' ? { passphrase: input } : command === 'reset' ? { confirmation: input } : {});
  } else {
    const { workerVaultCall } = await import('./worker-core');
    const method = { status: '__vaultStatus', unlock: '__unlock', lock: '__lock', takeOver: '__takeOver', reset: '__resetVault' }[command];
    return workerVaultCall(method, command === 'unlock' || command === 'reset' ? [input] : []);
  }
}

export const getVaultStatus = () => vaultCall('status') as Promise<VaultStatus>;

export type OsUnlockStatus = { enabled: boolean; configured: boolean };
async function osUnlockCall(command: string, passphrase?: string): Promise<unknown> {
  if (!(__HOI_DESKTOP__ || isDesktop()) || isMock) throw new Error('OS unlock is available in the desktop app.');
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke(command, passphrase === undefined ? {} : { passphrase });
}
export const osUnlockStatus = () => osUnlockCall('hoi_os_unlock_status') as Promise<OsUnlockStatus>;
export const enableOsUnlock = (passphrase: string) => osUnlockCall('hoi_os_unlock_enable', passphrase);
export const disableOsUnlock = () => osUnlockCall('hoi_os_unlock_disable');
export const unlockWithOs = () => osUnlockCall('hoi_os_unlock');

/** Passkeys that open this browser's vault. Desktop unlock uses native commands. */
async function passkeyCall(method: string, args: unknown[] = []): Promise<unknown> {
  if (isMock || __HOI_DESKTOP__ || isDesktop()) {
    if (method === '__vaultPasskeys') return [];
    throw new Error('Passkeys unlock hoi in a browser.');
  }
  const { workerVaultCall } = await import('./worker-core');
  return workerVaultCall(method, args);
}
export const vaultPasskeys = () => passkeyCall('__vaultPasskeys') as Promise<{ credential: string; salt: string }[]>;
export const unlockWithPasskey = (credential: string, prf: string) => passkeyCall('__unlockPasskey', [credential, prf]);
export const addVaultPasskey = (credential: string, salt: string, prf: string) => passkeyCall('__addPasskey', [credential, salt, prf]);
export const removeVaultPasskeys = () => passkeyCall('__removePasskeys');
export const unlockVault = (passphrase: string) => vaultCall('unlock', passphrase);
export const takeOverVault = () => vaultCall('takeOver');
/** Explicit recovery while locked. This deletes local storage; it cannot decrypt it. */
export async function resetVault(confirmation: string): Promise<void> {
  await vaultCall('reset', confirmation);
  try {
    for (const key of ['hoi:theme', 'hoi:motion', 'hoi:sync-hidden', 'hoi:consent:network', 'hoi:vault-login']) localStorage.removeItem(key);
    sessionStorage.removeItem('hoi:sort');
  }
  catch { /* Storage preferences must not prevent returning to setup. */ }
  location.reload();
}
export async function lockVault(): Promise<void> {
  await vaultCall('lock');
  // Destroy page state, rendered content and workers. Never retain a password in session storage.
  location.reload();
}

export function getCore(): Promise<HoiCore> {
  return (pending ??= load());
}

async function load(): Promise<HoiCore> {
  if (import.meta.env.VITE_HOI_BACKEND === 'mock') {
    const { createMockCore } = await import('./mock-core');
    return createMockCore();
  }
  if (__HOI_DESKTOP__ || isDesktop()) {
    const { createTauriCore } = await import('./tauri-core');
    return await createTauriCore();
  } else {
    const { createWorkerCore } = await import('./worker-core');
    return await createWorkerCore();
  }
}
