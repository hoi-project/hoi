#!/usr/bin/env node
// Draws the installers' artwork for both Tauri apps, in hoi.sh's colours and type:
//   apps/<app>/src-tauri/installer/header.bmp          150×57   NSIS, top of the inner pages
//   apps/<app>/src-tauri/installer/sidebar.bmp         164×314  NSIS, welcome and finish pages
//   apps/<app>/src-tauri/installer/dmg-background.png  660×400  macOS disk image window
// tauri.conf.json points at them. They are committed, like the icons, so building an installer
// needs no browser; run this again after changing the drawing or the brand:
//   node scripts/installer-art.mjs          (CHROMIUM_PATH=… for a browser Playwright did not install)
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));

/** The sizes NSIS and Finder expect; tauri.conf.json's dmg windowSize matches DMG. */
export const ART = {
  header: { file: 'header.bmp', width: 150, height: 57 },
  sidebar: { file: 'sidebar.bmp', width: 164, height: 314 },
  dmg: { file: 'dmg-background.png', width: 660, height: 400 },
};
/** Where Finder puts the app and the Applications link in the disk image window (icon centres). */
export const DMG_POSITIONS = { app: { x: 180, y: 170 }, applications: { x: 480, y: 170 } };

const C = { bg: '#FBFAF5', card: '#FFFFFF', ink: '#1A1A1A', mute: '#6A6A63', coral: '#FF6B4A', cobalt: '#2F55E6', sun: '#FFE16A', onDark: '#FBFAF5', cobaltDark: '#8AA0FF' };

export const APPS = {
  desktop: {
    name: 'hoi',
    sub: '',
    sticker: 'the app',
    headline: ['Communities', 'on your devices.'],
    tagline: 'A forum with nobody in the middle.',
    dark: false,
  },
  setup: {
    name: 'hoi',
    sub: 'Node Setup',
    sticker: 'node setup',
    headline: ['Your node,', 'your server.'],
    tagline: 'Set up a hoi node on a server, step by step.',
    dark: true,
  },
};

/** A 24-bit, bottom-up Windows bitmap from RGBA pixels, the kind NSIS shows. */
export function bmp(width, height, rgba) {
  if (rgba.length !== width * height * 4) throw new Error('pixel count does not match the size');
  const row = Math.ceil((width * 3) / 4) * 4;
  const size = 54 + row * height;
  const out = Buffer.alloc(size);
  out.write('BM', 0, 'ascii');
  out.writeUInt32LE(size, 2);
  out.writeUInt32LE(54, 10);
  out.writeUInt32LE(40, 14);
  out.writeInt32LE(width, 18);
  out.writeInt32LE(height, 22);
  out.writeUInt16LE(1, 26);
  out.writeUInt16LE(24, 28);
  out.writeUInt32LE(row * height, 34);
  out.writeInt32LE(2835, 38); // 72 dpi
  out.writeInt32LE(2835, 42);
  for (let y = 0; y < height; y++) {
    const at = 54 + (height - 1 - y) * row;
    for (let x = 0; x < width; x++) {
      const i = (y * width + x) * 4;
      out[at + x * 3] = rgba[i + 2];
      out[at + x * 3 + 1] = rgba[i + 1];
      out[at + x * 3 + 2] = rgba[i];
    }
  }
  return out;
}

/** Width and height from a BMP or PNG header. */
export function imageSize(bytes) {
  if (bytes.subarray(0, 2).toString('ascii') === 'BM') return { width: bytes.readInt32LE(18), height: Math.abs(bytes.readInt32LE(22)), bits: bytes.readUInt16LE(28) };
  if (bytes.readUInt32BE(0) === 0x89504e47) return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) };
  throw new Error('neither a BMP nor a PNG');
}

const mark = (size, line) => `<svg width="${size}" height="${size}" viewBox="0 0 30 30" aria-hidden="true">
  <line x1="8" y1="10" x2="22" y2="9" stroke="${line}" stroke-width="2" stroke-linecap="round"/>
  <line x1="8" y1="10" x2="15" y2="22" stroke="${line}" stroke-width="2" stroke-linecap="round"/>
  <line x1="22" y1="9" x2="15" y2="22" stroke="${line}" stroke-width="2" stroke-linecap="round"/>
  <circle cx="8" cy="10" r="5" fill="${C.coral}"/><circle cx="22" cy="9" r="5" fill="${C.cobalt}"/>
  <circle cx="15" cy="22" r="5" fill="${C.sun}" stroke="${line}" stroke-width="2"/></svg>`;

function page(fonts, width, height, body, css = '') {
  return `<!doctype html><html><head><style>
${fonts}
*{box-sizing:border-box;margin:0}
html,body{width:${width}px;height:${height}px;overflow:hidden}
body{font-family:Gabarito,system-ui,sans-serif;-webkit-font-smoothing:antialiased}
.mono{font-family:'Geist Mono',ui-monospace,monospace}
${css}</style></head><body>${body}</body></html>`;
}

function header(app, fonts) {
  const { width, height } = ART.header;
  return page(fonts, width, height, `
    <div class="h">${mark(36, C.ink)}<div><b>${app.name}</b>${app.sub ? `<span>${app.sub}</span>` : ''}</div></div>`, `
    body{background:${C.card};color:${C.ink}}
    .h{height:100%;display:flex;align-items:center;gap:7px;padding:0 8px}
    b{display:block;font-size:24px;font-weight:800;line-height:1;letter-spacing:-.02em}
    span{display:block;font-size:12px;font-weight:700;color:${C.cobalt};line-height:1.1;margin-top:2px;white-space:nowrap}`);
}

function sidebar(app, fonts) {
  const { width, height } = ART.sidebar;
  const ink = app.dark ? C.onDark : C.ink;
  return page(fonts, width, height, `
    <div class="s">
      <div class="brand">${mark(40, ink)}<b>hoi</b></div>
      <hr>
      <i class="sticker">${app.sticker}</i>
      <h1>${app.headline[0]}<br><em>${app.headline[1]}</em></h1>
      <p>${app.tagline}</p>
      <div class="foot mono">hoi.sh</div>
    </div>`, `
    body{background:${app.dark ? C.ink : C.bg};color:${ink}}
    .s{height:100%;padding:18px 14px 14px;display:flex;flex-direction:column}
    .brand{display:flex;align-items:center;gap:6px}
    .brand b{font-size:34px;font-weight:800;letter-spacing:-.03em;line-height:1}
    hr{border:0;border-top:3px solid ${ink};margin:12px 0 18px}
    .sticker{align-self:flex-start;font-style:normal;font-size:12px;font-weight:800;background:${C.sun};color:${C.ink};
      padding:3px 8px;border:2px solid ${C.ink};border-radius:4px;transform:rotate(-3deg);text-transform:lowercase}
    h1{font-size:23px;line-height:1.08;font-weight:800;letter-spacing:-.02em;margin-top:14px}
    em{font-style:normal;color:${app.dark ? C.cobaltDark : C.cobalt}}
    p{font-size:12.5px;line-height:1.35;margin-top:10px;color:${app.dark ? '#C9C7BE' : C.mute}}
    .foot{margin-top:auto;font-size:11px;color:${app.dark ? '#C9C7BE' : C.mute}}`);
}

function dmg(app, fonts) {
  const { width, height } = ART.dmg;
  const { app: a, applications: b } = DMG_POSITIONS;
  const title = app.sub ? `${app.name} ${app.sub}` : app.name;
  return page(fonts, width, height, `
    <h1>Drag <b>${title}</b> to <em>Applications</em></h1>
    <svg class="arrow" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
      <path d="M ${a.x + 86} ${a.y} C ${a.x + 130} ${a.y - 34}, ${b.x - 130} ${a.y - 34}, ${b.x - 92} ${b.y - 4}"
        fill="none" stroke="${C.ink}" stroke-width="4" stroke-linecap="round" stroke-dasharray="2 10"/>
      <path d="M ${b.x - 106} ${b.y - 18} L ${b.x - 88} ${b.y - 2} L ${b.x - 112} ${b.y + 4}" fill="none" stroke="${C.ink}"
        stroke-width="4" stroke-linecap="round" stroke-linejoin="round"/>
    </svg>
    <div class="foot">${mark(26, C.ink)}<span class="mono">${app.tagline}</span></div>`, `
    body{background:${C.bg};color:${C.ink};position:relative}
    h1{position:absolute;top:34px;left:0;right:0;text-align:center;font-size:26px;font-weight:800;letter-spacing:-.02em}
    h1 b{font-weight:800}
    h1 em{font-style:normal;color:${C.cobalt}}
    .arrow{position:absolute;inset:0}
    .foot{position:absolute;left:24px;right:24px;bottom:20px;display:flex;align-items:center;gap:8px;
      border-top:2px solid ${C.ink};padding-top:12px;font-size:12px;color:${C.mute}}`);
}

function fontFaces() {
  const file = (pkg, name) => readFileSync(join(root, 'packages/tokens/node_modules', pkg, 'files', name)).toString('base64');
  return [
    `@font-face{font-family:'Gabarito';font-weight:400 900;src:url(data:font/woff2;base64,${file('@fontsource-variable/gabarito', 'gabarito-latin-wght-normal.woff2')}) format('woff2')}`,
    `@font-face{font-family:'Geist Mono';font-weight:400;src:url(data:font/woff2;base64,${file('@fontsource/geist-mono', 'geist-mono-latin-400-normal.woff2')}) format('woff2')}`,
  ].join('\n');
}

async function main() {
  const { chromium } = await import('playwright');
  const fonts = fontFaces();
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  try {
    for (const [name, app] of Object.entries(APPS)) {
      const dir = join(root, 'apps', name, 'src-tauri', 'installer');
      mkdirSync(dir, { recursive: true });
      for (const [kind, draw] of [['header', header], ['sidebar', sidebar], ['dmg', dmg]]) {
        const { file, width, height } = ART[kind];
        const p = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 1 });
        await p.setContent(draw(app, fonts));
        await p.evaluate(() => document.fonts.ready);
        const png = await p.screenshot({ type: 'png' });
        let bytes = png;
        if (file.endsWith('.bmp')) {
          // The browser decodes its own screenshot, so the pixels need no PNG decoder here.
          const rgba = await p.evaluate(async ({ data, w, h }) => {
            const img = new Image();
            img.src = `data:image/png;base64,${data}`;
            await img.decode();
            const c = document.createElement('canvas');
            c.width = w;
            c.height = h;
            const g = c.getContext('2d');
            g.drawImage(img, 0, 0);
            return Array.from(g.getImageData(0, 0, w, h).data);
          }, { data: png.toString('base64'), w: width, h: height });
          bytes = bmp(width, height, Uint8Array.from(rgba));
        }
        writeFileSync(join(dir, file), bytes);
        await p.close();
        console.log(`wrote ${relative(root, join(dir, file))}`);
      }
    }
  } finally {
    await browser.close();
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
