import { describe, expect, it } from 'vitest';
import { arrange, move } from '../lib/arrange';

const c = (...ids: string[]) => ids.map((id) => ({ id }));

describe('your own order of communities', () => {
  it('puts favorites first, then your order, then the ones you have not placed', () => {
    const a = arrange(c('a', 'b', 'c', 'd', 'e'), ['d', 'gone', 'b'], ['e', 'b', 'c']);
    expect(a.favorites.map((x) => x.id)).toEqual(['d', 'b']);
    expect(a.rest.map((x) => x.id)).toEqual(['e', 'c', 'a']);
  });

  it('keeps the core order when nothing was arranged', () => {
    expect(arrange(c('a', 'b'))).toEqual({ favorites: [], rest: c('a', 'b') });
  });

  it('moves by steps and before another, and stays inside the list', () => {
    expect(move(['a', 'b', 'c'], 'c', { by: -1 })).toEqual(['a', 'c', 'b']);
    expect(move(['a', 'b', 'c'], 'a', { by: -1 })).toEqual(['a', 'b', 'c']);
    expect(move(['a', 'b', 'c'], 'a', { by: 5 })).toEqual(['b', 'c', 'a']);
    expect(move(['a', 'b', 'c'], 'c', { before: 'a' })).toEqual(['c', 'a', 'b']);
    expect(move(['a', 'b', 'c'], 'a', { before: null })).toEqual(['b', 'c', 'a']);
    expect(move(['a'], 'x', { by: 1 })).toEqual(['a']);
  });
});
