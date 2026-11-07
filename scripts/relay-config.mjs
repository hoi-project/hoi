/** Explicit relay URLs are build-time policy, never supplied by an untrusted invite. */
export function relayUrls(value, fallback = []) {
  const values = value === undefined ? fallback : JSON.parse(value);
  if (!Array.isArray(values) || values.length > 3 || values.some(item => typeof item !== 'string')) {
    throw new Error('VITE_HOI_RELAY_URLS must be a JSON array of at most three relay base URLs.');
  }
  return [...new Set(values.map(value => {
    if (value.replace(/\/+$/, '') === '/relay') return '/relay';
    const url = new URL(value);
    const loopback = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
    if (url.protocol !== 'https:' && !(url.protocol === 'http:' && loopback)) throw new Error('Relay URLs require HTTPS except on loopback.');
    if (url.username || url.password || url.search || url.hash) throw new Error('Relay URLs cannot contain credentials, queries or fragments.');
    return url.href.replace(/\/+$/, '');
  }))];
}
