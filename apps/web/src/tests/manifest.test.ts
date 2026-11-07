// The manifest and theme-color metas must use the token colours (they can't read CSS variables).
import { describe, expect, it } from 'vitest';
import tokens from '../../../../design/tokens/tokens.json';
import manifestText from '../../public/manifest.webmanifest?raw';
import html from '../../index.html?raw';

const manifest = JSON.parse(manifestText);

describe('PWA colours follow the tokens', () => {
  it('manifest', () => {
    expect(manifest.background_color).toBe(tokens.color.light.bg.value);
    expect(manifest.theme_color).toBe(tokens.color.light.card.value);
    expect(manifest.start_url).toBe('./');
    expect(manifest.icons.some((i: { purpose?: string }) => i.purpose === 'maskable')).toBe(true);
  });
  it('theme-color metas', () => {
    expect(html).toContain(`content="${tokens.color.light.card.value}" media="(prefers-color-scheme: light)"`);
    expect(html).toContain(`content="${tokens.color.dark.card.value}" media="(prefers-color-scheme: dark)"`);
  });
});
