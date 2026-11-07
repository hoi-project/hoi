import type { Plugin } from 'vite';
import { headers } from '@hoi/vite-plugin-hardening';

/**
 * Emits `_headers` (the format Netlify and similar hosts read) next to the pages, carrying the exact
 * Content-Security-Policy that @hoi/vite-plugin-hardening injected as a <meta>, plus the headers
 * a meta tag can't set (frame-ancestors, nosniff, referrer policy, …). Reading the policy back
 * from the built index.html keeps the two in lockstep. Must run after hardening().
 */
export default function headersFile(extra: Record<string, string> = {}): Plugin {
  return {
    name: 'hoi-headers-file',
    apply: 'build',
    enforce: 'post',
    // After hardening's own post-ordered hook has written the CSP <meta>.
    generateBundle: { order: 'post', handler(_options, bundle) {
      const index = bundle['index.html'];
      const html = index && index.type === 'asset' ? String(index.source) : '';
      const m = html.match(/<meta http-equiv="Content-Security-Policy" content="([^"]+)">/);
      if (!m) this.error('[hoi-headers-file] no CSP <meta> in index.html; is hardening() listed before this plugin?');
      const directives: Record<string, string[]> = {};
      for (const part of m[1].split(';')) {
        const [name, ...values] = part.trim().split(/\s+/);
        if (name) directives[name] = values;
      }
      const body =
        headers(directives, extra) +
        [
          '',
          '# Hashed build output never changes.',
          '/assets/*',
          '  Cache-Control: public, max-age=31536000, immutable',
          '',
          '# The release manifest is what the apps check for updates; always revalidate.',
          '/releases/*',
          '  Cache-Control: no-cache',
          '',
          '# Operator release notices can change independently of a software build.',
          '/release.json',
          '  Cache-Control: no-store',
          '',
          '# The installers are read by a shell or PowerShell, not rendered: plain text, always fresh.',
          '/install.sh',
          '  Content-Type: text/plain; charset=utf-8',
          '  Cache-Control: no-cache',
          '/install.ps1',
          '  Content-Type: text/plain; charset=utf-8',
          '  Cache-Control: no-cache',
          '',
        ].join('\n');
      this.emitFile({ type: 'asset', fileName: '_headers', source: body });
    } },
  };
}
