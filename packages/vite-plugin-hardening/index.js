// Build-time hardening for hoi's static sites (TECHNICAL.md §11: strict CSP + SRI, no third-party scripts).
// - Injects a Content-Security-Policy <meta> into every emitted HTML page (build only; dev keeps HMR working).
// - Adds integrity="sha384-…" to every <script src> and <link rel=stylesheet|modulepreload> that points at a bundled file.
// Hosts that support response headers should also send the same policy (plus frame-ancestors) as a header; see `headers()`.
import { createHash } from 'node:crypto';

/** @param {Record<string, string[]>} directives */
export function cspString(directives) {
  return Object.entries(directives)
    .map(([k, v]) => (v.length ? `${k} ${v.join(' ')}` : k))
    .join('; ');
}

/**
 * @param {{ csp?: Record<string, string[]> | false, sri?: boolean }} [opts]
 * @returns {import('vite').Plugin}
 */
export default function hardening(opts = {}) {
  const csp = opts.csp === false ? null : cspString(opts.csp ?? {
    'default-src': ["'self'"],
    'script-src': ["'self'"],
    'style-src': ["'self'"],
    'img-src': ["'self'", 'data:'],
    'font-src': ["'self'"],
    'connect-src': ["'self'"],
    'object-src': ["'none'"],
    'base-uri': ["'none'"],
    'form-action': ["'none'"],
  });
  const sri = opts.sri !== false;
  let base = '/';

  return {
    name: 'hoi-hardening',
    apply: 'build',
    enforce: 'post',
    configResolved(config) {
      base = config.base || '/';
    },
    // `order: 'post'`: Vite's import analysis rewrites dynamic-import preload lists in its own
    // generateBundle, so hashing earlier would give entry chunks a stale integrity value.
    generateBundle: { order: 'post', handler(_options, bundle) {
      const hashOf = new Map();
      for (const [fileName, item] of Object.entries(bundle)) {
        const data = item.type === 'chunk' ? item.code : item.source;
        hashOf.set(fileName, 'sha384-' + createHash('sha384').update(data).digest('base64'));
      }
      for (const [fileName, item] of Object.entries(bundle)) {
        if (item.type !== 'asset' || !fileName.endsWith('.html')) continue;
        let html = typeof item.source === 'string' ? item.source : Buffer.from(item.source).toString('utf8');
        if (csp && !html.includes('Content-Security-Policy')) {
          html = html.replace(/<head>/i, `<head>\n    <meta http-equiv="Content-Security-Policy" content="${csp}">`);
        }
        if (sri) {
          html = html.replace(/<(script|link)\b([^>]*?)(\/?)>/gi, (tag, name, attrs, slash) => {
            if (/\bintegrity=/.test(attrs)) return tag;
            const isScript = name.toLowerCase() === 'script';
            if (!isScript && !/\brel=["']?(stylesheet|modulepreload)/i.test(attrs)) return tag;
            const m = attrs.match(isScript ? /\bsrc=["']([^"']+)["']/i : /\bhref=["']([^"']+)["']/i);
            if (!m) return tag;
            const ref = resolveRef(m[1], fileName, base);
            const h = ref && hashOf.get(ref);
            if (!h) return tag;
            const co = /\bcrossorigin\b/i.test(attrs) ? '' : ' crossorigin';
            return `<${name}${attrs} integrity="${h}"${co}${slash}>`;
          });
        }
        item.source = html;
      }
    } },
  };
}

function resolveRef(url, htmlFile, base) {
  if (/^(https?:)?\/\//i.test(url) || url.startsWith('data:')) return null;
  let p = url.split(/[?#]/)[0];
  if (p.startsWith(base)) p = p.slice(base.length);
  else if (p.startsWith('/')) p = p.slice(1);
  else {
    const dir = htmlFile.includes('/') ? htmlFile.slice(0, htmlFile.lastIndexOf('/') + 1) : '';
    const parts = (dir + p).split('/');
    const out = [];
    for (const s of parts) {
      if (s === '..') out.pop();
      else if (s !== '.' && s !== '') out.push(s);
    }
    p = out.join('/');
  }
  return p;
}

/** `_headers` body (for hosts that read it, such as Netlify) for the same policy, plus headers a meta tag cannot set. */
export function headers(directives, extra = {}) {
  const policy = cspString({ ...directives, 'frame-ancestors': ["'none'"] });
  const lines = [
    '/*',
    `  Content-Security-Policy: ${policy}`,
    '  X-Content-Type-Options: nosniff',
    '  Referrer-Policy: no-referrer',
    '  Strict-Transport-Security: max-age=31536000; includeSubDomains',
    '  Permissions-Policy: interest-cohort=(), browsing-topics=(), geolocation=(), microphone=(), payment=()',
    '  Cross-Origin-Opener-Policy: same-origin',
    ...Object.entries(extra).map(([k, v]) => `  ${k}: ${v}`),
  ];
  return lines.join('\n') + '\n';
}
