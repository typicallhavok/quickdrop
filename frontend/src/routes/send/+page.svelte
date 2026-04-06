<script lang="ts">
  import "./page.css";
  import { pickFile, sendFileCmd, isTauri, loadDiscoveredDevices } from "$lib/tauri";
  import { transfers, discoveredDevices } from "$lib/stores";
  import { formatBytes } from "$lib/utils";
  import type { Transfer } from "$lib/types";
  import { listen } from "@tauri-apps/api/event";
  import type { UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  let filePath = $state<string | null>(null);
  let fileName = $state("");
  let fileSize = $state(0);
  let targetDeviceId = $state("");
  let sending = $state(false);
  let isDragOver = $state(false);
  let error = $state("");
  let success = $state("");

  onMount(() => {
    loadDiscoveredDevices();
    const interval = setInterval(loadDiscoveredDevices, 3000);
    return () => clearInterval(interval);
  });

  async function browse() {
    const path = await pickFile();
    if (path) {
      filePath = path;
      const parts = path.replace(/\\/g, "/").split("/");
      fileName = parts[parts.length - 1];
      fileSize = 0;
      error = "";
    }
  }

  function clearFile(e: Event) {
    e.stopPropagation();
    filePath = null;
    fileName = "";
    fileSize = 0;
    error = "";
  }

  function onDragover(e: DragEvent) {
    e.preventDefault();
  }
  function onDragleave() {}
  function onDrop(e: DragEvent) {
    e.preventDefault();
  }

  $effect(() => {
    let unlistenDrop: UnlistenFn;
    let unlistenEnter: UnlistenFn;
    let unlistenLeave: UnlistenFn;

    const setup = async () => {
      if (!isTauri) return;
      try {
        unlistenDrop = await listen<{ paths: string[] }>(
          "tauri://drag-drop",
          (e) => {
            isDragOver = false;
            if (e.payload.paths?.length > 0) {
              const path = e.payload.paths[0];
              filePath = path;
              const parts = path.replace(/\\/g, "/").split("/");
              fileName = parts[parts.length - 1];
              fileSize = 0;
              error = "";
            }
          },
        );
        unlistenEnter = await listen(
          "tauri://drag-enter",
          () => (isDragOver = true),
        );
        unlistenLeave = await listen(
          "tauri://drag-leave",
          () => (isDragOver = false),
        );
      } catch (err) {
        console.warn(
          "Tauri drag-and-drop features are unavailable outside a Tauri webview:",
          err,
        );
      }
    };

    setup();

    return () => {
      if (unlistenDrop) unlistenDrop();
      if (unlistenEnter) unlistenEnter();
      if (unlistenLeave) unlistenLeave();
    };
  });

  async function send() {
    if (!filePath || !targetDeviceId) return;
    sending = true;
    error = "";
    success = "";
    try {
      const id = await sendFileCmd(targetDeviceId, filePath);
      const selectedDeviceName = $discoveredDevices.find(d => d.id === targetDeviceId)?.name || targetDeviceId;
      const t: Transfer = {
        id,
        file_name: fileName,
        file_size: fileSize,
        bytes_done: 0,
        direction: "send",
        peer_name: selectedDeviceName,
        peer_ip: "", // Optional, can be removed if not needed anymore
        status: "pending",
      };
      transfers.update((l) => [t, ...l]);
      success = "Transfer queued";
      filePath = null;
      fileName = "";
      fileSize = 0;
      targetDeviceId = "";
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      sending = false;
    }
  }

  $effect(() => {
    if (!success) return;
    const id = setTimeout(() => {
      success = "";
    }, 3000);
    return () => clearTimeout(id);
  });

  const canSend = $derived(!!filePath && !!targetDeviceId && !sending);
</script>

<div class="page">
  <div class="section-head">
    <h2>Send File</h2>
  </div>

  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="dropzone"
    class:has-file={!!filePath}
    class:drag-over={isDragOver}
    role="button"
    tabindex="0"
    ondrop={onDrop}
    ondragover={onDragover}
    ondragleave={onDragleave}
    onclick={() => !filePath && browse()}
    onkeydown={(e) => e.key === "Enter" && !filePath && browse()}
  >
    {#if filePath}
      <div class="file-row">
        <div class="file-icon" aria-hidden="true">
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              d="M19.5 14.25v-2.625a3.375 3.375 0 0 0-3.375-3.375h-1.5A1.125 1.125 0 0 1 13.5 7.125v-1.5a3.375 3.375 0 0 0-3.375-3.375H8.25m2.25 0H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 0 0-9-9Z"
            />
          </svg>
        </div>
        <div class="file-info">
          <span class="file-name">{fileName}</span>
          {#if fileSize > 0}<span class="file-size"
              >{formatBytes(fileSize)}</span
            >{/if}
        </div>
        <button class="clear-btn" onclick={clearFile} aria-label="Remove file">
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            aria-hidden="true"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              d="M6 18 18 6M6 6l12 12"
            />
          </svg>
        </button>
      </div>
    {:else}
      <div class="drop-prompt">
        <div class="drop-icon" aria-hidden="true">
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.3"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              d="M3 16.5v2.25A2.25 2.25 0 0 0 5.25 21h13.5A2.25 2.25 0 0 0 21 18.75V16.5m-13.5-9L12 3m0 0 4.5 4.5M12 3v13.5"
            />
          </svg>
        </div>
        <p class="drop-main">
          {isDragOver ? "Release to select" : "Drop a file here"}
        </p>
        <p class="drop-hint">
          or <span class="link-text">click to browse</span>
        </p>
      </div>
    {/if}
  </div>

  <!-- Target Device -->
  <div class="field" style="margin-top: 16px;">
    <label for="target-device">Select Device</label>
    <select
      id="target-device"
      bind:value={targetDeviceId}
    >
      <option value="" disabled selected>Select a nearby device...</option>
      {#each $discoveredDevices as device}
        <option value={device.id}>{device.name}</option>
      {/each}
    </select>
    {#if $discoveredDevices.length === 0}
      <small style="display: block; margin-top: 8px; color: var(--text-2);">Scanning for nearby devices...</small>
    {/if}
  </div>

  <!-- Actions -->
  <div class="action-row">
    <button class="btn btn-primary btn-lg" disabled={!canSend} onclick={send}>
      {#if sending}
        <svg
          class="spin-anim"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          style="width:16px;height:16px"
          aria-hidden="true"
        >
          <path
            stroke-linecap="round"
            d="M12 2v4m0 12v4M4.93 4.93l2.83 2.83m8.48 8.48 2.83 2.83M2 12h4m12 0h4M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83"
          />
        </svg>
        Sending…
      {:else}
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          aria-hidden="true"
        >
          <path
            stroke-linecap="round"
            stroke-linejoin="round"
            d="M3 16.5v2.25A2.25 2.25 0 0 0 5.25 21h13.5A2.25 2.25 0 0 0 21 18.75V16.5m-13.5-9L12 3m0 0 4.5 4.5M12 3v13.5"
          />
        </svg>
        Send
      {/if}
    </button>
    {#if error}
      <span class="msg err">{error}</span>
    {/if}
    {#if success}<span class="msg ok">{success}</span>{/if}
  </div>
</div>
