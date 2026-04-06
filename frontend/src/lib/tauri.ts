import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { get } from 'svelte/store';
import type { Settings, Transfer, IncomingOffer, Device, DiscoveredDevice } from './types';
import { transfers, incomingOffers, devices, localIp, settings, discoveredDevices } from './stores';
export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export async function loadDiscoveredDevices(): Promise<void> {
  if (!isTauri) {
    discoveredDevices.set([]);
    return;
  }
  const list = await invoke<DiscoveredDevice[]>('get_discovered_devices');
  discoveredDevices.set(list);
}

export async function loadSettings(): Promise<void> {
  if (!isTauri) return;
  try {
    const s = await invoke<Settings>('get_settings');
    settings.set(s);
  } catch (e) {
    console.warn("Failed to load settings:", e);
  }
}

export async function saveSettings(s: Settings): Promise<void> {
  if (!isTauri) {
    settings.set(s);
    return;
  }
  await invoke('save_settings', { settings: s });
  settings.set(s);
}

export async function loadLocalIp(): Promise<void> {
  if (!isTauri) {
    localIp.set('127.0.0.1');
    return;
  }
  try {
    const ip = await invoke<string>('get_local_ip');
    localIp.set(ip);
  } catch {
    localIp.set('unavailable');
  }
}

export async function sendFileCmd(deviceId: string, filePath: string): Promise<string> {
  if (!isTauri) {
    return `mock-${Date.now()}`;
  }
  return await invoke<string>('send_file_cmd', { targetId: deviceId, filePath });
}

export async function loadTransfers(): Promise<void> {
  if (!isTauri) {
    transfers.set([]);
    return;
  }
  const list = await invoke<Transfer[]>('get_transfers');
  transfers.set(list);
}

export async function loadDevices(): Promise<void> {
  if (!isTauri) {
    devices.set([]);
    return;
  }
  const list = await invoke<Device[]>('get_devices');
  devices.set(list);
}

export async function removeDevice(deviceId: string): Promise<void> {
  if (!isTauri) {
    devices.update(d => d.filter(x => x.id !== deviceId));
    return;
  }
  await invoke('remove_device', { deviceId });
  devices.update(d => d.filter(x => x.id !== deviceId));
}

export async function acceptTransfer(transferId: string): Promise<void> {
  if (!isTauri) {
    incomingOffers.update(o => o.filter(x => x.id !== transferId));
    return;
  }
  await invoke('accept_transfer', { transferId });
  incomingOffers.update(o => o.filter(x => x.id !== transferId));
}

export async function rejectTransfer(transferId: string): Promise<void> {
  if (!isTauri) {
    incomingOffers.update(o => o.filter(x => x.id !== transferId));
    return;
  }
  await invoke('reject_transfer', { transferId });
  incomingOffers.update(o => o.filter(x => x.id !== transferId));
}

export async function trustAndAcceptTransfer(transferId: string): Promise<void> {
  if (!isTauri) {
    incomingOffers.update(o => o.filter(x => x.id !== transferId));
    return;
  }
  await invoke('trust_and_accept_transfer', { transferId });
  incomingOffers.update(o => o.filter(x => x.id !== transferId));
}

export async function openDownloads(): Promise<void> {
  const { download_dir } = get(settings);
  if (!isTauri) {
    alert("Would open downloads folder: " + download_dir);
    return;
  }
  const { openPath } = await import('@tauri-apps/plugin-opener');
  await openPath(download_dir);
}

export async function pickFile(): Promise<string | null> {
  if (!isTauri) {
    return new Promise(resolve => {
      const input = document.createElement('input');
      input.type = 'file';
      input.onchange = (e: any) => {
        const file = e.target.files?.[0];
        resolve(file ? file.name : null);
      };
      input.click();
    });
  }
  try {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const result = await open({ multiple: false, directory: false });
    return typeof result === 'string' ? result : null;
  } catch {
    return null;
  }
}

export async function pickDirectory(): Promise<string | null> {
  if (!isTauri) {
    return "/mock/directory/path";
  }
  try {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const result = await open({ multiple: false, directory: true });
    return typeof result === 'string' ? result : null;
  } catch {
    return null;
  }
}

/** Wire up backend → UI event listeners. Call once at app startup. */
export function setupListeners(): void {
  if (!isTauri) return;

  listen<Transfer>('transfer-progress', (event) => {
    transfers.update(list => {
      const idx = list.findIndex(t => t.id === event.payload.id);
      if (idx >= 0) {
        const copy = [...list];
        copy[idx] = event.payload;
        return copy;
      }
      return [event.payload, ...list];
    });
  }).catch(err => console.warn("Tauri event listener failed:", err));

  listen<Transfer>('transfer-complete', (event) => {
    transfers.update(list => {
      const idx = list.findIndex(t => t.id === event.payload.id);
      if (idx < 0) return list;
      const copy = [...list];
      copy[idx] = { ...event.payload, status: 'done' };
      return copy;
    });
  }).catch(err => console.warn("Tauri event listener failed:", err));

  listen<Transfer>('transfer-error', (event) => {
    transfers.update(list => {
      const idx = list.findIndex(t => t.id === event.payload.id);
      if (idx < 0) return list;
      const copy = [...list];
      copy[idx] = { ...event.payload, status: 'error' };
      return copy;
    });
  }).catch(err => console.warn("Tauri event listener failed:", err));

  listen<IncomingOffer>('incoming-offer', (event) => {
    incomingOffers.update(o => [event.payload, ...o]);
  }).catch(err => console.warn("Tauri event listener failed:", err));
}
