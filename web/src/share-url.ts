import type { NetworkInterfaceInfo } from './types';

export function browserShareUrl(location: string): string {
  return new URL('/share', location).toString();
}

export function interfaceShareUrl(addressWithPrefix: string, port: number): string {
  const address = addressWithPrefix.split('/')[0];
  const host = address.includes(':') ? `[${address}]` : address;
  return `http://${host}:${port}/share`;
}

export function shareAddressForInterface(network: NetworkInterfaceInfo): string | undefined {
  return network.ipv4Addresses[0]
    ?? network.ipv6Addresses.find((address) => !address.toLowerCase().startsWith('fe80:'));
}
