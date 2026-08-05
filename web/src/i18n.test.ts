import { describe, expect, it } from 'vitest';
import { messages } from './i18n';

describe('translations', () => {
  it('keeps every supported locale aligned with English keys', () => {
    const englishKeys = Object.keys(messages('en')).sort();
    expect(Object.keys(messages('zh-CN')).sort()).toEqual(englishKeys);
    expect(Object.keys(messages('zh-TW')).sort()).toEqual(englishKeys);
  });
});
