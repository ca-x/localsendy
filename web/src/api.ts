import type {
  DeviceInfo,
  NetworkMode,
  NetworkSettings,
  OutgoingTransfer,
  PendingTransfer,
  ReceivedFile,
  StatusResponse,
  StorageSettings,
  DirectoryListing,
} from './types';

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`/api/v1${path}`, init);
  if (!response.ok) {
    const body = await response.json().catch(() => ({} as { error?: string }));
    throw new Error(body.error ?? `Request failed (${response.status})`);
  }
  if (response.status === 204) return undefined as T;
  return response.json() as Promise<T>;
}

export const getStatus = () => request<StatusResponse>('/status');
export const getDevices = () => request<DeviceInfo[]>('/devices');
export const scanDevices = () => request<void>('/devices/scan', { method: 'POST' });
export const getNetworkSettings = () => request<NetworkSettings>('/networks');
export const updateNetworkSettings = (mode: NetworkMode, selectedInterfaces: string[], labels: Record<string, string>) =>
  request<NetworkSettings>('/networks', {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ mode, selectedInterfaces, labels }),
  });
export const probeDevice = (address: string) =>
  request<DeviceInfo>('/devices/probe', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ address }),
  });
export const getPending = () => request<PendingTransfer | null>('/pending');
export const decidePending = (decision: 'accept' | 'reject') =>
  request<void>(`/pending/${decision}`, { method: 'POST' });
export const getHistory = () => request<ReceivedFile[]>('/history');
export const getTransfers = () => request<OutgoingTransfer[]>('/transfers');
export const getStorageSettings = () => request<StorageSettings>('/storage');
export const updateStorageSettings = (subdirectory: string) =>
  request<StorageSettings>('/storage', {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ subdirectory }),
  });
export const listStorageDirectories = (path = '') => {
  const query = new URLSearchParams({ path });
  return request<DirectoryListing>(`/storage/directories?${query}`);
};
export const createStorageDirectory = (parent: string, name: string) =>
  request<DirectoryListing>('/storage/directories', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ parent, name }),
  });

export async function sendFiles(target: DeviceInfo, files: File[], pin?: string) {
  const form = new FormData();
  form.append('target', JSON.stringify(target));
  if (pin) form.append('pin', pin);
  files.forEach((file) => form.append('files', file, file.name));
  return request<{ transferId: string; filesSent: number; totalBytes: number }>('/send', {
    method: 'POST',
    body: form,
  });
}
