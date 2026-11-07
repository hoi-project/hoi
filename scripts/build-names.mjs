// What scripts/build-all.mjs makes and what each file is called, in one place, so a folder of
// builds says at a glance which tool a file belongs to. Every name follows
//   <tool>_<version>_<os>-<arch>[-setup|-portable].<ext>
// except the hoi-node archive, which keeps the release name (hoi-node_<version>_<rust target>.tar.gz)
// that install.sh --binary, the node setup app and the release list already know.

/** The parts in the order they are built; --only and --skip take these names. */
export const PARTS = ['server', 'node-setup', 'app', 'website'];

/** Each part's folder, its name for people, and what it is for (the generated README says this). */
export const TOOLS = {
  app: {
    folder: 'app',
    slug: 'hoi-desktop',
    title: 'hoi',
    app: 'desktop',
    pkg: '@hoi/desktop',
    about: 'The hoi app for your computer. It is also your home node: your phone and your browser connect to it.',
  },
  'node-setup': {
    folder: 'node-setup',
    slug: 'hoi-node-setup',
    title: 'hoi Node Setup',
    app: 'setup',
    pkg: '@hoi/setup',
    binary: 'hoi-node-setup',
    about: 'A wizard that installs and manages a hoi node on your server (a VPS) over SSH, or on this computer. ' +
      'It uploads the hoi-node from ../server/ that it finds on its own, so no published release is needed.',
  },
  server: {
    folder: 'server',
    slug: 'hoi-server',
    title: 'hoi Server',
    about: 'The server itself: hoi-node with its service installer, for 64-bit Linux with systemd ' +
      '(Debian 12, Ubuntu 24.04 or newer). One bundle installs a public node, a public node with a registry, ' +
      'a home node (a Raspberry Pi: the arm64 bundle) or a private node.',
  },
  website: {
    folder: 'website',
    slug: 'hoi-website',
    title: 'hoi website',
    about: 'The hoi.sh website with the browser app in /app, as static files for any web host.',
  },
};

/** Rust targets of the Linux servers, by the arch names used in file names. */
export const LINUX_TARGETS = { x64: 'x86_64-unknown-linux-gnu', arm64: 'aarch64-unknown-linux-gnu' };

/** This computer as file names say it: windows, macos or linux, and x64 or arm64. */
export function hostOs(platform = process.platform) {
  return { win32: 'windows', darwin: 'macos' }[platform] ?? 'linux';
}
export function hostArch(arch = process.arch) {
  return arch === 'arm64' ? 'arm64' : arch === 'ia32' ? 'x86' : 'x64';
}

/** The architecture a Tauri bundle name carries (x64, aarch64, amd64, universal, …) in file-name form. */
export function archIn(name) {
  if (/universal/i.test(name)) return 'universal';
  if (/aarch64|arm64/i.test(name)) return 'arm64';
  if (/(^|[_.-])(x86(?!_64)|i686|i386)([_.-]|$)/i.test(name)) return 'x86';
  return 'x64';
}

const BUNDLE_KINDS = [
  { test: /-setup\.exe$/i, os: 'windows', suffix: '-setup.exe' },
  { test: /\.exe$/i, os: 'windows', suffix: '.exe' },
  { test: /\.msi$/i, os: 'windows', suffix: '.msi' },
  { test: /\.dmg$/i, os: 'macos', suffix: '.dmg' },
  { test: /\.AppImage$/, os: 'linux', suffix: '.AppImage' },
  { test: /\.deb$/i, os: 'linux', suffix: '.deb' },
  { test: /\.rpm$/i, os: 'linux', suffix: '.rpm' },
];

/**
 * The name a Tauri bundle gets in builds/: `hoi Node Setup_1.0.0_x64-setup.exe` becomes
 * `hoi-node-setup_1.0.0_windows-x64-setup.exe`, `hoi_1.0.0_amd64.deb` `hoi-desktop_1.0.0_linux-x64.deb`.
 * Null for a file that is not a bundle people download.
 */
export function bundleName(slug, version, file) {
  const kind = BUNDLE_KINDS.find((k) => k.test.test(file));
  if (!kind) return null;
  return `${slug}_${version}_${kind.os}-${archIn(file)}${kind.suffix}`;
}

/** The app as one file to run without installing: the bare .exe, or the .app zipped on macOS. */
export function portableName(slug, version, os, arch) {
  return `${slug}_${version}_${os}-${arch}-portable.${os === 'windows' ? 'exe' : 'zip'}`;
}

export const serverName = (version, arch) => `hoi-server_${version}_linux-${arch}.tar.gz`;
export const nodeName = (version, target) => `hoi-node_${version}_${target}.tar.gz`;
export const websiteName = (version) => `hoi-website_${version}.tar.gz`;

/** One line for the README about a file, from its name alone. */
export function describe(name) {
  const os = { windows: 'Windows', macos: 'macOS', linux: 'Linux' };
  let m = /^hoi-node_[^_]+_(x86_64|aarch64)-unknown-linux-gnu\.tar\.gz$/.exec(name);
  if (m) {
    return `hoi-node alone for ${m[1] === 'aarch64' ? 'ARM64' : 'x64'} Linux: the program hoi Node Setup uploads to the server, ` +
      'and what `install.sh --binary` takes.';
  }
  m = /^hoi-server_[^_]+_linux-(x64|arm64)\.tar\.gz$/.exec(name);
  if (m) {
    const folder = `hoi-server-linux-${m[1]}`;
    return `Server bundle for ${m[1] === 'arm64' ? 'ARM64 (a Raspberry Pi, an ARM VPS)' : 'x64'} Linux. ` +
      `On the server: \`tar -xzf ${name} && bash ${folder}/install.sh\`. ` +
      `From your computer over SSH: extract it and run \`node ${folder}/deploy.mjs user@host\`.`;
  }
  if (/^hoi-website_/.test(name)) return 'The website and the browser app (in /app) as static files; serve the folder from any web host.';
  m = /_(windows|macos|linux)-(x64|arm64|x86|universal)(-setup|-portable)?\.(exe|msi|dmg|AppImage|deb|rpm|zip)$/.exec(name);
  if (!m) return '';
  const [, sys, arch, variant, ext] = m;
  const on = `${os[sys]} ${arch}`;
  if (variant === '-portable') {
    return sys === 'windows'
      ? `Portable app for ${on}: run it, nothing is installed. Needs WebView2, which Windows 10 and 11 have.`
      : `Portable app for ${on}: unzip and open the .app, nothing is installed. Not notarised: Control-click, Open.`;
  }
  return {
    exe: `Installer for ${on}, for your account (no administrator needed).`,
    msi: `Windows Installer package for ${on}, for managed installs.`,
    dmg: `Disk image for ${on}: drag the app to Applications. Not notarised: Control-click, Open the first time.`,
    AppImage: `AppImage for ${on}: \`chmod +x\` and run it, nothing is installed.`,
    deb: `Package for Debian and Ubuntu (${arch}): \`sudo apt install ./${name}\`.`,
    rpm: `Package for Fedora and openSUSE (${arch}).`,
  }[ext];
}

/** "12.3 MB" */
export function size(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB'];
  let n = bytes / 1024;
  let i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return `${n.toFixed(n < 10 ? 1 : 0)} ${units[i]}`;
}
