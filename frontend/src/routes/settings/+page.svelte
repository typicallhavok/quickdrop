<script lang="ts">
  import { onMount } from 'svelte';
  import { settings, devices } from '$lib/stores';
  import { saveSettings, pickDirectory, loadDevices, removeDevice } from '$lib/tauri';
  import type { Settings } from '$lib/types';

  let form = $state<Settings>({
    local_name: '',
    download_dir: './downloads',
    run_in_tray: false,
    resume_transfers: true,
    port: 52341
  });

  let saving = $state(false);
  let saved  = $state(false);
  let err    = $state('');

  onMount(() => {
    settings.subscribe(s => { form = { ...s }; });
    loadDevices();
  });

  async function browseDir() {
    const dir = await pickDirectory();
    if (dir) form.download_dir = dir;
  }

  async function save() {
    saving = true; err = '';
    try {
      await saveSettings({ ...form });
      saved = true;
      setTimeout(() => { saved = false; }, 2200);
    } catch (e: unknown) {
      err = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  async function doRemoveDevice(id: string) {
    await removeDevice(id);
  }
</script>

<div class="h-screen w-full bg-[#0E0E0E] text-on-surface flex flex-col font-['Inter'] relative">
  <!-- Header -->
  <header class="flex items-center gap-4 px-8 h-20 border-b border-surface-container-highest bg-[#0E0E0E] z-10 w-full">
    <button class="text-[#BAC9CC] hover:bg-surface-container p-2 rounded-full transition-colors" onclick={() => window.history.back()}>
      <span class="material-symbols-outlined">arrow_back</span>
    </button>
    <div class="text-[1.375rem] font-bold text-[#E5E2E1] tracking-tight">Settings</div>
  </header>

  <main class="flex-1 overflow-y-auto px-8 py-10 flex justify-center">
    <div class="w-full max-w-3xl space-y-12">
      <!-- Storage Section -->
      <section class="space-y-4">
        <h3 class="text-[#00E5FF] font-bold tracking-widest uppercase text-sm mb-6">Storage & Downloads</h3>
        <div class="bg-surface-container-lowest border border-outline-variant/10 rounded-xl p-6 shadow-xl backdrop-blur-sm">
          <div class="flex flex-col md:flex-row md:items-center justify-between gap-4">
            <div>
              <div class="text-[1rem] font-semibold text-on-surface">Download Folder</div>
              <div class="text-sm text-on-surface-variant mt-1">Default location for received files</div>
            </div>
            <div class="flex items-center gap-2 flex-1 md:max-w-md">
              <input type="text" bind:value={form.download_dir} class="flex-1 bg-surface-container-high border border-outline-variant/20 rounded-lg px-4 py-2 text-sm text-on-surface focus:border-[#00E5FF]/50 focus:outline-none transition-colors" spellcheck="false" />
              <button class="bg-[#00E5FF]/10 text-[#00E5FF] hover:bg-[#00E5FF]/20 px-4 py-2 rounded-lg text-sm font-semibold transition-colors" onclick={browseDir}>Browse</button>
            </div>
          </div>
        </div>
      </section>

      <!-- App Behavior Section -->
      <section class="space-y-4">
        <h3 class="text-[#00E5FF] font-bold tracking-widest uppercase text-sm mb-6">Application</h3>
        <div class="bg-surface-container-lowest border border-outline-variant/10 rounded-xl p-6 shadow-xl backdrop-blur-sm">
          <div class="flex items-center justify-between">
            <div>
              <div class="text-[1rem] font-semibold text-on-surface">Run in Background</div>
              <div class="text-sm text-on-surface-variant mt-1">Keep receiving files when window is closed</div>
            </div>
            <!-- Custom Toggle -->
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <!-- svelte-ignore a11y_interactive_supports_focus -->
            <div
              class="relative inline-flex h-6 w-11 items-center rounded-full transition-colors cursor-pointer {form.run_in_tray ? 'bg-[#00E5FF]' : 'bg-surface-container-highest'}"
              role="switch"
              aria-checked={form.run_in_tray}
              onclick={() => { form.run_in_tray = !form.run_in_tray; }}>
              <span class="inline-block h-4 w-4 transform rounded-full bg-[#0E0E0E] transition-transform {form.run_in_tray ? 'translate-x-6' : 'translate-x-1'}"></span>
            </div>
          </div>
        </div>

        <div class="bg-surface-container-lowest border border-outline-variant/10 rounded-xl p-6 shadow-xl backdrop-blur-sm mt-4">
          <div class="flex items-center justify-between">
            <div>
              <div class="text-[1rem] font-semibold text-on-surface">Resume Interrupted Transfers</div>
              <div class="text-sm text-on-surface-variant mt-1">Continue from where a cancelled transfer left off. When off, a new file is created with a (n) suffix instead.</div>
            </div>
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <!-- svelte-ignore a11y_interactive_supports_focus -->
            <div
              class="relative inline-flex h-6 w-11 items-center rounded-full transition-colors cursor-pointer {form.resume_transfers ? 'bg-[#00E5FF]' : 'bg-surface-container-highest'}"
              role="switch"
              aria-checked={form.resume_transfers}
              onclick={() => { form.resume_transfers = !form.resume_transfers; }}>
              <span class="inline-block h-4 w-4 transform rounded-full bg-[#0E0E0E] transition-transform {form.resume_transfers ? 'translate-x-6' : 'translate-x-1'}"></span>
            </div>
          </div>
        </div>
      </section>

      <!-- Trusted Devices Section -->
      <section class="space-y-4">
        <h3 class="text-[#00E5FF] font-bold tracking-widest uppercase text-sm mb-6">Trusted Devices</h3>
        <div class="bg-surface-container-lowest border border-outline-variant/10 rounded-xl shadow-xl backdrop-blur-sm overflow-hidden">
          {#if $devices.length === 0}
            <div class="p-8 text-center text-on-surface-variant text-sm flex flex-col items-center">
              <span class="material-symbols-outlined text-4xl mb-3 opacity-50">security</span>
              You haven't trusted any devices yet.
            </div>
          {:else}
            <div class="divide-y divide-outline-variant/10">
              {#each $devices as device}
                <div class="p-5 flex items-center justify-between hover:bg-[rgba(255,255,255,0.02)] transition-colors">
                  <div class="flex items-center gap-4">
                    <div class="w-10 h-10 bg-surface-container rounded flex items-center justify-center text-primary">
                      <span class="material-symbols-outlined">devices</span>
                    </div>
                    <div>
                      <div class="font-bold text-on-surface">{device.name}</div>
                      <div class="text-xs text-on-surface-variant font-mono mt-0.5 truncate max-w-xs">{device.public_key.substring(0, 16)}...</div>
                    </div>
                  </div>
                  <button class="border border-error/30 text-error hover:bg-error/10 px-3 py-1.5 rounded text-xs font-bold transition-all flex items-center gap-2" onclick={() => doRemoveDevice(device.id)}>
                    <span class="material-symbols-outlined" style="font-size: 14px">delete</span>
                    Revoke
                  </button>
                </div>
              {/each}
            </div>
          {/if}
        </div>
      </section>
    </div>
  </main>
  
  <!-- Footer with Save Action -->
  <footer class="border-t border-surface-container-highest bg-[#0E0E0E]/90 backdrop-blur-md p-6 flex items-center justify-end z-20">
    <div class="flex items-center gap-4">
      {#if saved}
         <span class="text-primary text-sm font-bold flex items-center gap-1 fade-up"><span class="material-symbols-outlined" style="font-size: 18px">check_circle</span> Settings Saved</span>
      {/if}
      {#if err}
         <span class="text-error text-sm max-w-xs truncate">{err}</span>
      {/if}
      <button class="bg-[#00E5FF] text-[#0E0E0E] shadow-[0_0_15px_rgba(0,229,255,0.3)] font-bold px-8 py-2.5 rounded hover:bg-[#00E5FF]/80 transition-all flex items-center gap-2" onclick={save} disabled={saving}>
        {#if saving}
          <span class="material-symbols-outlined animate-spin" style="font-size: 18px">sync</span> Saving...
        {:else}
          <span class="material-symbols-outlined" style="font-size: 18px">save</span> Save Changes
        {/if}
      </button>
    </div>
  </footer>
</div>

<style>
  @keyframes fadeUp {
    0% { opacity: 0; transform: translateY(5px); }
    100% { opacity: 1; transform: translateY(0); }
  }
  .fade-up {
    animation: fadeUp 0.3s ease-out forwards;
  }
</style>
