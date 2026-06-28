<script lang="ts">
  import { onMount } from "svelte";
  import {
    loadSettings,
    loadLocalIp,
    setupListeners,
    acceptTransfer,
    rejectTransfer,
    trustAndAcceptTransfer
  } from "$lib/tauri";
  import { incomingOffers, toasts } from "$lib/stores";
  import { formatBytes } from "$lib/utils";
  import "../app.css";
  import "./layout.css";

  let { children } = $props();

  onMount(async () => {
    try {
      await Promise.all([loadSettings(), loadLocalIp()]);
    } catch (e) {
      console.warn("Tauri API failed to load state:", e);
    }
    setupListeners();
  });
</script>

{@render children()}

<!-- Global transient toasts (clipboard received/sent, etc.) -->
{#if $toasts.length > 0}
  <div class="toast-stack">
    {#each $toasts as toast (toast.id)}
      <div class="toast">
        <span class="material-symbols-outlined" style="font-size:18px">content_paste</span>
        <span>{toast.message}</span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .toast-stack {
    position: fixed;
    bottom: 20px;
    right: 20px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    z-index: 1000;
  }
  .toast {
    display: flex;
    align-items: center;
    gap: 8px;
    background: #1c1c1c;
    color: #efefef;
    border: 1px solid #00e5ff44;
    box-shadow: 0 4px 24px rgba(0, 0, 0, 0.5);
    border-radius: 8px;
    padding: 10px 14px;
    font-size: 13px;
    animation: toast-in 0.2s ease;
  }
  @keyframes toast-in {
    from { opacity: 0; transform: translateY(8px); }
    to   { opacity: 1; transform: translateY(0); }
  }
</style>

