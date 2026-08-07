import type { UploadProgress } from './api';

interface DeviceTransferProgress {
  transferredBytes: number;
  totalBytes: number;
}

export function summarizeSendProgress(
  browser: UploadProgress,
  transfers: DeviceTransferProgress[],
  expectedDeviceBytes: number,
): UploadProgress {
  const deviceTotal = Math.max(
    expectedDeviceBytes,
    transfers.reduce((total, transfer) => total + transfer.totalBytes, 0),
  );
  const deviceLoaded = transfers.reduce((total, transfer) => total + transfer.transferredBytes, 0);
  return {
    loaded: Math.min(browser.loaded, browser.total) + Math.min(deviceLoaded, deviceTotal),
    total: browser.total + deviceTotal,
  };
}
