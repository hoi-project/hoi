import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { test } from 'node:test';
import { ART, DMG_POSITIONS, bmp, imageSize } from './installer-art.mjs';

const conf = (app) => JSON.parse(readFileSync(new URL(`../apps/${app}/src-tauri/tauri.conf.json`, import.meta.url), 'utf8'));
const file = (app, path) => new URL(`../apps/${app}/src-tauri/${path}`, import.meta.url);

test('bitmaps are 24-bit, bottom-up, with rows padded to four bytes', () => {
  // 3×2: red green blue / white black grey
  const px = [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255, 128, 128, 128, 255];
  const out = bmp(3, 2, Uint8Array.from(px));
  assert.deepEqual(imageSize(out), { width: 3, height: 2, bits: 24 });
  assert.equal(out.length, 54 + 12 * 2);
  // The bottom row comes first, blue-green-red.
  assert.deepEqual([...out.subarray(54, 63)], [255, 255, 255, 0, 0, 0, 128, 128, 128]);
  assert.deepEqual([...out.subarray(66, 75)], [0, 0, 255, 0, 255, 0, 255, 0, 0]);
  assert.throws(() => bmp(2, 2, new Uint8Array(4)), /pixel count/);
});

test('both installers show the artwork, at the sizes NSIS and Finder expect', () => {
  for (const app of ['desktop', 'setup']) {
    const { nsis } = conf(app).bundle.windows;
    const { dmg } = conf(app).bundle.macOS;
    assert.equal(nsis.installMode, 'currentUser', app);
    assert.equal(nsis.startMenuFolder, 'hoi', app);
    for (const [path, art] of [[nsis.headerImage, ART.header], [nsis.uninstallerHeaderImage, ART.header], [nsis.sidebarImage, ART.sidebar], [dmg.background, ART.dmg]]) {
      assert.ok(existsSync(file(app, path)), `${app}: ${path}`);
      const size = imageSize(readFileSync(file(app, path)));
      assert.equal(size.width, art.width, `${app}: ${path}`);
      assert.equal(size.height, art.height, `${app}: ${path}`);
      if (path.endsWith('.bmp')) assert.equal(size.bits, 24, `${app}: ${path}`);
    }
    for (const icon of [nsis.installerIcon, nsis.uninstallerIcon]) assert.ok(existsSync(file(app, icon)), `${app}: ${icon}`);
    assert.deepEqual(dmg.windowSize, { width: ART.dmg.width, height: ART.dmg.height });
    assert.deepEqual(dmg.appPosition, DMG_POSITIONS.app);
    assert.deepEqual(dmg.applicationFolderPosition, DMG_POSITIONS.applications);
  }
});

test('the node setup app has a name and an icon of its own; the app keeps the names installs rely on', () => {
  const setup = conf('setup');
  assert.equal(setup.productName, 'hoi Node Setup');
  assert.equal(setup.mainBinaryName, 'hoi-node-setup');
  assert.equal(setup.app.windows[0].title, 'hoi Node Setup');
  assert.equal(setup.identifier, 'sh.hoi.setup'); // its saved servers live under this
  const desktop = conf('desktop');
  // install.ps1 finds the app in %LOCALAPPDATA%\hoi as hoi-desktop.exe; the data folder follows the identifier.
  assert.equal(desktop.productName, 'hoi');
  assert.equal(desktop.mainBinaryName, undefined);
  assert.equal(desktop.identifier, 'sh.hoi.desktop');
  assert.notDeepEqual(readFileSync(file('setup', 'icons/icon.ico')), readFileSync(file('desktop', 'icons/icon.ico')));
});
