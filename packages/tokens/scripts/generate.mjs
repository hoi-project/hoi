// Generates platform token files from design/tokens/tokens.json, the single source of truth.
// Outputs are committed so native developers never need Node to get the palette.
// `--check` fails if any committed output is stale (used in CI).
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../../..');
const tokens = JSON.parse(readFileSync(resolve(root, 'design/tokens/tokens.json'), 'utf8'));
const check = process.argv.includes('--check');

const light = Object.fromEntries(Object.entries(tokens.color.light).map(([k, v]) => [k, v.value]));
const darkOverrides = Object.fromEntries(Object.entries(tokens.color.dark).filter(([k]) => !k.startsWith('_')).map(([k, v]) => [k, v.value]));
const dark = { ...light, ...darkOverrides };
const TAG_INK = '#1A1A1A'; // text on pastels, sun and coral is always this, in both themes
const header = (c) => `${c} Generated from design/tokens/tokens.json by packages/tokens/scripts/generate.mjs. Do not edit.`;

function css() {
  const vars = (set) => Object.entries(set).map(([k, v]) => `--${k}:${v};`).join(' ');
  const darkVars = vars(darkOverrides);
  const lines = [];
  lines.push(`/*${header('').trim()} */`);
  lines.push(':root{');
  lines.push(`  ${vars(light)} --tagink:${TAG_INK};`);
  lines.push(`  --display:'${tokens.font.display.family}',${tokens.font.display.fallback}; --mono:'${tokens.font.mono.family}',${tokens.font.mono.fallback};`);
  for (const [k, v] of Object.entries(tokens.space)) lines.push(`  --space-${k}:${v};`);
  for (const [k, v] of Object.entries(tokens.radius)) lines.push(`  --radius-${k}:${v};`);
  for (const [k, v] of Object.entries(tokens.type)) {
    lines.push(`  --type-${k}-size:${v.size}; --type-${k}-weight:${v.weight};` + (v.lineHeight ? ` --type-${k}-lh:${v.lineHeight};` : '') + (v.tracking ? ` --type-${k}-tracking:${v.tracking};` : ''));
  }
  for (const [k, v] of Object.entries(tokens.motion.duration)) lines.push(`  --dur-${k}:${v}ms;`);
  lines.push('  color-scheme:light;');
  lines.push('}');
  lines.push(`@media (prefers-color-scheme: dark){ :root:not([data-theme="light"]){ ${darkVars} color-scheme:dark } }`);
  // Any subtree can be dark on its own (the first-run game's field), whatever the page is.
  lines.push(`:root[data-theme="dark"], [data-theme="dark"]{ ${darkVars} color-scheme:dark }`);
  return lines.join('\n') + '\n';
}

const camel = (k) => k.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
const hex = (v) => v.replace('#', '').toUpperCase();

function swift() {
  const entries = Object.keys(light);
  const out = [];
  out.push(`${header('//')}`);
  out.push('import SwiftUI');
  out.push('');
  out.push('/// hoi colour tokens. Each colour resolves for light and dark appearance like the web (`prefers-color-scheme`).');
  out.push('/// Text on pastels, `sun` and `coral` is always `HoiColor.tagInk`, in both themes.');
  out.push('public enum HoiColor {');
  for (const k of entries) out.push(`    public static let ${camel(k)} = Color(light: 0x${hex(light[k])}, dark: 0x${hex(dark[k])})`);
  out.push(`    public static let tagInk = Color(hex: 0x${hex(TAG_INK)})`);
  out.push(`    /// Community palette, in the order communities pick from.`);
  out.push(`    public static let communityPalette: [String: Color] = [${tokens.color.community_palette.map((c) => `"${c}": ${camel(c)}`).join(', ')}]`);
  out.push('}');
  out.push('');
  out.push('public enum HoiSpace {');
  for (const [k, v] of Object.entries(tokens.space)) out.push(`    public static let s${k}: CGFloat = ${parseFloat(v)}`);
  out.push('}');
  out.push('');
  out.push('public enum HoiRadius {');
  for (const [k, v] of Object.entries(tokens.radius)) out.push(`    public static let ${camel(k)}: CGFloat = ${parseFloat(v)}`);
  out.push('}');
  out.push('');
  out.push('public enum HoiFont {');
  out.push(`    public static let display = "${tokens.font.display.family}"`);
  out.push(`    public static let mono = "${tokens.font.mono.family}"`);
  out.push('}');
  out.push('');
  out.push('public enum HoiMotion {');
  for (const [k, v] of Object.entries(tokens.motion.duration)) out.push(`    public static let ${camel(k)}: Double = ${v / 1000}`);
  for (const [k, v] of Object.entries(tokens.motion.stagger)) out.push(`    public static let stagger${k[0].toUpperCase() + k.slice(1)}: Double = ${v / 1000}`);
  out.push('}');
  out.push('');
  out.push('extension Color {');
  out.push('    init(hex: UInt32) {');
  out.push('        self.init(.sRGB, red: Double((hex >> 16) & 0xFF) / 255, green: Double((hex >> 8) & 0xFF) / 255, blue: Double(hex & 0xFF) / 255, opacity: 1)');
  out.push('    }');
  out.push('    init(light: UInt32, dark: UInt32) {');
  out.push('        #if canImport(UIKit)');
  out.push('        self.init(UIColor { $0.userInterfaceStyle == .dark ? UIColor(Color(hex: dark)) : UIColor(Color(hex: light)) })');
  out.push('        #else');
  out.push('        self.init(hex: light)');
  out.push('        #endif');
  out.push('    }');
  out.push('}');
  return out.join('\n') + '\n';
}

function kotlin() {
  const entries = Object.keys(light);
  const out = [];
  out.push(header('//'));
  out.push('package sh.hoi.core.theme');
  out.push('');
  out.push('import androidx.compose.ui.graphics.Color');
  out.push('import androidx.compose.ui.unit.dp');
  out.push('');
  out.push('/** hoi colour tokens. Text on pastels, sun and coral is always [HoiColors.tagInk], in both themes. */');
  out.push('data class HoiColors(');
  out.push(entries.map((k) => `    val ${camel(k)}: Color,`).join('\n'));
  out.push('    val tagInk: Color = Color(0xFF' + hex(TAG_INK) + '),');
  out.push(')');
  out.push('');
  out.push('val HoiLight = HoiColors(');
  out.push(entries.map((k) => `    ${camel(k)} = Color(0xFF${hex(light[k])}),`).join('\n'));
  out.push(')');
  out.push('');
  out.push('val HoiDark = HoiColors(');
  out.push(entries.map((k) => `    ${camel(k)} = Color(0xFF${hex(dark[k])}),`).join('\n'));
  out.push(')');
  out.push('');
  out.push(`/** Community palette, in the order communities pick from. */`);
  out.push(`val CommunityPalette = listOf(${tokens.color.community_palette.map((c) => `"${c}"`).join(', ')})`);
  out.push('');
  out.push('object HoiSpace {');
  for (const [k, v] of Object.entries(tokens.space)) out.push(`    val s${k} = ${parseFloat(v)}.dp`);
  out.push('}');
  out.push('');
  out.push('object HoiRadius {');
  for (const [k, v] of Object.entries(tokens.radius)) out.push(`    val ${camel(k)} = ${parseFloat(v)}.dp`);
  out.push('}');
  out.push('');
  out.push('object HoiMotion {');
  for (const [k, v] of Object.entries(tokens.motion.duration)) out.push(`    const val ${camel(k)}Ms = ${v}`);
  for (const [k, v] of Object.entries(tokens.motion.stagger)) out.push(`    const val stagger${k[0].toUpperCase() + k.slice(1)}Ms = ${v}`);
  out.push('}');
  out.push('');
  out.push(`const val HoiDisplayFont = "${tokens.font.display.family}"`);
  out.push(`const val HoiMonoFont = "${tokens.font.mono.family}"`);
  return out.join('\n') + '\n';
}

const outputs = [
  ['packages/tokens/generated/tokens.css', css()],
  ['packages/tokens/generated/HoiTokens.swift', swift()],
  ['packages/tokens/generated/HoiTokens.kt', kotlin()],
];
// Native packages get their own copy when they exist (apps/mobile).
const nativeTargets = [
  ['apps/mobile/ios/HoiCore/Sources/HoiTheme/HoiTokens.swift', swift()],
  ['apps/mobile/android/hoi-core/src/main/kotlin/sh/hoi/core/theme/HoiTokens.kt', kotlin()],
];
for (const [p, c] of nativeTargets) if (existsSync(resolve(root, dirname(p)))) outputs.push([p, c]);

let stale = 0;
for (const [p, content] of outputs) {
  const abs = resolve(root, p);
  if (check) {
    if (!existsSync(abs) || readFileSync(abs, 'utf8') !== content) { console.error(`stale: ${p}`); stale++; }
    continue;
  }
  mkdirSync(dirname(abs), { recursive: true });
  writeFileSync(abs, content);
  console.log(`wrote ${p}`);
}
if (stale) { console.error(`${stale} token output(s) are stale; run \`pnpm tokens\`.`); process.exit(1); }
