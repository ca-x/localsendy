export type Tab = 'send' | 'receive' | 'settings';
export type Locale = 'en' | 'zh-CN' | 'zh-TW';
export type AliasLocale = 'auto' | 'en' | 'zh-CN' | 'zh-TW';

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
  version: string;
  alias: string;
  deviceType?: DeviceInfo['deviceType'];
  deviceModel?: string;
  webAddress: string;
  localsendPort: number;
  protocol: string;
  multicastIpv4: string;
  multicastIpv6: string;
  dataDirectory: string;
  autoAccept: boolean;
  discoveryIntervalSeconds: number;
  maxUploadBytes: number;
  uptimeSeconds: number;
  nearbyDevices: number;
}

export interface EnvironmentSettings {
  autoAccept: boolean;
  alias: string;
  aliasLocale: AliasLocale;
}

export interface StorageSettings {
  root: string;
  subdirectory: string;
  resolvedPath: string;
}

export interface DirectoryListing {
  path: string;
  parent?: string;
  directories: string[];
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
  coveredBy?: string;
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
  transferredBytes: number;
  status: 'preparing' | 'sending' | 'completed' | 'failed';
  createdAt: string;
  error?: string;
  contentType?: string;
  isClipboard: boolean;
}

export interface IncomingTransfer {
  id: string;
  sessionId: string;
  fileId: string;
  senderAlias: string;
  fileName: string;
  totalBytes: number;
  transferredBytes: number;
  status: 'waiting' | 'receiving' | 'completed' | 'failed';
  createdAt: string;
  error?: string;
}

export interface SendTargetResult {
  transferId: string;
  targetAlias: string;
  filesSent: number;
  totalBytes: number;
  success: boolean;
  error?: string;
}

export interface SendResponse {
  transferId: string;
  transfers: SendTargetResult[];
  filesSent: number;
  totalBytes: number;
}

export interface LinkShareFile {
  id: string;
  name: string;
  size: number;
}

export interface LinkShareRequest {
  sessionId: string;
  ip: string;
  userAgent?: string;
  status: 'pending' | 'accepted';
  createdAt: string;
}

export interface LinkShare {
  active: boolean;
  shareId?: string;
  urls: string[];
  files: LinkShareFile[];
  totalBytes: number;
  autoAccept: boolean;
  pin?: string;
  requests: LinkShareRequest[];
  createdAt?: string;
}
