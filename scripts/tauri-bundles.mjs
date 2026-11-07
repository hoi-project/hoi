// Which Windows bundles `tauri build` makes for a version. Windows Installer (.msi, made with WiX)
// takes a version as numbers only, and Tauri refuses any other after the whole build has run:
// "optional pre-release identifier in app version must be numeric-only and cannot be greater
// than 65535 for msi target". A preview such as 1.0.0-preview.1 is built with the NSIS setup
// (*-setup.exe) alone, the one install.ps1 prefers; a release such as 1.0.0 gets both.

import { existsSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

const small = (part) => /^\d+$/.test(part) && Number(part) <= 65535;

/** Whether WiX takes this version, checked the way Tauri's bundler checks it. */
export function msiTakesVersion(version) {
  const m = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+([0-9A-Za-z.-]+))?$/.exec(String(version));
  if (!m) return false;
  const [, major, minor, patch, pre, build] = m;
  if (Number(major) > 255 || Number(minor) > 255 || Number(patch) > 65535) return false;
  // Tauri reads build metadata first and, when it is a number, uses it and ignores the rest.
  if (build !== undefined) return small(build);
  return pre === undefined || small(pre);
}

/** The value of `--target`/`-t` in `args`, or null. */
function targetIn(args) {
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--target' || args[i] === '-t') return args[i + 1] ?? null;
    if (args[i].startsWith('--target=')) return args[i].slice('--target='.length);
  }
  return null;
}

/**
 * `args` for the Tauri CLI, with `--bundles nsis` added to a Windows `build` whose version WiX
 * refuses. Bundles chosen on the command line (`--bundles`, `-b`, `--no-bundle`) are left alone.
 * Returns the arguments and, when it narrowed them, a line saying why.
 */
export function windowsBundles(args, version, platform = process.platform) {
  if (args[0] !== 'build' || msiTakesVersion(version)) return { args, note: null };
  if (args.some((a) => a === '-b' || a === '--no-bundle' || a === '--bundles' || a.startsWith('--bundles='))) return { args, note: null };
  const target = targetIn(args);
  if (!(target ? target.includes('windows') : platform === 'win32')) return { args, note: null };
  return {
    args: [...args, '--bundles', 'nsis'],
    note: `tauri build: Windows Installer (.msi) takes numeric versions only, not ${version}; building the NSIS setup (-setup.exe) alone.`,
  };
}

// What `tauri build` leaves for people to download, found the same way by scripts/release-local.mjs
// (a flat release folder) and scripts/build-all.mjs (builds/, a folder per tool).

const DOWNLOADS = /\.(dmg|msi|exe|AppImage|deb|rpm|app\.tar\.gz|AppImage\.tar\.gz)(\.sig)?$/;

/** Files under `dir` (a Tauri target folder) that are bundles people download; .app folders are skipped. */
export function findBundles(dir) {
  const found = [];
  const walk = (d) => {
    for (const name of readdirSync(d)) {
      const p = join(d, name);
      const s = statSync(p);
      if (s.isDirectory() && !name.endsWith('.app')) walk(p);
      else if (s.isFile() && DOWNLOADS.test(name)) found.push(p);
    }
  };
  if (existsSync(dir)) walk(dir);
  return found;
}

/** The node setup app's product name is "hoi Node Setup", so Tauri names its bundles with spaces
 * (and a GitHub release turns them into dots). Release files keep the plain `hoi-node-setup_` start. */
export function releaseBundleName(name) {
  return name.replace(/^hoi[ ._-]node[ ._-]setup(?=_)/i, 'hoi-node-setup');
}
