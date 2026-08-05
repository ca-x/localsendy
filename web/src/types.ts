export type Tab = 'send' | 'receive' | 'settings';
export type Locale = 'en' | 'zh-CN' | 'zh-TW';

export interface DeviceInfo {
  alias: string;
  version: string;
  deviceModel?: string;
  deviceType?: 'mobile' | 'desktop' | 'web' | 'headless' | 'server';
  fingerprint: string;
  port: number;
  protocol: 'http' | 'https';
  download: boolean;
  ip?: string;
  sourceInterface?: string;
  sourceInterfaceLabel?: string;
}

export interface StatusResponse {
  alias: string;
  webAddress: string;
  localsendPort: number;
  protocol: string;
  dataDirectory: string;
  autoAccept: boolean;
  uptimeSeconds: number;
  nearbyDevices: number;
}

export type NetworkMode = 'all' | 'selected';
export type NetworkInterfaceKind = 'ethernet' | 'wifi' | 'bridge' | 'tunnel' | 'virtual' | 'other';

export interface NetworkInterfaceInfo {
  name: string;
  label?: string;
  kind: NetworkInterfaceKind;
  ipv4Addresses: string[];
  ipv6Addresses: string[];
  ipv4Discovery: boolean;
  ipv6Discovery: boolean;
  discoveryCapable: boolean;
  pointToPoint: boolean;
  selected: boolean;
}

export interface NetworkSettings {
  mode: NetworkMode;
  selectedInterfaces: string[];
  activeDiscoveryInterfaces: string[];
  interfaces: NetworkInterfaceInfo[];
}

export interface PendingTransfer {
  sender: DeviceInfo;
  files: Array<{ id: string; name: string; size: number; fileType: string }>;
  totalBytes: number;
}

export interface ReceivedFile {
  fileName: string;
  size: number;
  sender: string;
  time: string;
}

export interface OutgoingTransfer {
  id: string;
  targetAlias: string;
  fileNames: string[];
  totalBytes: number;
  status: 'preparing' | 'completed' | 'failed';
  createdAt: string;
  error?: string;
}
