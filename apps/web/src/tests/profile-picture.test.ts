import { describe, expect, it } from 'vitest';
import { avatarUrl, cropRect } from '../lib/profile-picture';

describe('profile framing', () => {
  it('uses the same square crop for the preview and saved picture', () => {
    expect(cropRect(1200, 800, 1, 0, 0)).toEqual({ x: 200, y: 0, size: 800 });
    expect(cropRect(1200, 800, 2, 1, -1)).toEqual({ x: 800, y: 0, size: 400 });
    expect(cropRect(800, 1200, 1, -1, 1)).toEqual({ x: 0, y: 400, size: 800 });
  });
  it('clamps controls so the crop cannot leave the image', () => {
    expect(cropRect(800, 1200, 0, 99, -99)).toEqual({ x: 0, y: 0, size: 800 });
    expect(cropRect(800, 1200, 99, 1, 1)).toEqual({ x: 600, y: 1000, size: 200 });
  });
  it('only renders bounded local byte strings, never arbitrary image URLs', () => {
    expect(avatarUrl('https://tracker.example/pixel')).toBeUndefined();
    expect(avatarUrl('data:image/svg+xml;base64,abcd')).toBeUndefined();
    expect(avatarUrl('a'.repeat(5463))).toBeUndefined();
    expect(avatarUrl('_9j_')).toBe('data:image/jpeg;base64,/9j/');
  });
});
