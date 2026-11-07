// Download page: highlight the visitor's OS, then fill each row from the same-origin, signed
// release list (/releases/manifest.json). Without scripts the rows link to the list and the repo.
import { $, $$, wireCopyButtons } from '../common';
import { detectOS, OS_LABEL } from '../platform';
import { fetchVerifiedManifest } from '../../../../scripts/release-verify.mjs';
declare const __HOI_PUBLICATION__: import('../../../../scripts/publication.mjs').Publication;

interface ReleaseFile {
  platform: string;
  arch: string;
  kind: string;
  /** `setup` for the node setup app (apps/setup); absent for the hoi app and hoi-node. */
  product?: string;
  name: string;
  url: string;
  size: number;
  sha256: string;
}
interface Release {
  version: string;
  date: string;
  notes?: string;
  files: ReleaseFile[];
}
interface Manifest {
  schema: 1;
  latest: string | null;
  signature?: string;
  releases: Release[];
}

const ARCH: Record<string, string> = { universal: 'Apple silicon + Intel', x86_64: 'x86-64', aarch64: 'ARM64' };
const PLATFORM: Record<string, string> = { macos: 'macOS', windows: 'Windows', linux: 'Linux' };
/** Which files a row lists: the setup app's by product, every other row's by platform. */
const forRow = (row: HTMLElement, f: ReleaseFile) =>
  row.dataset.platform === 'setup' ? f.product === 'setup' : f.platform === row.dataset.platform && f.product !== 'setup';
const ALWAYS = ['macos', 'windows', 'linux']; // rows that say so when a release lacks them

/* ---------- which device is this? ---------- */
const os = detectOS();
const mine = os ? $(`.dl-row#${os}`) : null;
if (os && mine) {
  mine.classList.add('is-you');
  const tag = $('.dl-you', mine);
  if (tag) {
    tag.textContent = `looks like you're on ${OS_LABEL[os]}`;
    tag.hidden = false;
  }
  // Lead with it, unless the visitor came for a specific row (#windows).
  const list = mine.parentElement;
  if (list && !location.hash) list.prepend(mine);
}

/* ---------- the release list ---------- */
const el = <K extends keyof HTMLElementTagNameMap>(tag: K, props: Partial<HTMLElementTagNameMap[K]> = {}, ...kids: (Node | string)[]) => {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...kids);
  return e;
};
const link = (href: string, text: string) => el('a', { href, textContent: text });

const isStr = (v: unknown): v is string => typeof v === 'string' && v.length > 0;
function safeUrl(raw: string): string | null {
  try {
    const u = new URL(raw, location.href);
    return u.protocol === 'https:' || u.origin === location.origin ? u.href : null;
  } catch {
    return null;
  }
}
function validFile(f: unknown): f is ReleaseFile {
  const x = f as Partial<ReleaseFile> | null;
  return !!x && isStr(x.platform) && isStr(x.arch) && isStr(x.kind) && isStr(x.name) && isStr(x.url) &&
    typeof x.size === 'number' && x.size >= 0 && isStr(x.sha256) && /^[0-9a-f]{64}$/i.test(x.sha256) && safeUrl(x.url) !== null;
}

function size(bytes: number): string {
  if (bytes >= 1e9) return (bytes / 1e9).toFixed(2) + ' GB';
  if (bytes >= 1e6) return (bytes / 1e6).toFixed(1) + ' MB';
  return Math.max(1, Math.round(bytes / 1e3)) + ' KB';
}
function day(iso: string): string {
  const d = new Date(iso + 'T00:00:00Z');
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString('en-GB', { day: 'numeric', month: 'long', year: 'numeric', timeZone: 'UTC' });
}

function fileRow(f: ReleaseFile): HTMLLIElement {
  const download = el('a', { className: 'pill pill-ink pill-sm', href: safeUrl(f.url) ?? '#', textContent: `Download .${f.kind}` });
  download.setAttribute('aria-label', `Download ${f.name}`);
  const copy = el('button', { type: 'button', className: 'copy-btn', textContent: 'Copy' });
  copy.dataset.copy = '';
  copy.dataset.copyText = f.sha256.toLowerCase();
  copy.setAttribute('aria-label', `Copy SHA-256 of ${f.name}`);
  return el('li', { className: 'file' },
    el('div', { className: 'file-top' },
      download,
      el('span', { className: 'file-meta' },
        el('span', { className: 'mono', textContent: f.name }),
        el('span', { textContent: size(f.size) }),
        ...(f.product === 'setup' ? [el('span', { className: 'tag t-soft', textContent: PLATFORM[f.platform] ?? f.platform })] : []),
        el('span', { className: 'tag t-soft', textContent: ARCH[f.arch] ?? f.arch }),
      ),
    ),
    el('div', { className: 'sha' },
      el('span', { className: 'sha-l', textContent: 'SHA-256' }),
      el('code', { textContent: f.sha256.toLowerCase() }),
      copy,
    ),
  );
}

function showRelease(m: Manifest, r: Release, status: HTMLElement): void {
  const bits: (Node | string)[] = [`Released ${day(r.date)}`];
  const notes = r.notes ? safeUrl(r.notes) : null;
  if (notes) bits.push(' · ', link(notes, 'release notes'));
  bits.push(' · ', link('/releases/manifest.json', 'manifest.json'));
  if (isStr(m.signature)) bits.push(' and its ', link(`/releases/${encodeURIComponent(m.signature)}`, 'signature'));
  bits.push(' · ', link('/verify/', 'how to check a download'));
  status.replaceChildren(
    el('span', { className: 'eyebrow', textContent: 'latest · signed' }),
    el('h2', { textContent: `hoi ${r.version}` }),
    el('p', { className: 'muted' }, ...bits),
  );

  const files = (Array.isArray(r.files) ? r.files : []).filter(validFile);
  $$('.dl-row').forEach((row) => {
    const slot = $('.files-slot', row);
    if (!slot) return;
    const mineFiles = files.filter((f) => forRow(row, f));
    if (mineFiles.length) {
      slot.replaceChildren(el('ul', { className: 'files' }, ...mineFiles.map(fileRow)));
    } else if (ALWAYS.includes(row.dataset.platform ?? '')) {
      slot.replaceChildren(el('p', { className: 'muted', textContent: `No ${$('h3', row)?.textContent?.trim()} build in hoi ${r.version}.` }));
    } else {
      slot.replaceChildren();
    }
  });
  wireCopyButtons(document);
}

function showEmpty(status: HTMLElement): void {
  const tpl = $<HTMLTemplateElement>('#tpl-empty');
  status.replaceChildren(tpl ? tpl.content.cloneNode(true) : el('h2', { textContent: 'No signed release yet.' }));
  // Bring the build commands up here, where they're needed now.
  const build = $('#build');
  const codes = build ? $('.codes', build) : null;
  if (build && codes) {
    status.appendChild(codes);
    build.hidden = true;
  }
  $$('.files-slot').forEach((slot) => {
    slot.replaceChildren(slot.hasAttribute('data-optional') ? '' : el('p', { className: 'muted', textContent: 'Waiting for the first signed release.' }));
  });
}

async function load(): Promise<void> {
  const status = $('#release');
  if (!status) return;
  try {
    if (!__HOI_PUBLICATION__.manifestKey || !__HOI_PUBLICATION__.downloads) { showEmpty(status); return; }
    const m = await fetchVerifiedManifest(__HOI_PUBLICATION__);
    const r = m.latest ? m.releases.find((x) => x && x.version === m.latest) : undefined;
    if (r && isStr(r.version)) showRelease(m, r, status);
    else showEmpty(status);
  } catch {
    status.replaceChildren(el('h2', { textContent: "The release list could not be verified." }), el('p', { textContent: 'No downloads are offered. Try again when connected, or follow the local verification instructions.' }), link('/verify/', 'Verify a download'));
  }
}

void load();
