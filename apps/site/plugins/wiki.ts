import { readFileSync, existsSync } from 'node:fs';
import { createHash } from 'node:crypto';
import type { Plugin } from 'vite';
import { renderBlocks } from './wiki-blocks.ts';
import type { Corpus, Passage } from '../src/wiki/assistant-context.ts';

/**
 * The wiki's chrome, resolved at build time so every page works without scripts:
 *
 *   <!-- @wiki-open -->    the layout, the sidebar (current page marked) and the phone menu
 *   <!-- @wiki-crumbs -->  "Wiki / <group>" above the title
 *   <!-- @wiki-close -->   previous/next, the end of the layout and "On this page" from the
 *                          article's <section class="wk-sec" id> + <h2>, and <h3 id> headings
 *   <!-- @sbom -->         the dependency tables (plugins/sbom.ts)
 *
 * `virtual:wiki-index` is the search index (titles, headings and the first words of every
 * section), bundled into the wiki script: search needs no request.
 * Must be listed after partials(): it runs on the already-expanded page.
 */
export interface WikiPage { slug: string; title: string; summary: string; group: string; groupTitle: string; color: string; href: string; file: string }
interface NavFile { groups: { id: string; title: string; color: string; pages: { slug: string; title: string; summary: string }[] }[] }

export const WIKI_COLORS = ['sun', 'coral', 'peach', 'mint', 'sky', 'lilac', 'cobalt'];

export function loadWiki(root: string): WikiPage[] {
  const nav = JSON.parse(readFileSync(root + 'wiki/nav.json', 'utf8')) as NavFile;
  const out: WikiPage[] = [];
  const seen = new Set<string>();
  for (const g of nav.groups) {
    if (!WIKI_COLORS.includes(g.color)) throw new Error(`[hoi-wiki] group ${g.id}: unknown color ${g.color}`);
    for (const p of g.pages) {
      if (seen.has(p.slug)) throw new Error(`[hoi-wiki] page ${p.slug} listed twice`);
      if (!/^[a-z0-9-]*$/.test(p.slug)) throw new Error(`[hoi-wiki] bad slug ${p.slug}`);
      seen.add(p.slug);
      out.push({
        ...p,
        group: g.id,
        groupTitle: g.title,
        color: g.color,
        href: p.slug ? `/wiki/${p.slug}/` : '/wiki/',
        file: p.slug ? `wiki/${p.slug}/index.html` : 'wiki/index.html',
      });
    }
  }
  return out;
}

const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

const ENTITIES: Record<string, string> = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ', rarr: '→', larr: '←', harr: '↔', mdash: '—', ndash: '–', hellip: '…', middot: '·', times: '×', le: '≤', ge: '≥', ne: '≠', para: '¶', sect: '§', thinsp: ' ' };
export function textOf(html: string): string {
  return html
    .replace(/<(script|style|svg)\b[\s\S]*?<\/\1>/gi, ' ')
    .replace(/<[^>]+>/g, ' ')
    .replace(/&(#x[0-9a-f]+|#\d+|\w+);/gi, (m, e: string) => {
      if (e[0] === '#') return String.fromCodePoint(e[1] === 'x' || e[1] === 'X' ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10));
      return ENTITIES[e] ?? m;
    })
    .replace(/\s+/g, ' ')
    .trim();
}

interface Heading { level: 2 | 3; id: string; text: string; start: number }
const HEADINGS = /<section\b[^>]*\bclass="wk-sec[^"]*"[^>]*\bid="([^"]+)"[^>]*>\s*<h2\b[^>]*>([\s\S]*?)<\/h2>|<h3\b[^>]*\bid="([^"]+)"[^>]*>([\s\S]*?)<\/h3>/g;
export function headingsOf(html: string): Heading[] {
  const out: Heading[] = [];
  for (const m of html.matchAll(HEADINGS)) {
    if (m[1]) out.push({ level: 2, id: m[1], text: textOf(m[2]), start: m.index ?? 0 });
    else out.push({ level: 3, id: m[3], text: textOf(m[4]), start: m.index ?? 0 });
  }
  return out;
}

/** Full, bounded passages for the opt-in explanation helper. Never part of ordinary search. */
export function passageCorpus(pages: WikiPage[], read: (p: WikiPage) => string): Corpus {
  const passages: Passage[] = [];
  for (const page of pages) {
    const html = read(page);
    const article = html.slice(html.indexOf('<article'), html.indexOf('</article>'));
    const headings = headingsOf(article);
    let parent = '';
    headings.forEach((h, i) => {
      const url = page.href + '#' + h.id;
      if (h.level === 2) parent = url;
      const body = article.slice(h.start, headings[i + 1]?.start ?? article.length);
      const links = [...new Set([...body.matchAll(/href="([^"]+)"/g)].map((m) => m[1])
        .map((href) => href.startsWith('#') ? page.href + href : href)
        .filter((href) => /^\/wiki\/(?:[a-z0-9-]+\/)?(?:#[a-zA-Z0-9_-]+)?$/.test(href)))];
      // Keep paragraphs, code lines and table rows intelligible after stripping markup.
      const lines = body.replace(/<\/(?:p|li|dd|tr|pre|h2|h3)>/g, '$&\n')
        .replace(/<\/t[dh]>/g, '$& | ').split('\n').map(textOf).filter(Boolean);
      let chunk = '';
      let part = 0;
      const flush = () => {
        if (chunk.trim()) passages.push({ id: url + ':' + part++, url, parent, title: h.text, page: page.title, text: chunk.trim(), links });
        chunk = '';
      };
      for (const line of lines) {
        // Oversized prose/code rows are split at whitespace, without discarding their tail.
        for (const word of line.split(/\s+/)) {
          if (chunk.length + word.length + 1 > 2600) flush();
          chunk += (chunk && !chunk.endsWith('\n') ? ' ' : '') + word;
        }
        chunk += '\n';
      }
      flush();
    });
  }
  const version = createHash('sha256').update(JSON.stringify(passages)).digest('hex').slice(0, 16);
  return { version, passages };
}

function sidebar(pages: WikiPage[], current: WikiPage | undefined, idPrefix: string): string {
  const groups: { id: string; title: string; color: string; pages: WikiPage[] }[] = [];
  for (const p of pages) {
    let g = groups.find((x) => x.id === p.group);
    if (!g) groups.push((g = { id: p.group, title: p.groupTitle, color: p.color, pages: [] }));
    g.pages.push(p);
  }
  return (
    `<nav class="wk-nav" aria-label="Wiki">` +
    `<div class="wk-search js-only"><button type="button" class="wk-search-btn" data-search-open aria-haspopup="dialog"><span class="wk-search-i" aria-hidden="true"></span><span>Search the wiki</span><kbd aria-hidden="true">/</kbd></button></div>` +
    groups
      .map(
        (g) =>
          `<div class="wk-group g-${g.color}"><p class="wk-group-t" id="${idPrefix}${g.id}"><i aria-hidden="true"></i>${esc(g.title)}</p><ul aria-labelledby="${idPrefix}${g.id}">` +
          g.pages
            .map((p) => `<li><a href="${p.href}"${p === current ? ' aria-current="page"' : ''}>${esc(p.title)}</a></li>`)
            .join('') +
          `</ul></div>`,
      )
      .join('') +
    `</nav>`
  );
}

function toc(html: string): string {
  const article = html.slice(html.indexOf('<article'), html.indexOf('</article>'));
  const hs = headingsOf(article);
  if (!hs.length) return '';
  return (
    `<aside class="wk-toc" aria-label="On this page"><p class="wk-toc-t">On this page</p><ol>` +
    hs.map((h) => `<li class="l${h.level}"><a href="#${esc(h.id)}">${esc(h.text)}</a></li>`).join('') +
    `</ol></aside>`
  );
}

/** Every page but the home page, as cards under their group's name. */
function wikiMap(pages: WikiPage[]): string {
  const groups: { title: string; color: string; pages: WikiPage[] }[] = [];
  for (const p of pages) {
    if (!p.slug) continue;
    let g = groups.find((x) => x.title === p.groupTitle);
    if (!g) groups.push((g = { title: p.groupTitle, color: p.color, pages: [] }));
    g.pages.push(p);
  }
  return (
    `<div class="wk-groups">` +
    groups.map((g) =>
      `<div class="g-${g.color}"><h3>${esc(g.title)}</h3><div class="wk-cards">` +
      g.pages.map((p) => `<a class="wk-card g-${p.color}" href="${p.href}"><i></i><b>${esc(p.title)}</b><span>${esc(p.summary)}</span></a>`).join('') +
      `</div></div>`,
    ).join('') +
    `</div>`
  );
}

export default function wiki(opts: { root: string; sbom: () => string }): Plugin {
  const root = opts.root.endsWith('/') ? opts.root : opts.root + '/';
  const VIRTUAL = 'virtual:wiki-index';
  const RESOLVED = '\0' + VIRTUAL;
  const PASSAGES = 'virtual:wiki-passages';

  return {
    name: 'hoi-wiki',
    resolveId(id) {
      if (id === VIRTUAL) return RESOLVED;
      if (id === PASSAGES) return '\0' + PASSAGES;
    },
    load(id) {
      if (id === '\0' + PASSAGES) {
        this.addWatchFile(root + 'wiki/nav.json');
        const corpus = passageCorpus(loadWiki(root), (page) => {
          this.addWatchFile(root + page.file);
          const html = readFileSync(root + page.file, 'utf8')
            .replace('<!-- @sbom -->', () => opts.sbom()).replace(/<!--[\s\S]*?-->/g, '');
          return renderBlocks(html, page.file);
        });
        return 'export default ' + JSON.stringify(corpus) + ';';
      }
      if (id !== RESOLVED) return;
      const pages = loadWiki(root);
      const index = pages.map((p) => {
        const file = root + p.file;
        if (existsSync(file)) this.addWatchFile(file);
        const html = existsSync(file) ? readFileSync(file, 'utf8').replace(/<!--[\s\S]*?-->/g, '') : '';
        const article = html.includes('<article') ? html.slice(html.indexOf('<article'), html.indexOf('</article>')) : '';
        const lead = /<p class="wk-lead"[^>]*>([\s\S]*?)<\/p>/.exec(article);
        const hs = headingsOf(article);
        const sections = hs.map((h, i) => {
          const end = i + 1 < hs.length ? hs[i + 1].start : article.length;
          const body = textOf(article.slice(h.start, end)).slice(h.text.length).trim();
          return { id: h.id, t: h.text, x: body.slice(0, 220) };
        });
        return { u: p.href, t: p.title, g: p.groupTitle, c: p.color, s: p.summary, l: lead ? textOf(lead[1]).slice(0, 220) : '', h: sections };
      });
      return `export default ${JSON.stringify(index)};`;
    },
    transformIndexHtml: {
      order: 'pre',
      handler(html, ctx) {
        let out = html;
        if (out.includes('<!-- @sbom -->')) out = out.replace('<!-- @sbom -->', () => opts.sbom());
        if (!out.includes('<!-- @wiki-')) return out;
        const pages = loadWiki(root);
        const href = ctx.path.replace(/index\.html$/, '');
        const i = pages.findIndex((p) => p.href === href);
        if (i < 0) throw new Error(`[hoi-wiki] ${ctx.path} uses the wiki layout but is not listed in wiki/nav.json`);
        const page = pages[i];
        const full = out.includes('<!-- @wiki-open full -->');
        const prev = pages[i - 1];
        const next = pages[i + 1];
        const open =
          `<div class="wk${full ? ' full' : ''} g-${page.color}">` +
          `<aside class="wk-side" aria-label="Wiki sections">${sidebar(pages, page, 'wkg-')}</aside>` +
          `<main id="main" class="wk-main" tabindex="-1">` +
          `<details class="wk-menu"><summary><span class="wk-menu-k">Contents</span><span class="wk-menu-v">${esc(page.groupTitle)} · ${esc(page.title)}</span></summary>${sidebar(pages, page, 'wkm-')}</details>`;
        const pager =
          `<nav class="wk-pager" aria-label="Previous and next page">` +
          (prev ? `<a class="wk-pg prev g-${prev.color}" href="${prev.href}" rel="prev"><span>← Previous</span><b>${esc(prev.title)}</b></a>` : '<span></span>') +
          (next ? `<a class="wk-pg next g-${next.color}" href="${next.href}" rel="next"><span>Next →</span><b>${esc(next.title)}</b></a>` : '<span></span>') +
          `</nav>`;
        const crumbs =
          `<nav class="wk-crumbs" aria-label="Breadcrumb"><ol><li><a href="/wiki/">Wiki</a></li><li><span>${esc(page.groupTitle)}</span></li>` +
          (page.slug ? `<li><span aria-current="page">${esc(page.title)}</span></li>` : '') +
          `</ol></nav>`;
        if (out.includes('<!-- @wiki-map -->')) out = out.replace('<!-- @wiki-map -->', () => wikiMap(pages));
        out = renderBlocks(out, ctx.path)
          .replace(/<!-- @wiki-open(?: full)? -->/, () => open)
          .replace('<!-- @wiki-crumbs -->', () => crumbs);
        out = out.replace('<!-- @wiki-close -->', () => `${pager}</main>${full ? '' : toc(out)}</div>`);
        return out;
      },
    },
  };
}
