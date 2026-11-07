// Shared by builds, browser surfaces and the registry. Informational metadata never changes
// the protocol implemented by an installed binary, and never authorizes deleting local data.
const version = (value) => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(value);
const positive = (value) => Number.isSafeInteger(value) && value > 0;
const text = (value, max) => typeof value === 'string' && value.trim().length > 0 && value.length <= max;

export function validateRelease(value) {
  if (!value || typeof value !== 'object' || value.schemaVersion !== 1 || !positive(value.revision)
    || !['preview', 'stable'].includes(value.channel) || !version(value.protocolVersion) || !positive(value.indexVersion) || value.indexVersion > 2_147_483_647
    || !Array.isArray(value.supportedProtocols) || !value.supportedProtocols.length || value.supportedProtocols.length > 16
    || !value.supportedProtocols.every(version) || new Set(value.supportedProtocols).size !== value.supportedProtocols.length
    || !value.supportedProtocols.includes(value.protocolVersion)
    || !value.notice || !text(value.notice.title, 100) || !text(value.notice.message, 1200)) {
    throw new Error('Invalid release configuration. Expected schema 1 with a channel, revision, protocol/index versions, supported protocols and notice.');
  }
  return {
    schemaVersion: 1, revision: value.revision, channel: value.channel,
    protocolVersion: value.protocolVersion, indexVersion: value.indexVersion,
    supportedProtocols: [...value.supportedProtocols],
    notice: { title: value.notice.title, message: value.notice.message },
  };
}

/** A newer operator notice can change words, never this build's compatibility or stability. */
export function noticeForBuild(build, remote) {
  const current = remote && remote.revision >= build.revision ? remote : build;
  const sameBuildFamily = current.protocolVersion === build.protocolVersion && current.indexVersion === build.indexVersion && current.channel === build.channel;
  const notice = sameBuildFamily ? current.notice : build.notice;
  return {
    title: notice.title, message: notice.message,
    build: `${build.channel} · protocol ${build.protocolVersion} · index ${build.indexVersion}`,
    preview: build.channel === 'preview',
    update: sameBuildFamily ? '' : `Operator update: ${current.notice.title}. ${current.notice.message} Current protocol ${current.protocolVersion}, index ${current.indexVersion}. This installed build remains ${build.channel}.`,
  };
}
