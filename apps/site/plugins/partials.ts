import { readFileSync } from 'node:fs';
import type { Plugin } from 'vite';

/**
 * Tiny HTML partials for the static site, resolved in `transformIndexHtml` (dev and build).
 *
 *   <!-- @include header.html nav="site" -->   inlines partials/header.html; attributes become variables
 *   {{APP_URL}}                                 replaced by a variable (global, or from the include)
 *
 * Variables are HTML-escaped. An unknown variable or a missing partial fails the build.
 * Links marked `data-nav` whose href equals the current page get aria-current="page".
 */
export interface PartialsOptions {
  /** Absolute path of the partials directory. */
  dir: string;
  /** Global variables, available in pages and partials. */
  vars: Record<string, string>;
}

const INCLUDE = /<!--\s*@include\s+([\w.-]+\.html)((?:\s+[\w-]+="[^"]*")*)\s*-->/g;
const ATTR = /([\w-]+)="([^"]*)"/g;
const VAR = /\{\{\s*([\w-]+)\s*\}\}/g;

const escapeHtml = (s: string) =>
  s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

export default function partials(opts: PartialsOptions): Plugin {
  const dir = opts.dir.endsWith('/') ? opts.dir : opts.dir + '/';
  const used = new Set<string>();

  function expand(html: string, vars: Record<string, string>, from: string, depth: number): string {
    if (depth > 8) throw new Error(`[hoi-partials] includes nested too deep in ${from} (a cycle?)`);
    // Variables first, so an include name can itself use one (nav-{{nav}}.html).
    const withVars = html.replace(VAR, (_m, key: string) => {
      if (!(key in vars)) throw new Error(`[hoi-partials] unknown variable {{${key}}} in ${from}`);
      return vars[key];
    });
    return withVars.replace(INCLUDE, (_m, name: string, attrs: string) => {
      const local = { ...vars };
      for (const [, k, v] of attrs.matchAll(ATTR)) local[k] = v;
      const file = dir + name;
      let src: string;
      try {
        src = readFileSync(file, 'utf8');
      } catch {
        throw new Error(`[hoi-partials] missing partial ${name} (included from ${from})`);
      }
      used.add(file);
      return expand(src.trim(), local, name, depth + 1);
    });
  }

  return {
    name: 'hoi-partials',
    transformIndexHtml: {
      order: 'pre',
      handler(html, ctx) {
        // ctx.path is "/download/index.html" in build and dev; the page URL is "/download/".
        const page = ctx.path.replace(/index\.html$/, '');
        const vars: Record<string, string> = { PAGE: page };
        for (const [k, v] of Object.entries(opts.vars)) vars[k] = escapeHtml(v);
        const out = expand(html, vars, ctx.path, 0);
        const href = page.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
        return out.replace(
          new RegExp(`<a((?=[^>]*\\bdata-nav\\b)[^>]*\\bhref="${href}"[^>]*)>`, 'g'),
          '<a$1 aria-current="page">',
        );
      },
    },
    handleHotUpdate({ file, server }) {
      // Partials aren't modules; reload every page when one changes in dev.
      if (file.startsWith(dir)) {
        server.ws.send({ type: 'full-reload' });
        return [];
      }
    },
  };
}
