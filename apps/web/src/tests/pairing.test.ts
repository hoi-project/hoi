import { describe, expect, it } from 'vitest';
import { describeHome } from '../lib/pairing';

describe('reading a pairing link', () => {
  it('says whether the link points at this device, the local network or the internet', () => {
    expect(describeHome('ws://127.0.0.1:7842', null)).toEqual({ host: '127.0.0.1', scope: 'device' });
    expect(describeHome('ws://localhost:7842', 'abc@192.168.1.20:7844')).toEqual({ host: 'localhost, 192.168.1.20', scope: 'network' });
    expect(describeHome(null, 'abc@[fe80::1]:7844,abc@10.0.0.3:7844').scope).toBe('network');
    expect(describeHome('wss://home.example:7842', null)).toEqual({ host: 'home.example', scope: 'internet' });
    expect(describeHome('ws://127.0.0.1:7842', 'abc@203.0.113.9:7844').scope).toBe('internet');
    expect(describeHome(null, null)).toEqual({ host: '', scope: 'none' });
  });
  it('never lets a name stand in for the address', () => {
    expect(describeHome('wss://lena-laptop@evil.example/', null).host).toBe('evil.example');
    expect(describeHome('not a url', null)).toEqual({ host: 'not a url', scope: 'internet' });
  });
});
