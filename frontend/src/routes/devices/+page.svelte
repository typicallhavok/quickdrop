<script lang="ts">
  import './page.css';
  import { onMount } from 'svelte';
  import { devices } from '$lib/stores';
  import { loadDevices, removeDevice } from '$lib/tauri';

  onMount(loadDevices);

  function truncKey(key: string): string {
    return key.length > 22 ? key.slice(0, 11) + '…' + key.slice(-11) : key;
  }

  function initials(name: string): string {
    return name.trim().slice(0, 2).toUpperCase() || '??';
  }
</script>

<div class="page">
  <div class="section-head">
    <h2>Trusted Devices</h2>
    {#if $devices.length > 0}
      <span class="count-pill">{$devices.length}</span>
    {/if}
  </div>

  {#if $devices.length === 0}
    <div class="empty-state">
      <svg width="44" height="44" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.2" aria-hidden="true">
        <path stroke-linecap="round" stroke-linejoin="round" d="M9 17.25v1.007a3 3 0 0 1-.879 2.122L7.5 21h9l-.621-.621A3 3 0 0 1 15 18.257V17.25m6-12V15a2.25 2.25 0 0 1-2.25 2.25H5.25A2.25 2.25 0 0 1 3 15V5.25A2.25 2.25 0 0 1 5.25 3h13.5A2.25 2.25 0 0 1 21 5.25z" />
      </svg>
      <p>No trusted devices<br />Devices you pair with will appear here</p>
    </div>
  {:else}
    <div class="device-list">
      {#each $devices as d (d.id)}
        <div class="d-card fade-up">
          <div class="avatar" aria-hidden="true">{initials(d.name)}</div>
          <div class="d-info">
            <span class="d-name">{d.name}</span>
            <span class="d-key">{truncKey(d.public_key)}</span>
          </div>
          <button class="btn btn-danger btn-sm" onclick={() => removeDevice(d.id)}>Remove</button>
        </div>
      {/each}
    </div>
  {/if}
</div>
