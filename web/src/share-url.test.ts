import { describe, expect, it } from 'vitest';
import { browserShareUrl, interfaceShareUrl } from './share-url';

describe('share URL selection', () => {
  it('uses the current browser origin without adding the internal port', () => {
    expect(browserShareUrl('https://localsendy.lazycat.example/app')).toBe(
      'https://localsendy.lazycat.example/share',
    );
  });

  it('adds the web port to a selected IPv4 interface', () => {
    expect(interfaceShareUrl('192.168.1.20/24', 52222)).toBe(
      'http://192.168.1.20:52222/share',
    );
  });

  it('wraps a selected IPv6 interface in brackets', () => {
    expect(interfaceShareUrl('fd00::20/64', 52222)).toBe(
      'http://[fd00::20]:52222/share',
    );
  });
});
