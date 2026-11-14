import { describe, expect, it } from 'vitest';
import source from '../setup/Setup.svelte?raw';

describe('hoi Node Setup uploads', () => {
  it('never uploads to a fixed name under /tmp that root later copies (a private mktemp -d folder instead)', () => {
    // A shared fixed path lets a second local user pre-create or link the name and steer what root installs.
    const fixed = [...source.matchAll(/bridge\.upload\(session, ['"`](\/tmp\/[^'"`]+)/g)].map((m) => m[1]);
    expect(fixed).toEqual([]);
    expect(source).not.toMatch(/install [^\n]*\/tmp\/hoi-(notice|journald)/);
  });

  it('keeps the private-folder pattern for the kit, update, uninstall, notice and retention', () => {
    expect(source.match(/umask 077 && mktemp -d \/tmp\/hoi-setup\.XXXXXXXX/g)?.length ?? 0).toBeGreaterThanOrEqual(1);
    expect(source.match(/privateDir\(\)/g)?.length ?? 0).toBeGreaterThanOrEqual(5);
  });
});
