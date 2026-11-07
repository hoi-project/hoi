// Create or update the repository's labels from .github/labels.yml with the GitHub CLI.
//   node scripts/labels.mjs [--dry-run] [--repo owner/name]
// It never deletes labels: GitHub's defaults and labels added by hand stay.
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

/** Parses the one-label-per-line flow-map format of .github/labels.yml (no YAML dependency). */
export function parseLabels(text) {
  const labels = [];
  for (const [index, raw] of text.split(/\r?\n/).entries()) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    const body = /^- \{(.*)\}$/.exec(line);
    if (!body) throw new Error(`labels.yml:${index + 1}: expected "- { name: …, color: …, description: … }"`);
    const fields = {};
    for (const [, key, quoted, plain] of body[1].matchAll(/(\w+):\s*(?:"([^"]*)"|([^,]*?))\s*(?:,|$)/g)) {
      fields[key] = quoted ?? plain;
    }
    if (!fields.name || !/^[0-9a-f]{6}$/i.test(fields.color ?? '')) {
      throw new Error(`labels.yml:${index + 1}: needs a name and a six-digit hex color`);
    }
    if ((fields.description ?? '').length > 100) throw new Error(`labels.yml:${index + 1}: description over 100 characters`);
    labels.push({ name: fields.name, color: fields.color.toLowerCase(), description: fields.description ?? '' });
  }
  const names = labels.map((label) => label.name.toLowerCase());
  const duplicate = names.find((name, i) => names.indexOf(name) !== i);
  if (duplicate) throw new Error(`labels.yml: duplicate label ${duplicate}`);
  return labels;
}

function main() {
  const args = process.argv.slice(2);
  const dryRun = args.includes('--dry-run');
  const repoIndex = args.indexOf('--repo');
  const repo = repoIndex >= 0 ? ['--repo', args[repoIndex + 1]] : [];
  const file = fileURLToPath(new URL('../.github/labels.yml', import.meta.url));
  for (const label of parseLabels(readFileSync(file, 'utf8'))) {
    const command = ['label', 'create', label.name, '--color', label.color, '--description', label.description, '--force', ...repo];
    if (dryRun) console.log(`gh ${command.map((part) => JSON.stringify(part)).join(' ')}`);
    else execFileSync('gh', command, { stdio: 'inherit' });
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main();
