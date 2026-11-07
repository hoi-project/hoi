/**
 * Text blocks the wiki renders into static HTML at build time (no script needed to read them).
 *
 * Sequence diagrams:
 *   <div class="wk-seq" data-title="Opening a node link">
 *   lanes: Browser | Home node | Public node
 *   -- Connect --                                  a phase label
 *   Browser -> Home node: WebSocket upgrade         a request
 *   Home node --> Browser: <code>hello</code>       a reply (dashed)
 *   Browser => Home node @coral: records            bulk data, in another colour
 *   Home node: checks every record                  a note on one lane
 *   Browser .. Public node: no direct connection    a note spanning lanes
 *   </div>
 *
 * Byte layouts (one box per line, a blank line starts a new row):
 *   <div class="wk-bytes" data-title="A stamp">
 *   id | 32 B | sun | 4 | BLAKE3 of the canonical record
 *   nonce | 8 B | coral | 1 | what the miner varies
 *   </div>
 *
 * Labels may contain inline HTML (<code>, <b>). Colours: sun coral peach mint sky lilac cobalt mute.
 */
const COLORS = new Set(['sun', 'coral', 'peach', 'mint', 'sky', 'lilac', 'cobalt', 'mute']);
const strip = (s: string) => s.replace(/<[^>]+>/g, '').replace(/&amp;/g, '&').trim();
const attr = (s: string) => s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;');

function lines(body: string): string[] {
  return body
    .replace(/-&gt;/g, '->')
    .replace(/=&gt;/g, '=>')
    .split('\n')
    .map((l) => l.trim());
}

export function renderSeq(attrs: string, body: string, where: string): string {
  const title = /data-title="([^"]*)"/.exec(attrs)?.[1] ?? '';
  const ls = lines(body).filter(Boolean);
  const head = ls.shift() ?? '';
  if (!/^lanes:/i.test(head)) throw new Error(`[hoi-wiki] ${where}: a wk-seq block must start with "lanes: A | B"`);
  const lanes = head.replace(/^lanes:/i, '').split('|').map((s) => s.trim()).filter(Boolean);
  if (lanes.length < 2 || lanes.length > 5) throw new Error(`[hoi-wiki] ${where}: wk-seq needs 2–5 lanes`);
  const lane = (name: string) => {
    const i = lanes.findIndex((l) => l.toLowerCase() === strip(name).toLowerCase());
    if (i < 0) throw new Error(`[hoi-wiki] ${where}: unknown lane "${name}" (lanes: ${lanes.join(', ')})`);
    return i + 1;
  };
  const rows: string[] = [];
  for (const l of ls) {
    let m: RegExpExecArray | null;
    if ((m = /^--\s*(.+?)\s*--$/.exec(l))) {
      rows.push(`<div class="wk-seq-row phase" role="listitem"><span class="wk-seq-a">${m[1]}</span></div>`);
    } else if ((m = /^(.+?)\s*(-->|->|=>)\s*(.+?)\s*(?:@(\w+))?\s*:\s*(.*)$/.exec(l)) && !m[1].includes(':')) {
      const a = lane(m[1]);
      const b = lane(m[3]);
      if (a === b) throw new Error(`[hoi-wiki] ${where}: "${l}" goes from a lane to itself; write a note ("${m[1]}: …")`);
      const color = m[4] && COLORS.has(m[4]) ? ` c-${m[4]}` : '';
      const kind = m[2] === '-->' ? ' reply' : m[2] === '=>' ? ' bulk' : '';
      const cls = `msg s${Math.min(a, b)} e${Math.max(a, b)}${a > b ? ' back' : ''}${kind}${color}`;
      rows.push(`<div class="wk-seq-row ${cls}" role="listitem"><span class="wk-seq-a"><span class="wk-seq-ft">${lanes[a - 1]} → ${lanes[b - 1]}</span><span class="sr-only">${lanes[a - 1]} to ${lanes[b - 1]}: </span>${m[5]}</span></div>`);
    } else if ((m = /^(.+?)\s*\.\.\s*(.+?)\s*:\s*(.*)$/.exec(l))) {
      lane(m[1]);
      lane(m[2]);
      rows.push(`<div class="wk-seq-row note span" role="listitem"><span class="wk-seq-a"><span class="wk-seq-ft">${strip(m[1])} … ${strip(m[2])}</span>${m[3]}</span></div>`);
    } else if ((m = /^([^:]+?)\s*:\s*(.*)$/.exec(l))) {
      const a = lane(m[1]);
      rows.push(`<div class="wk-seq-row note n${a}" role="listitem"><span class="wk-seq-a"><span class="wk-seq-ft">${lanes[a - 1]}</span><span class="sr-only">${lanes[a - 1]}: </span>${m[2]}</span></div>`);
    } else {
      throw new Error(`[hoi-wiki] ${where}: can't read sequence line "${l}"`);
    }
  }
  return (
    `<figure class="wk-seq lanes-${lanes.length}">` +
    (title ? `<figcaption>${title}</figcaption>` : '') +
    `<div class="wk-seq-in"><div class="wk-seq-head" aria-hidden="true">${lanes.map((n, i) => `<span class="h${i + 1}">${n}</span>`).join('')}</div>` +
    `<div class="wk-seq-list" role="list" aria-label="${attr(strip(title) || 'Sequence')}">${rows.join('')}</div></div></figure>`
  );
}

export function renderBytes(attrs: string, body: string, where: string): string {
  const title = /data-title="([^"]*)"/.exec(attrs)?.[1] ?? '';
  const groups: string[][] = [[]];
  for (const l of lines(body)) {
    if (!l) { if (groups[groups.length - 1].length) groups.push([]); continue; }
    const [name, size = '', color = 'sun', width = '1', ...desc] = l.split('|').map((s) => s.trim());
    if (!COLORS.has(color)) throw new Error(`[hoi-wiki] ${where}: wk-bytes colour "${color}"`);
    if (!/^([1-9]|1[0-2])$/.test(width)) throw new Error(`[hoi-wiki] ${where}: wk-bytes width "${width}" (1–12)`);
    groups[groups.length - 1].push(
      `<span class="c-${color} w${width}"><b>${name}</b>${size ? `<i>${size}</i>` : ''}${desc.length ? `<em>${desc.join('|')}</em>` : ''}</span>`,
    );
  }
  return (
    `<figure class="wk-bytes">` +
    (title ? `<figcaption>${title}</figcaption>` : '') +
    groups.filter((g) => g.length).map((g) => `<div class="wk-bytes-row">${g.join('')}</div>`).join('') +
    `</figure>`
  );
}

/** <div class="wk-code" data-lang="rust" data-title="path"> gets its header (title, language, copy). */
export function renderCode(attrs: string): string {
  const title = /data-title="([^"]*)"/.exec(attrs)?.[1] ?? '';
  const lang = /data-lang="([^"]*)"/.exec(attrs)?.[1] ?? '';
  return (
    `<div class="wk-code"${attrs}><div class="wk-code-h"><span class="dots" aria-hidden="true"><i></i><i></i><i></i></span>` +
    `<span class="t">${title}</span>${lang ? `<span class="lang">${lang}</span>` : ''}` +
    `<button type="button" class="wk-copy js-only" aria-label="Copy${title ? ' ' + attr(strip(title)) : ' code'}">Copy</button></div>`
  );
}

export function renderBlocks(html: string, where: string): string {
  return html
    .replace(/<div class="wk-code"([^>]*)>(?!<div class="wk-code-h")/g, (_m, attrs: string) => renderCode(attrs))
    .replace(/<div class="wk-seq"([^>]*)>([\s\S]*?)<\/div>/g, (_m, attrs: string, body: string) => renderSeq(attrs, body, where))
    .replace(/<div class="wk-bytes"([^>]*)>([\s\S]*?)<\/div>/g, (_m, attrs: string, body: string) => renderBytes(attrs, body, where));
}
