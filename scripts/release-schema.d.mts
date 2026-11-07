export interface ReleaseConfig {
  schemaVersion: 1;
  revision: number;
  channel: 'preview' | 'stable';
  protocolVersion: string;
  indexVersion: number;
  supportedProtocols: string[];
  notice: { title: string; message: string };
}
export function validateRelease(value: unknown): ReleaseConfig;
export function noticeForBuild(build: ReleaseConfig, remote?: ReleaseConfig | null): {
  title: string; message: string; build: string; preview: boolean; update: string;
};
