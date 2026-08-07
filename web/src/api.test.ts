import { afterEach, describe, expect, it, vi } from 'vitest';
import { scanDevices, sendFiles, sendText, startLinkShare, stopLinkShare, updateEnvironmentSettings } from './api';
import type { DeviceInfo } from './types';

const target: DeviceInfo = {
  alias: 'Phone',
  version: '2.1',
  fingerprint: 'fingerprint',
  port: 53317,
  protocol: 'https',
  download: false,
  ip: '192.168.1.10',
};

const responseBody = JSON.stringify({ transferId: 'transfer', transfers: [], filesSent: 0, totalBytes: 0 });

describe('API responses', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('accepts a successful empty discovery response', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(null, { status: 202 })));

    await expect(scanDevices()).resolves.toBeUndefined();
  });

  it('updates environment-backed settings through the settings endpoint', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({
      autoAccept: true,
      alias: '聪明的覆盆子',
      aliasLocale: 'zh-CN',
    }), { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);

    await expect(updateEnvironmentSettings({
      autoAccept: true,
      alias: '',
      aliasLocale: 'zh-CN',
    })).resolves.toMatchObject({ aliasLocale: 'zh-CN' });

    const [path, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(path).toBe('/api/v1/settings');
    expect(init.method).toBe('PUT');
    expect(JSON.parse(String(init.body))).toEqual({
      autoAccept: true,
      alias: '',
      aliasLocale: 'zh-CN',
    });
  });

  it('sends clipboard text to every selected device', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(responseBody, { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);

    await sendText([target, { ...target, alias: 'Tablet', fingerprint: 'tablet' }], 'hello');

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(JSON.parse(String(init.body))).toMatchObject({
      text: 'hello',
      targets: [{ alias: 'Phone' }, { alias: 'Tablet' }],
    });
  });

  it('puts all selected devices before files in multipart requests', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(responseBody, { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);

    await sendFiles([target], [new File(['hello'], 'hello.txt', { type: 'text/plain' })]);

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    const form = init.body as FormData;
    expect(JSON.parse(String(form.get('targets')))).toEqual([target]);
    expect((form.getAll('files')[0] as File).name).toBe('hello.txt');
  });

  it('reports browser upload progress when requested', async () => {
    class FakeXmlHttpRequest {
      status = 200;
      responseText = responseBody;
      upload: { onprogress: ((event: ProgressEvent) => void) | null } = { onprogress: null };
      onload: (() => void) | null = null;
      onerror: (() => void) | null = null;
      onabort: (() => void) | null = null;

      open() {}

      send() {
        this.upload.onprogress?.(new ProgressEvent('progress', {
          lengthComputable: true,
          loaded: 5,
          total: 10,
        }));
        this.onload?.();
      }
    }
    vi.stubGlobal('XMLHttpRequest', FakeXmlHttpRequest);
    const onProgress = vi.fn();

    await sendFiles(
      [target],
      [new File(['hello'], 'hello.txt', { type: 'text/plain' })],
      undefined,
      onProgress,
    );

    expect(onProgress).toHaveBeenCalledWith({ loaded: 5, total: 10 });
  });

  it('starts one link share with files before opening the share page', async () => {
    let submittedBody: FormData | undefined;
    class FakeXmlHttpRequest {
      status = 200;
      responseText = JSON.stringify({ active: true, urls: ['http://host/share'], files: [] });
      upload: { onprogress: ((event: ProgressEvent) => void) | null } = { onprogress: null };
      onload: (() => void) | null = null;
      onerror: (() => void) | null = null;
      onabort: (() => void) | null = null;
      body?: FormData;

      open(_method: string, path: string) {
        expect(path).toBe('/api/v1/share');
      }

      send(body: FormData) {
        this.body = body;
        submittedBody = body;
        expect(body.get('autoAccept')).toBe('false');
        expect((body.get('files') as File).name).toBe('share.txt');
        this.onload?.();
      }
    }
    vi.stubGlobal('XMLHttpRequest', FakeXmlHttpRequest);

    await expect(startLinkShare(
      [new File(['share'], 'share.txt', { type: 'text/plain' })],
      false,
      '',
      'https://localsendy.lazycat.example/share',
      vi.fn(),
    )).resolves.toMatchObject({ active: true });

    expect(submittedBody?.get('shareUrl')).toBe('https://localsendy.lazycat.example/share');
  });

  it('stops only the identified share with a keepalive delete request', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal('fetch', fetchMock);

    await stopLinkShare('share/id', true);

    const [path, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(path).toBe('/api/v1/share?shareId=share%2Fid');
    expect(init).toMatchObject({ method: 'DELETE', keepalive: true });
  });
});
