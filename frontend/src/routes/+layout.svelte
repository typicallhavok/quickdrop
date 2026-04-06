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
  import { incomingOffers } from "$lib/stores";
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

