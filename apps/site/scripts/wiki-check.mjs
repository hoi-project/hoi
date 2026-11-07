#!/usr/bin/env node
// Checks the wiki's pages without building them:
//   node scripts/wiki-check.mjs [wiki/records/index.html …]   (default: every page in wiki/nav.json)
// Structure (head, layout markers, one h1, a lead), the CSP rules (no style attributes, <style>,
// inline scripts or handlers), template variables, unique ids, balanced tags, tables wrapped for
// scrolling, demo names, and every /wiki/ link and #anchor resolving. Exits 1 on any problem.
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderBlocks } from '../plugins/wiki-blocks.ts';

const site = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const nav = JSON.parse(readFileSync(join(site, 'wiki/nav.json'), 'utf8'));
const pages = nav.groups.flatMap((g) => g.pages.map((p) => ({ ...p, href: p.slug ? `/wiki/${p.slug}/` : '/wiki/', file: join(site, p.slug ? `wiki/${p.slug}/index.html` : 'wiki/index.html') })));
const DEMOS = new Set(['hash', 'stamp', 'pipeline', 'reconcile', 'gossip', 'shamir', 'scatter', 'ladder', 'expiry', 'ratelimit']);
const VARS = new Set(['REPO_URL', 'APP_URL', 'SITE_URL', 'WIKI_PAGES', 'SBOM_PACKAGES']);
const VOID = new Set(['area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'source', 'track', 'wbr', 'path', 'circle', 'rect', 'line', 'polyline', 'polygon', 'ellipse', 'stop', 'use']);
const CHECKED = new Set(['div', 'section', 'article', 'ul', 'ol', 'li', 'table', 'thead', 'tbody', 'tr', 'td', 'th', 'figure', 'figcaption', 'details', 'summary', 'a', 'span', 'code', 'pre', 'p', 'dl', 'dt', 'dd', 'h1', 'h2', 'h3', 'h4', 'svg', 'g', 'text', 'b', 'strong', 'em', 'i', 'small', 'nav', 'aside', 'main', 'button', 'kbd', 'sup', 'sub', 'mark', 'blockquote', 'defs', 'marker', 'tspan']);

const args = process.argv.slice(2).map((a) => resolve(a));
const targets = args.length ? pages.filter((p) => args.includes(p.file)) : pages;
if (args.length && !targets.length) { console.error('none of those files is listed in wiki/nav.json'); process.exit(1); }

const problems = [];
const bad = (p, what) => problems.push(`${relative(site, p.file)}: ${what}`);
const idsOf = new Map();

for (const p of pages) {
  if (!existsSync(p.file)) continue;
  const html = readFileSync(p.file, 'utf8');
  idsOf.set(p.href, new Set([...html.matchAll(/\bid="([^"]+)"/g)].map((m) => m[1])));
}

for (const p of targets) {
  if (!existsSync(p.file)) { bad(p, 'missing (listed in nav.json)'); continue; }
  const raw = readFileSync(p.file, 'utf8');
  const html = raw.replace(/<!--[\s\S]*?-->/g, (c) => (/@(include|wiki-)/.test(c) ? c : ''));
  const need = [
    [/^<!doctype html>\n<html lang="en" data-theme="dark">/i, 'starts with <!doctype html> and <html lang="en" data-theme="dark">'],
    [/<!-- @include head\.html title="[^"]+ · hoi wiki" description="[^"]{20,}" -->/, 'head include with title "… · hoi wiki" and a description'],
    [/<body class="wiki">/, '<body class="wiki">'],
    [/<!-- @include header\.html nav="wiki" -->/, 'header include with nav="wiki"'],
    [/<!-- @wiki-open(?: full)? -->/, '<!-- @wiki-open -->'],
    [/<article class="wk-doc(?: wide)?">/, '<article class="wk-doc">'],
    [/<!-- @wiki-crumbs -->/, '<!-- @wiki-crumbs -->'],
    [/<p class="wk-lead">/, 'a <p class="wk-lead">'],
    [/<!-- @wiki-close -->/, '<!-- @wiki-close -->'],
    [/<!-- @include footer\.html -->/, 'footer include'],
    [/<script type="module" src="\/src\/pages\/(wiki|playground)\.ts"><\/script>/, 'the wiki script'],
  ];
  for (const [re, what] of need) if (!re.test(raw)) bad(p, `needs ${what}`);
  if ((html.match(/<h1\b/g) || []).length !== 1) bad(p, 'exactly one <h1>');
  if (/\sstyle\s*=/i.test(html)) bad(p, 'style attribute (the CSP forbids them)');
  if (/<style\b/i.test(html)) bad(p, '<style> element');
  if ((html.match(/<script\b/gi) || []).length !== 1) bad(p, 'a <script> besides the module script');
  if (/\son[a-z]+\s*=\s*["']/i.test(html)) bad(p, 'inline event handler');
  for (const m of raw.matchAll(/\{\{\s*([\w-]+)\s*\}\}/g)) if (!VARS.has(m[1])) bad(p, `unknown variable {{${m[1]}}}`);
  if (/\{\{(?!\s*(REPO_URL|APP_URL|SITE_URL|WIKI_PAGES|SBOM_PACKAGES)\s*\}\})/.test(raw)) bad(p, 'stray "{{" (write &#123;&#123;)');
  if (/[\w.+-]+@[\w-]+(?:\.[\w-]+)*\.[a-z]{2,}\b/i.test(html.replace(/<script[\s\S]*?<\/script>/g, ''))) bad(p, 'email address');
  // The same rule as scripts/public-copy.test.mjs: public pages carry no deployment-configuration hints.
  const hint = /VITE_HOI_|HOI_REGISTRY_|No registry is configured|Set\s+\w+_URL|this build creates unregistered/i.exec(html.replace(/<script\b[^>]*>[\s\S]*?<\/script>/gi, ''));
  if (hint) bad(p, `deployment-configuration hint "${hint[0]}" (name the setting and link the README instead)`);
  const ids = [...html.matchAll(/\bid="([^"]+)"/g)].map((m) => m[1]);
  const dup = ids.filter((id, i) => ids.indexOf(id) !== i);
  if (dup.length) bad(p, `duplicate ids: ${[...new Set(dup)].join(', ')}`);
  for (const m of html.matchAll(/<section\b([^>]*)>/g)) {
    if (/class="wk-sec/.test(m[1]) && !/\bid="/.test(m[1])) bad(p, 'a wk-sec section without an id');
  }
  for (const m of html.matchAll(/<table\b/g)) {
    const before = html.slice(Math.max(0, m.index - 300), m.index);
    if (!/<div class="wk-table[^"]*" tabindex="0" role="region" aria-label="[^"]+">\s*$/.test(before)) bad(p, 'a <table> not directly in <div class="wk-table" tabindex="0" role="region" aria-label="…">');
  }
  for (const m of html.matchAll(/data-demo="([^"]+)"/g)) if (!DEMOS.has(m[1])) bad(p, `unknown demo ${m[1]}`);
  // The sequence diagrams and byte layouts, through the build's own renderer.
  try { renderBlocks(html, relative(site, p.file)); } catch (e) { bad(p, e.message.replace(/^\[hoi-wiki\] [^:]+: /, '')); }
  for (const m of html.matchAll(/href="([^"]+)"/g)) {
    const href = m[1].replace(/&amp;/g, '&');
    if (href.startsWith('#')) {
      if (!ids.includes(decodeURIComponent(href.slice(1)))) bad(p, `link to missing #${href.slice(1)}`);
    } else if (href.startsWith('/wiki/')) {
      const [path, hash] = href.split('#');
      const target = pages.find((q) => q.href === path);
      if (!target) bad(p, `link to unknown wiki page ${path}`);
      else if (hash && idsOf.has(path) && !idsOf.get(path).has(hash)) bad(p, `link to missing ${path}#${hash}`);
    } else if (href.startsWith('/') && !/^\/(download|verify|relay|communities|protocol|privacy|app)\/|^\/(#.*)?$/.test(href)) {
      bad(p, `link to unknown site page ${href}`);
    }
  }
  // Balanced tags (outside code, which may hold anything escaped).
  const body = html.slice(html.indexOf('<article'), html.lastIndexOf('</article>') + 10).replace(/<pre\b[\s\S]*?<\/pre>/g, '<pre></pre>');
  const stack = [];
  for (const m of body.matchAll(/<(\/?)([a-zA-Z][\w-]*)\b[^>]*?(\/?)>/g)) {
    const [, close, tagRaw, self] = m;
    const tag = tagRaw.toLowerCase();
    if (self || VOID.has(tag) || !CHECKED.has(tag)) continue;
    if (!close) stack.push(tag);
    else {
      const top = stack.pop();
      if (top !== tag) { bad(p, `unbalanced tags near </${tag}> (open: <${top ?? 'none'}>, at …${body.slice(Math.max(0, m.index - 80), m.index).replace(/\s+/g, ' ')})`); break; }
    }
  }
  if (stack.length) bad(p, `unclosed tags: ${stack.join(', ')}`);
  const text = body.replace(/<[^>]+>/g, ' ');
  if (/(^|[^&\w])<(?![a-z/!])/.test(text)) bad(p, 'an unescaped "<" in text');
}

if (problems.length) {
  console.error(problems.map((x) => '✗ ' + x).join('\n'));
  console.error(`\n${problems.length} problem(s) in ${targets.length} page(s).`);
  process.exit(1);
}
console.log(`✓ ${targets.length} wiki page(s) look right.`);
