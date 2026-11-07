// What the window asks the setup app's Rust side for (apps/setup/src-tauri/src/lib.rs). In a
// plain browser (a test, a screenshot) a mock stands in and no server is contacted.
export interface Target {
  host: string;
  port: number;
  user: string;
}
export type Auth = { kind: 'agent' } | { kind: 'key'; path: string; passphrase?: string } | { kind: 'password'; password: string };
export type AuthKind = { kind: 'agent' } | { kind: 'key'; path: string } | { kind: 'password' };
export interface HostKey {
  algorithm: string;
  fingerprint: string;
}
export interface Profile {
  id: string;
  label: string;
  host: string;
  port: number;
  user: string;
  auth: AuthKind;
  hostKey?: string;
  role: string;
  name: string;
  nodeLink?: string;
  noticeHash?: string;
  config: string;
  notice: string;
  created: number;
  lastUsed: number;
}
export interface RunResult {
  code: number;
  text: string;
}
export interface LogLine {
  run: string;
  stream: 'out' | 'err';
  text: string;
}
/** A hoi-node archive on this computer (hoi-node_<version>_<target>.tar.gz), found or picked. */
export interface NodeFile {
  path: string;
  name: string;
  version: string;
  target: string;
  bytes: number;
}
export interface Uploaded {
  name: string;
  path: string;
  sha256: string;
  bytes: number;
}
export interface UploadProgress {
  run: string;
  sent: number;
  total: number;
}
export interface Platform {
  os: string;
  arch: string;
  user: string;
  home: string;
}

export interface Bridge {
  profiles(): Promise<Profile[]>;
  saveProfile(profile: Profile): Promise<Profile[]>;
  removeProfile(id: string): Promise<Profile[]>;
  probe(target: Target): Promise<HostKey>;
  connect(target: Target, auth: Auth, hostKey: string): Promise<string>;
  run(session: string, run: string, command: string, pty: boolean, answer?: string): Promise<RunResult>;
  read(session: string, command: string): Promise<RunResult>;
  upload(session: string, path: string, text: string, mode: string): Promise<void>;
  /** Uploads a hoi-node archive from this computer into `dir` on the server and checks its SHA-256 there. */
  uploadNode(session: string, run: string, path: string, dir: string): Promise<Uploaded>;
  /** hoi-node archives beside the app, in ../server (builds/), or in a development build the checkout's builds/. */
  nodeFiles(): Promise<NodeFile[]>;
  pickNode(): Promise<NodeFile | null>;
  disconnect(session: string): Promise<void>;
  localRun(run: string, program: string, args: string[], stdin?: string): Promise<RunResult>;
  writeLocal(name: string, text: string): Promise<string>;
  platform(): Promise<Platform>;
  pickFile(): Promise<string | null>;
  saveFile(name: string, text: string): Promise<boolean>;
  openUrl(url: string): Promise<void>;
  onLog(fn: (line: LogLine) => void): Promise<() => void>;
  onUpload(fn: (progress: UploadProgress) => void): Promise<() => void>;
}

export const inTauri = () => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

async function tauriBridge(): Promise<Bridge> {
  const [{ invoke }, { listen }] = await Promise.all([import('@tauri-apps/api/core'), import('@tauri-apps/api/event')]);
  return {
    profiles: () => invoke('setup_profiles'),
    saveProfile: (profile) => invoke('setup_save_profile', { profile }),
    removeProfile: (id) => invoke('setup_remove_profile', { id }),
    probe: (target) => invoke('setup_probe', { target }),
    connect: (target, auth, hostKey) => invoke('setup_connect', { target, auth, hostKey }),
    run: (session, run, command, pty, answer) => invoke('setup_run', { session, run, command, pty, answer: answer ?? null }),
    read: (session, command) => invoke('setup_read', { session, command }),
    upload: (session, path, text, mode) => invoke('setup_upload', { session, path, text, mode }),
    uploadNode: (session, run, path, dir) => invoke('setup_upload_node', { session, run, path, dir }),
    nodeFiles: () => invoke('setup_node_files'),
    pickNode: () => invoke('setup_pick_node'),
    disconnect: (session) => invoke('setup_disconnect', { session }),
    localRun: (run, program, args, stdin) => invoke('setup_local_run', { run, program, args, stdin: stdin ?? null }),
    writeLocal: (name, text) => invoke('setup_write_local', { name, text }),
    platform: () => invoke('setup_platform'),
    pickFile: () => invoke('setup_pick_file'),
    saveFile: (name, text) => invoke('setup_save_file', { name, text }),
    openUrl: (url) => invoke('setup_open_url', { url }),
    onLog: (fn) => listen<LogLine>('setup://log', (e) => fn(e.payload)),
    onUpload: (fn) => listen<UploadProgress>('setup://upload', (e) => fn(e.payload)),
  };
}

export async function getBridge(): Promise<Bridge> {
  if (inTauri()) return tauriBridge();
  const { mockBridge } = await import('./mock-bridge');
  return mockBridge();
}
