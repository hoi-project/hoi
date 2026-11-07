// A small syntax colourer for the wiki's code blocks. It reads the block's text and rebuilds it
// as spans (textContent only, never innerHTML). Unknown languages stay plain.
const KW: Record<string, string[]> = {
  rust: ['as', 'async', 'await', 'break', 'const', 'continue', 'crate', 'dyn', 'else', 'enum', 'extern', 'false', 'fn', 'for', 'if', 'impl', 'in', 'let', 'loop', 'match', 'mod', 'move', 'mut', 'pub', 'ref', 'return', 'self', 'Self', 'static', 'struct', 'super', 'trait', 'true', 'type', 'unsafe', 'use', 'where', 'while', 'Some', 'None', 'Ok', 'Err'],
  ts: ['as', 'async', 'await', 'break', 'case', 'catch', 'class', 'const', 'continue', 'default', 'delete', 'do', 'else', 'export', 'extends', 'false', 'finally', 'for', 'from', 'function', 'if', 'import', 'in', 'instanceof', 'interface', 'let', 'new', 'null', 'of', 'return', 'switch', 'this', 'throw', 'true', 'try', 'type', 'typeof', 'undefined', 'var', 'void', 'while', 'yield', 'readonly', 'keyof', 'declare'],
  sh: ['if', 'then', 'else', 'fi', 'for', 'do', 'done', 'case', 'esac', 'export', 'cd', 'sudo', 'echo', 'set', 'in', 'function', 'return', 'local'],
  kotlin: ['class', 'fun', 'val', 'var', 'object', 'package', 'import', 'return', 'if', 'else', 'when', 'null', 'true', 'false', 'private', 'override', 'suspend', 'data', 'interface', 'companion'],
  swift: ['class', 'func', 'let', 'var', 'struct', 'enum', 'import', 'return', 'if', 'else', 'guard', 'nil', 'true', 'false', 'private', 'public', 'throws', 'try', 'async', 'await', 'extension', 'protocol', 'case', 'switch'],
  toml: ['true', 'false'],
  json: ['true', 'false', 'null'],
  cddl: ['uint', 'int', 'bstr', 'tstr', 'bool', 'null', 'text', 'bytes', 'any', 'float'],
};
KW.js = KW.ts;
KW.typescript = KW.ts;
KW.javascript = KW.ts;
KW.bash = KW.sh;
KW.yaml = KW.json;

const HASH_COMMENT = new Set(['sh', 'bash', 'toml', 'yaml', 'py']);

interface Tok { t: string; c: string }

function tokenize(src: string, lang: string): Tok[] {
  const kw = new Set(KW[lang] ?? []);
  const hash = HASH_COMMENT.has(lang);
  const out: Tok[] = [];
  const push = (t: string, c = '') => { if (t) out.push({ t, c }); };
  // One pass: comments, strings, numbers, words, everything else.
  const re = hash
    ? /(#[^\n]*)|("(?:[^"\\\n]|\\.)*"|'(?:[^'\\\n]|\\.)*')|(\b0x[0-9a-fA-F_]+\b|\b\d[\d_]*(?:\.\d+)?(?:[eE][+-]?\d+)?\b)|([A-Za-z_][\w-]*)|([\s\S])/g
    : /(\/\/[^\n]*|\/\*[\s\S]*?\*\/)|("(?:[^"\\\n]|\\.)*"|'(?:[^'\\\n]|\\.)*'|`(?:[^`\\]|\\.)*`)|(\b0x[0-9a-fA-F_]+\b|\b\d[\d_]*(?:\.\d+)?(?:[eE][+-]?\d+)?(?:u8|u16|u32|u64|usize|i64|f64)?\b)|([A-Za-z_]\w*!?)|([\s\S])/g;
  let m: RegExpExecArray | null;
  let plain = '';
  const flush = () => { push(plain); plain = ''; };
  while ((m = re.exec(src))) {
    if (m[1]) { flush(); push(m[1], 'tk-c'); }
    else if (m[2]) {
      flush();
      // "key": in JSON-like text is a field, not a string.
      const after = src.slice(re.lastIndex).match(/^\s*:/);
      push(m[2], after && (lang === 'json' || lang === 'ts' || lang === 'js') ? 'tk-f' : 'tk-s');
    } else if (m[3]) { flush(); push(m[3], 'tk-n'); }
    else if (m[4]) {
      const w = m[4];
      flush();
      if (kw.has(w)) push(w, 'tk-k');
      else if (w.endsWith('!') && lang === 'rust') push(w, 'tk-m');
      else if (/^[A-Z][A-Za-z0-9]*$/.test(w) && w.length > 1 && !/^[A-Z0-9_]+$/.test(w)) push(w, 'tk-t');
      else if (/^[A-Z][A-Z0-9_]+$/.test(w)) push(w, 'tk-n');
      else if (/^\s*:(?!:)/.test(src.slice(re.lastIndex)) && lang !== 'sh' && lang !== 'bash') push(w, 'tk-f');
      else push(w);
    } else plain += m[5];
  }
  flush();
  return out;
}

export function highlight(code: HTMLElement, lang: string): void {
  const l = lang.toLowerCase();
  if (!KW[l] && !['cbor', 'text', 'txt', 'http'].includes(l)) return;
  if (!KW[l]) return;
  const toks = tokenize(code.textContent ?? '', l);
  const frag = document.createDocumentFragment();
  for (const tk of toks) {
    if (!tk.c) frag.appendChild(document.createTextNode(tk.t));
    else {
      const s = document.createElement('span');
      s.className = tk.c;
      s.textContent = tk.t;
      frag.appendChild(s);
    }
  }
  code.replaceChildren(frag);
}
