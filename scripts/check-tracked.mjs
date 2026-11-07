// What the repository may track, and that its documentation links resolve (part of pnpm check:quick).
//   node scripts/check-tracked.mjs
// Refuses tracked credentials, databases, logs, build output and symlinks, and Markdown links to
// files that are not tracked. Files that only exist locally (untracked, ignored) are not looked at.
import { execFileSync } from 'node:child_process';
import { existsSync, lstatSync, readFileSync, realpathSync } from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
// Build output, dependencies, caches and local tool state: generated, never source.
const generated = new Set(['.git', '.agents', '.codex', '.aws', '.ssh', '.cache', 'node_modules',
  'target', 'dist', 'dist-desktop', 'dist-setup', 'builds', 'work', '.shots', 'coverage',
  '.wrangler', '.state', '.gradle', '.svelte-kit', 'test-results', 'playwright-report']);

/** Whether a repository-relative path may be tracked. */
export function isTrackable(path) {
  const normalized = path.replaceAll('\\', '/');
  if (normalized.includes(':')) return false;
  const parts = normalized.split('/');
  if (parts.some(part => !part || part === '..' || part === '.' || generated.has(part))) return false;
  const name = parts.at(-1);
  // Credentials: environment files (the example is fine), package and SSH credentials, keys.
  if (/^\.env(?:\.|$)/i.test(name) && name !== '.env.example') return false;
  if (['.npmrc', '.netrc', '_netrc', 'id_rsa', 'id_ed25519', 'id_ecdsa'].includes(name)) return false;
  // Logs, local databases and their journals, key and certificate files.
  return !/\.(?:log|db(?:-(?:wal|shm|journal))?|sqlite3?(?:-(?:wal|shm|journal))?|pem|key|p12|pfx)$/i.test(name);
}

/** Every relative Markdown link in the given files points at a tracked file or directory. */
export function checkLinks(base, paths) {
  const tracked = new Set(paths.map(path => path.replaceAll('\\', '/')));
  for (const path of paths.filter(path => path.endsWith('.md') && !path.startsWith('vendor/'))) {
    // Code spans and fenced blocks show Markdown syntax (`[text](target)`); they are not links.
    const source = readFileSync(join(base, path), 'utf8')
      .replace(/^(```|~~~)[^\n]*\n[\s\S]*?^\1[^\n]*$/gm, '')
      .replace(/(`+)[^`\n][\s\S]*?\1/g, '');
    for (const [, link] of source.matchAll(/\[[^\]]*\]\(([^)]+)\)/g)) {
      const target = link.split('#')[0].replace(/^<|>$/g, '');
      if (!target || /^[a-z][a-z\d+.-]*:/i.test(target)) continue;
      const full = resolve(base, dirname(path), target);
      const fromRoot = relative(base, full).replaceAll('\\', '/');
      const found = tracked.has(fromRoot) || [...tracked].some(file => file.startsWith(`${fromRoot}/`));
      if (!existsSync(full) || !found) throw new Error(`${path}: link to a missing or untracked file: ${target}`);
    }
  }
}

/** Every path is trackable, a regular file, and reached without a symlink. */
export function checkFiles(base, paths) {
  const source = realpathSync(base);
  for (const path of paths) {
    if (!isTrackable(path) || isAbsolute(path)) throw new Error(`Refusing a credential, database, log, build output or unsafe path: ${path}`);
    let current = source;
    for (const part of path.replaceAll('\\', '/').split('/')) {
      current = join(current, part);
      if (lstatSync(current).isSymbolicLink()) throw new Error(`Refusing symlink: ${path}`);
    }
    if (!lstatSync(current).isFile()) throw new Error(`Not a regular file: ${path}`);
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const files = [...new Set(execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard', '-z'], { cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 })
    .split('\0'))].filter(path => path && existsSync(join(root, path)));
  checkFiles(root, files);
  checkLinks(root, files);
  console.log(`Tracked files: ${files.length} checked; no credentials, databases, logs, build output or symlinks; documentation links resolve.`);
}
