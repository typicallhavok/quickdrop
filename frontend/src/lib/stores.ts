import { writable } from 'svelte/store';
import type { Settings, Transfer, IncomingOffer, Device, DiscoveredDevice } from './types';

export const settings = writable<Settings>({
  local_name: '',
  download_dir: './downloads',
  run_in_tray: false,
  port: 52341,
});

export const localIp      = writable<string>('—');
export const transfers    = writable<Transfer[]>([]);
export const incomingOffers = writable<IncomingOffer[]>([]);
export const devices      = writable<Device[]>([]);
export const discoveredDevices = writable<DiscoveredDevice[]>([]);
