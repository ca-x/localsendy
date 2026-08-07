import { describe, expect, it } from 'vitest';
import { summarizeSendProgress } from './send-progress';

describe('complete send progress', () => {
  it('combines browser staging and every device transfer without claiming early completion', () => {
    expect(summarizeSendProgress(
      { loaded: 110, total: 110 },
      [
        { transferredBytes: 25, totalBytes: 100 },
        { transferredBytes: 50, totalBytes: 100 },
      ],
      200,
    )).toEqual({ loaded: 185, total: 310 });
  });

  it('keeps the full target total before server-side transfer records appear', () => {
    expect(summarizeSendProgress(
      { loaded: 110, total: 110 },
      [],
      200,
    )).toEqual({ loaded: 110, total: 310 });
  });

  it('keeps the full target total while only some concurrent records exist', () => {
    expect(summarizeSendProgress(
      { loaded: 110, total: 110 },
      [{ transferredBytes: 25, totalBytes: 100 }],
      300,
    )).toEqual({ loaded: 135, total: 410 });
  });
});
