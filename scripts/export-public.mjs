// Export a clean tracked working tree without Git history or maintainer-only files.
import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, realpathSync } from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const generated = new Set(['.git', '.private', '.agents', '.codex', '.aws', '.ssh', '.cache', 'node_modules',
  'target', 'dist', 'dist-desktop', 'dist-setup', 'builds', 'work', '.shots', 'coverage',
  '.wrangler', '.state', '.gradle', '.svelte-kit', 'test-results', 'playwright-report', 'public-export']);
const planning = /^(?:docs\/(?:ROADMAP|DECISIONS|DESIGN|TECHNICAL|PROTOCOL|NETWORKING|LOCAL-STORAGE|PRIVATE-RELAY|SECURITY-REVIEW)\.md|docs\/spec\/DECISIONS\.md|design\/canvas\/|prototype\/|core\/crates\/hoi-net\/IROH\.md)/;

export function isPublicPath(path) {
  const normalized = path.replaceAll('\\', '/');
  if (planning.test(normalized) || normalized.includes(':')) return false;
  const parts = normalized.split('/');
  if (parts.some(part => !part || part === '..' || part === '.' || generated.has(part))) return false;
  const name = parts.at(-1);
  if (/^\.env(?:\.|$)/i.test(name) && name !== '.env.example') return false;
  if (['.npmrc', '.netrc', '_netrc', 'id_rsa', 'id_ed25519', 'id_ecdsa'].includes(name)) return false;
  return !/\.(?:log|db(?:-(?:wal|shm|journal))?|sqlite3?(?:-(?:wal|shm|journal))?|pem|key|p12|pfx)$/i.test(name);
}

export function checkPublicLinks(base, paths) {
  const included = new Set(paths.map(path => path.replaceAll('\\', '/')));
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
      const exported = included.has(fromRoot) || [...included].some(path => path.startsWith(`${fromRoot}/`));
      if (!isPublicPath(fromRoot) || !existsSync(full) || !exported) {
        throw new Error(`${path}: link targets missing or private source: ${target}`);
      }
    }
  }
}

export function validatePublicFiles(base, paths) {
  const source = realpathSync(base);
  // Every path is validated before writing anything; symlinks may escape the source tree.
  for (const path of paths) {
    if (!isPublicPath(path) || isAbsolute(path)) throw new Error(`Refusing private or unsafe path: ${path}`);
    let current = source;
    for (const part of path.replaceAll('\\', '/').split('/')) {
      current = join(current, part);
      if (lstatSync(current).isSymbolicLink()) throw new Error(`Refusing symlink: ${path}`);
    }
    if (!lstatSync(current).isFile()) throw new Error(`Not a regular file: ${path}`);
  }
}

export function copySnapshot(base, destination, paths) {
  const source = realpathSync(base);
  const output = resolve(destination);
  if (existsSync(output)) throw new Error('Export destination already exists; choose a new directory.');
  validatePublicFiles(source, paths);
  mkdirSync(output, { recursive: true });
  for (const path of paths) {
    const target = join(output, path);
    mkdirSync(dirname(target), { recursive: true });
    copyFileSync(join(source, path), target);
  }
}

function git(...args) {
  return execFileSync('git', args, { cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 });
}

function main() {
  const args = process.argv.slice(2);
  if (args.length !== 1 || !['--check', '--export'].includes(args[0])) {
    throw new Error('Usage: node scripts/export-public.mjs --check | --export');
  }
  const tracked = git('ls-files', '-z').split('\0').filter(Boolean);
  const files = tracked.filter(isPublicPath);
  const unexpected = tracked.filter(path => existsSync(join(root, path)) && !isPublicPath(path) && !path.startsWith('.private/'));
  if (unexpected.length) throw new Error(`Tracked local/generated/sensitive files: ${unexpected.join(', ')}`);
  if (args[0] === '--check') {
    const current = [...new Set(git('ls-files', '--cached', '--others', '--exclude-standard', '-z').split('\0'))]
      .filter(path => path && existsSync(join(root, path)));
    const publicFiles = current.filter(isPublicPath);
    validatePublicFiles(root, publicFiles);
    checkPublicLinks(root, publicFiles);
    console.log(`Public boundary: ${publicFiles.length} source paths; ${current.length - publicFiles.length} private paths excluded. Public documentation links resolve.`);
    return;
  }
  const dirty = [...git('diff', 'HEAD', '--name-only', '-z').split('\0'),
    ...git('ls-files', '--others', '--exclude-standard', '-z').split('\0')].filter(path => path && isPublicPath(path));
  if (dirty.length) throw new Error('Commit or resolve public-source changes before exporting; the exporter never commits.');
  checkPublicLinks(root, files);
  const destination = resolve(root, 'public-export');
  const within = relative(realpathSync(root), destination);
  if (within.startsWith(`..${sep}`) || isAbsolute(within)) throw new Error('Export must stay inside this workspace.');
  copySnapshot(root, destination, files);
  console.log(`Exported ${files.length} files to ${destination}; no Git history was copied. Review before publishing.`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main();
