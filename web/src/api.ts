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
  IncomingTransfer,
  SendResponse,
  EnvironmentSettings,
} from './types';

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`/api/v1${path}`, init);
  const text = response.status === 204 ? '' : await response.text();
  if (!response.ok) {
    const body = text ? JSON.parse(text) as { error?: string } : {};
    throw new Error(body.error ?? `Request failed (${response.status})`);
  }
  if (!text) return undefined as T;
  return JSON.parse(text) as T;
}

export interface UploadProgress {
  loaded: number;
  total: number;
}

function uploadMultipart<T>(path: string, body: FormData, onProgress: (progress: UploadProgress) => void): Promise<T> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open('POST', `/api/v1${path}`);
    xhr.upload.onprogress = (event) => {
      if (event.lengthComputable) {
        onProgress({ loaded: event.loaded, total: event.total });
      }
    };
    xhr.onload = () => {
      const text = xhr.responseText;
      if (xhr.status < 200 || xhr.status >= 300) {
        let message = `Request failed (${xhr.status})`;
        try {
          message = (JSON.parse(text) as { error?: string }).error ?? message;
        } catch {
          // Keep the status-based fallback for non-JSON proxy errors.
        }
        reject(new Error(message));
        return;
      }
      try {
        resolve(text ? JSON.parse(text) as T : undefined as T);
      } catch {
        reject(new Error('The server returned an invalid response'));
      }
    };
    xhr.onerror = () => reject(new Error('The upload connection failed'));
    xhr.onabort = () => reject(new Error('The upload was cancelled'));
    xhr.send(body);
  });
}

export const getStatus = () => request<StatusResponse>('/status');
export const getEnvironmentSettings = () => request<EnvironmentSettings>('/settings');
export const updateEnvironmentSettings = (settings: Partial<EnvironmentSettings>) =>
  request<EnvironmentSettings>('/settings', {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(settings),
  });
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
export const getIncomingTransfers = () => request<IncomingTransfer[]>('/transfers/incoming');
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

export async function sendFiles(
  targets: DeviceInfo[],
  files: File[],
  pin?: string,
  onUploadProgress?: (progress: UploadProgress) => void,
) {
  const form = new FormData();
  form.append('targets', JSON.stringify(targets));
  if (pin) form.append('pin', pin);
  files.forEach((file) => form.append('files', file, file.name));
  if (onUploadProgress) {
    return uploadMultipart<SendResponse>('/send', form, onUploadProgress);
  }
  return request<SendResponse>('/send', {
    method: 'POST',
    body: form,
  });
}

export const sendText = (targets: DeviceInfo[], text: string, pin?: string) =>
  request<SendResponse>('/send/text', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ targets, text, pin }),
  });
