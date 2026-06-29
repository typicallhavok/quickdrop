<script lang="ts">
  import './page.css';
  import { onMount, onDestroy } from 'svelte';
  import { fade } from 'svelte/transition';
  import { discoveredDevices, devices, transfers, incomingOffers } from '$lib/stores';
  import { formatBytes, pct } from '$lib/utils';

  import { open } from '@tauri-apps/plugin-dialog';
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { loadDiscoveredDevices, sendFileCmd, revealInFolder, sendClipboard, pushToast } from '$lib/tauri';

  interface SelectedFile {
    name: string;
    path: string;
    size: number;
  }

  let selectedFiles: SelectedFile[] = $state([]);
  let interval: ReturnType<typeof setInterval>;
  let isDragging = $state(false);
  let unlistenDragDrop: () => void;

  onMount(async () => {
    loadDiscoveredDevices();
    interval = setInterval(loadDiscoveredDevices, 3000);

    unlistenDragDrop = await getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === 'over' || event.payload.type === 'enter') {
        isDragging = true;
      } else if (event.payload.type === 'leave') {
        isDragging = false;
      } else if (event.payload.type === 'drop') {
        isDragging = false;
        const paths = event.payload.paths;
        if (paths && paths.length > 0) {
          selectedFiles = [
            ...selectedFiles,
            ...paths.map(p => {
              const name = p.split(/[/\\]/).pop() || 'Unknown';
              return { name, path: p, size: 0 };
            })
          ];
        }
      }
    });
  });

  onDestroy(() => {
    if (interval) clearInterval(interval);
    if (unlistenDragDrop) unlistenDragDrop();
  });

  function getDeviceIcon(name: string) {
    const n = name.toLowerCase();
    if (n.includes('phone') || n.includes('pixel') || n.includes('iphone') || n.includes('android')) return 'smartphone';
    if (n.includes('mac') || n.includes('windows') || n.includes('desktop')) return 'desktop_windows';
    return 'laptop';
  }

  async function openFilePicker() {
    try {
      const selected = await open({
        multiple: true,
        directory: false,
      });
      if (!selected) return;
      if (Array.isArray(selected)) {
        selectedFiles = [
          ...selectedFiles,
          ...selected.map((p: string) => {
            const name = p.split(/[/\\]/).pop() || 'Unknown';
            return { name, path: p, size: 0 };
          })
        ];
      } else {
        const p = selected as string;
        const name = p.split(/[/\\]/).pop() || 'Unknown';
        selectedFiles = [...selectedFiles, { name, path: p, size: 0 }];
      }
    } catch (e) {
      console.error(e);
    }
  }

  function removeFile(index: number) {
    selectedFiles = selectedFiles.filter((_, i) => i !== index);
  }

  /** Get the active/done transfers for a device */
  function getDeviceTransfers(deviceId: string) {
    const list = $transfers;
    // Match by peer_ip or peer_name containing the device ID
    return list.filter(t => (t.peer_ip === deviceId || t.peer_name === deviceId));
  }

  async function sendToDevice(deviceId: string) {
    if (selectedFiles.length === 0) return;
    const promises = selectedFiles.map(f => 
      sendFileCmd(deviceId, f.path).catch(err => {
        console.error("Failed to send file:", err);
      })
    );
    selectedFiles = []; // Clear after initiating transfers
    await Promise.all(promises);
  }

  async function resolveOffer(id: string, action: 'Accept' | 'Reject' | 'TrustAndAccept') {
    incomingOffers.update(list => list.filter(o => o.id !== id));
    if (action === 'Accept') {
      await invoke('accept_transfer', { transferId: id });
    } else if (action === 'Reject') {
      await invoke('reject_transfer', { transferId: id });
    } else if (action === 'TrustAndAccept') {
      await invoke('trust_and_accept_transfer', { transferId: id });
    }
  }

  async function cancelTransfer(id: string) {
    try {
      await invoke('cancel_transfer', { transferId: id });
    } catch (e) {
      console.error("Failed to cancel transfer", e);
    }
  }

  async function sendClipboardToDevice(deviceId: string) {
    try {
      await sendClipboard(deviceId);
    } catch (e) {
      console.error("Failed to send clipboard", e);
      pushToast('Failed to send clipboard');
    }
  }

  /** Transfers shown in the Activity panel: everything not already shown inline
   *  on a discovered-device card (i.e. incoming/received transfers, and sends to
   *  devices that have gone offline). Most recent first. */
  const activityTransfers = $derived(
    $transfers.filter(t => !$discoveredDevices.some(d => d.id === t.peer_ip || d.id === t.peer_name))
  );
</script>

<header class="bg-[#0E0E0E] flex justify-between items-center w-full px-6 h-16 z-50">
<div class="text-[1.375rem] font-bold text-[#E5E2E1] tracking-tight">Quickdrop</div>
<div class="flex items-center gap-4">
<button class="text-[#BAC9CC] hover:bg-[#2A2A2A] transition-colors duration-200 p-2 rounded-lg active:scale-95 transition-transform" onclick={() => window.location.href='/settings'}>
<span class="material-symbols-outlined" data-icon="settings">settings</span>
</button>
</div>
</header>
<!-- Main Content Area: The Monolithic Transfer Split -->
<main class="flex-1 flex overflow-hidden">
<!-- Left Side: The Vault (Files) -->
<section class="w-1/3 bg-surface-container-lowest flex flex-col p-10 border-none">
{#if $incomingOffers.length > 0}
  <h2 class="text-[1.375rem] font-bold mb-4 tracking-tight text-on-surface">Incoming</h2>
  <div class="mb-8 space-y-4">
    {#each $incomingOffers as offer}
      <div class="bg-surface-container p-4 rounded-lg flex flex-col hover:bg-[rgba(255,255,255,0.02)] transition-colors border border-outline-variant/10">
        <span class="text-sm font-semibold tracking-wide text-[#00E5FF] truncate">{offer.file_name}</span>
        <span class="text-[0.6875rem] tracking-widest uppercase text-on-surface-variant mb-4 mt-1">{formatBytes(offer.file_size)} from {offer.peer_name}</span>
        <div class="flex items-center gap-2">
          <button class="bg-[#00E5FF] text-[#0E0E0E] text-xs shadow-[0_0_10px_rgba(0,229,255,0.2)] font-bold px-3 py-1.5 rounded hover:bg-[#00E5FF]/80 transition-all flex-1" onclick={() => resolveOffer(offer.id, 'Accept')}>Accept</button>
          <button class="bg-surface-container-highest text-on-surface text-[0.6875rem] font-bold px-3 py-1.5 rounded hover:bg-surface-container-high transition-colors flex-1" onclick={() => resolveOffer(offer.id, 'TrustAndAccept')}>Always Trust</button>
          <button class="border border-outline-variant/30 text-error text-[0.6875rem] font-bold px-3 py-1.5 rounded hover:bg-error/10 hover:border-error/50 transition-colors flex-1" onclick={() => resolveOffer(offer.id, 'Reject')}>Reject</button>
        </div>
      </div>
    {/each}
  </div>
{/if}

<h2 class="text-[1.375rem] font-bold mb-8 tracking-tight text-on-surface">Selected Files</h2>
<div class="flex-1 space-y-6 overflow-y-auto">
{#if selectedFiles.length === 0 && $incomingOffers.length === 0}
  <div class="text-[0.875rem] text-on-surface-variant italic">No files selected.</div>
{/if}
{#each selectedFiles as file, index}
  <!-- File Entry -->
  <div class="group cursor-pointer flex justify-between items-center bg-surface-container p-3 rounded hover:bg-surface-container-highest">
    <div class="truncate mr-4">
      <div class="text-[0.875rem] tracking-wider text-on-surface font-medium truncate">{file.name}</div>
      <div class="text-[0.6875rem] tracking-widest uppercase text-on-surface-variant mt-1">{file.path}</div>
    </div>
    <button class="text-error hover:text-error-container" onclick={() => removeFile(index)}>
      <span class="material-symbols-outlined" data-icon="close">close</span>
    </button>
  </div>
{/each}

{#if activityTransfers.length > 0}
  <h2 class="text-[1.375rem] font-bold mt-8 mb-4 tracking-tight text-on-surface">Activity</h2>
  <div class="space-y-3">
    {#each activityTransfers as transfer (transfer.id)}
      <div class="bg-surface-container p-3 rounded border border-outline-variant/10" out:fade={{ duration: 400 }}>
        <div class="flex items-center justify-between mb-2 gap-2">
          <span class="text-[0.8125rem] font-medium text-on-surface truncate">{transfer.file_name}</span>
          {#if transfer.status === 'active'}
            <div class="flex items-center gap-2 shrink-0">
              <span class="text-[#00E5FF] text-xs font-bold">{pct(transfer.bytes_done, transfer.file_size)}%</span>
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <!-- svelte-ignore a11y_interactive_supports_focus -->
              <div role="button" class="text-error hover:bg-error/20 p-1 rounded-full transition-colors flex items-center" title="Cancel Transfer" onclick={() => cancelTransfer(transfer.id)}>
                <span class="material-symbols-outlined" style="font-size: 16px;">close</span>
              </div>
            </div>
          {:else if transfer.status === 'done'}
            <button class="flex items-center gap-1 text-[#00E5FF] text-[0.6875rem] font-bold hover:underline shrink-0" title="Open containing folder" onclick={() => revealInFolder(transfer.direction === 'receive' ? transfer.file_name : undefined)}>
              <span class="material-symbols-outlined" style="font-size: 14px;">folder_open</span>
              Open
            </button>
          {:else if transfer.status === 'error'}
            <span class="device-transfer-error shrink-0">
              <span class="material-symbols-outlined" style="font-size: 14px;">error</span>
              Failed
            </span>
          {:else if transfer.status === 'cancelled'}
            <span class="device-transfer-error shrink-0" style="opacity:0.7">
              <span class="material-symbols-outlined" style="font-size: 14px;">cancel</span>
              Cancelled
            </span>
          {/if}
        </div>
        <div class="progress-track">
          <div
            class="progress-fill {transfer.direction === 'receive' ? 'recv' : ''} {transfer.status === 'done' ? 'done' : ''} {transfer.status === 'error' ? 'error' : ''} {transfer.status === 'cancelled' ? 'cancelled' : ''}"
            style="width: {transfer.status === 'done' || transfer.status === 'error' || transfer.status === 'cancelled' ? 100 : pct(transfer.bytes_done, transfer.file_size)}%"
          ></div>
        </div>
        <div class="flex justify-between items-center w-full mt-2">
          <span class="text-[0.6875rem] uppercase tracking-widest text-on-surface-variant">
            {transfer.direction === 'receive' ? 'From' : 'To'} {transfer.peer_name}
          </span>
          {#if transfer.status === 'active' && transfer.speed_bps}
            <span class="text-xs font-bold text-[#00E5FF]">{formatBytes(transfer.speed_bps)}/s</span>
          {:else}
            <span class="text-[0.6875rem] text-on-surface-variant">{formatBytes(transfer.bytes_done)} / {formatBytes(transfer.file_size)}</span>
          {/if}
        </div>
      </div>
    {/each}
  </div>
{/if}
</div>
<!-- Drop Zone Area -->
<div class="mt-auto pt-10">
<button class="w-full h-40 flex flex-col items-center justify-center rounded transition-all duration-200 border-dashed border-2 {isDragging ? 'bg-primary-container/20 border-primary-container' : 'bg-surface-container-high border-outline-variant/20 hover:bg-surface-container-highest'}" onclick={openFilePicker}>
<span class="material-symbols-outlined {isDragging ? 'text-primary' : 'text-primary-container'} mb-3" data-icon="upload_file">upload_file</span>
<span class="text-[0.6875rem] tracking-widest uppercase {isDragging ? 'text-primary font-bold' : 'text-on-surface-variant'}">{isDragging ? 'Drop Files Here' : 'Click or Drag Files'}</span>
</button>
</div>
</section>
<!-- Right Side: The Canvas (Devices) -->
<section class="flex-1 bg-surface-container flex flex-col p-10">
<h2 class="text-[1.375rem] font-bold mb-8 tracking-tight text-on-surface">Devices</h2>
<div class="grid grid-cols-1 md:grid-cols-2 gap-4">
{#if $discoveredDevices.length === 0 && $devices.length === 0}
  <div class="col-span-full h-32 flex flex-col items-center justify-center opacity-50">
    <span class="material-symbols-outlined text-3xl mb-2">wifi_tethering</span>
    <span class="text-[0.875rem] tracking-wider text-on-surface font-medium">Looking for nearby devices...</span>
  </div>
{/if}
{#each $discoveredDevices as device (device.id)}
  <!-- Device Card. A clickable <div>, not a <button>: it contains nested
       interactive controls (clipboard, cancel) which are invalid inside a
       <button>, and a disabled <button> would swallow their clicks. Sending is
       guarded in sendToDevice (no-op when no files are selected). -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    role="button"
    tabindex="0"
    class="device-card bg-surface-container-high rounded flex flex-col hover:bg-surface-container-highest transition-colors cursor-pointer group w-full text-left"
    onclick={() => sendToDevice(device.id)}
    class:opacity-50={selectedFiles.length === 0}
  >
    <div class="flex items-center justify-between p-6 w-full">
      <div class="flex items-center gap-5">
        <div class="w-12 h-12 bg-surface-container-lowest flex items-center justify-center rounded">
          <span class="material-symbols-outlined text-on-surface-variant" data-icon={getDeviceIcon(device.name)}>{getDeviceIcon(device.name)}</span>
        </div>
        <div>
          <div class="text-[0.875rem] tracking-wider text-on-surface font-bold">{device.name}</div>
          <div class="text-[0.6875rem] tracking-widest uppercase text-[#00E5FF] mt-0.5">Nearby</div>
        </div>
      </div>
      <div class="flex items-center gap-1">
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_interactive_supports_focus -->
        <div
          role="button"
          title="Send clipboard text to this device"
          class="text-on-surface-variant hover:text-[#00E5FF] hover:bg-[#00E5FF]/10 p-2 rounded-lg transition-colors flex items-center"
          onclick={(e) => { e.stopPropagation(); sendClipboardToDevice(device.id); }}
        >
          <span class="material-symbols-outlined" style="font-size: 20px;">content_paste_go</span>
        </div>
        {#if getDeviceTransfers(device.id).length === 0}
          <span class="material-symbols-outlined text-primary-container opacity-0 group-hover:opacity-100 transition-opacity">
            send
          </span>
        {/if}
      </div>
    </div>
    <!-- Transfer Progress Area -->
    {#each getDeviceTransfers(device.id).slice(0, 5) as transfer (transfer.id)}
      <div class="device-transfer-status px-6 pb-5 w-full border-t border-outline-variant/10 pt-4" out:fade={{ duration: 400 }}>
        <div class="flex items-center justify-between mb-2">
          <span class="device-transfer-name truncate mr-4">{transfer.file_name}</span>
          {#if transfer.status === 'active'}
            <div class="flex items-center gap-2">
              <span class="device-transfer-pct text-[#00E5FF]">{pct(transfer.bytes_done, transfer.file_size)}%</span>
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <!-- svelte-ignore a11y_interactive_supports_focus -->
              <div role="button" class="text-error hover:bg-error/20 p-1 rounded-full transition-colors flex items-center" title="Cancel Transfer" onclick={(e) => { e.stopPropagation(); cancelTransfer(transfer.id); }}>
                <span class="material-symbols-outlined" style="font-size: 16px;">close</span>
              </div>
            </div>
          {:else if transfer.status === 'done'}
            <span class="device-transfer-done">
              <span class="material-symbols-outlined" style="font-size: 14px;">check_circle</span>
              Sent
            </span>
          {:else if transfer.status === 'error'}
            <span class="device-transfer-error">
              <span class="material-symbols-outlined" style="font-size: 14px;">error</span>
              Failed
            </span>
          {:else if transfer.status === 'cancelled'}
            <span class="device-transfer-error" style="opacity:0.7">
              <span class="material-symbols-outlined" style="font-size: 14px;">cancel</span>
              Cancelled
            </span>
          {/if}
        </div>
        <div class="progress-track">
          <div
            class="progress-fill {transfer.direction === 'receive' ? 'recv' : ''} {transfer.status === 'done' ? 'done' : ''} {transfer.status === 'error' ? 'error' : ''} {transfer.status === 'cancelled' ? 'cancelled' : ''}"
            style="width: {transfer.status === 'done' || transfer.status === 'error' || transfer.status === 'cancelled' ? 100 : pct(transfer.bytes_done, transfer.file_size)}%"
          ></div>
        </div>
        {#if transfer.status === 'active'}
          <div class="flex justify-between items-center w-full mt-2">
            <span class="device-transfer-bytes text-xs text-on-surface-variant font-medium tracking-wide">
              {formatBytes(transfer.bytes_done)} / {formatBytes(transfer.file_size)}
            </span>
            {#if transfer.speed_bps}
              <span class="text-xs font-bold text-[#00E5FF] tracking-wider">
                {formatBytes(transfer.speed_bps)}/s
              </span>
            {/if}
          </div>
        {/if}
      </div>
    {/each}
  </div>
{/each}
</div>
</section>
</main>
<!-- BottomNavBar (Mobile Only Anchor) -->
<nav class="md:hidden fixed bottom-0 left-0 w-full z-50 flex justify-around items-center h-20 bg-[#0E0E0E]">
<div class="flex flex-col items-center justify-center text-[#00E5FF] font-bold">
<span class="material-symbols-outlined" data-icon="folder_open" style="font-variation-settings: 'FILL' 1;">folder_open</span>
<span class="font-['Inter'] text-[0.6875rem] tracking-widest uppercase mt-1">Vault</span>
</div>
<div class="flex flex-col items-center justify-center text-[#BAC9CC]">
<span class="material-symbols-outlined" data-icon="swap_horiz">swap_horiz</span>
<span class="font-['Inter'] text-[0.6875rem] tracking-widest uppercase mt-1">Transfer</span>
</div>
<div class="flex flex-col items-center justify-center text-[#BAC9CC]">
<span class="material-symbols-outlined" data-icon="history">history</span>
<span class="font-['Inter'] text-[0.6875rem] tracking-widest uppercase mt-1">Activity</span>
</div>
</nav>
