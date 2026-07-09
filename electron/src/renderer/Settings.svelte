<script lang="ts">
  import { onMount } from "svelte";
  import { SettingsForm } from "../shared/settingsForm";
  import { decideCapture, formatHotkey, type Modifiers } from "../shared/hotkey";
  import { FakeProvider, applyAutostart } from "../shared/autostart";
  import {
    DEFAULT_CONFIG,
    normalize,
    type Config,
  } from "../shared/config";

  // In the Electron Settings window the preload exposes `nudgeSettings`
  // (IPC-backed). In the browser sub-app it's absent → localStorage +
  // FakeProvider. The SettingsForm / recorder logic is identical either way.
  const settingsApi =
    typeof window !== "undefined" ? window.nudgeSettings : undefined;

  const STORAGE_KEY = "nudge-config";

  // Browser persistence. The Electron Settings window will swap these for IPC
  // calls (Layer B5); the pure SettingsForm/recorder logic stays identical.
  function loadStored(): Config {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { ...DEFAULT_CONFIG };
    try {
      return normalize(JSON.parse(raw));
    } catch {
      return { ...DEFAULT_CONFIG };
    }
  }
  function persistConfig(cfg: Config): void {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(cfg));
  }

  // `persisted` is the on-disk view. Autostart toggles write through it
  // immediately (transactional, no Save); Save writes the whole form.
  const initial = loadStored();
  let persisted = $state<Config>(initial);
  const provider = new FakeProvider(initial.autostart);
  const form = SettingsForm.fromConfig(initial);

  // Reactive mirrors of the form fields (Svelte 5 runes can't bind to a class
  // field directly, so we mirror and push changes back into the form).
  let hotkey = $state(form.hotkey);
  let intervalText = $state(form.intervalText);
  let autostart = $state(form.autostart);

  let recording = $state(false);
  let hotkeyBeforeRecord = "";
  let banner = $state("");

  // Electron loads its config over IPC (async), so seed the fields after mount.
  onMount(async () => {
    if (!settingsApi) return;
    const cfg = await settingsApi.getConfig();
    reseed(cfg);
  });

  function reseed(cfg: Config) {
    persisted = cfg;
    form.hotkey = cfg.hotkey;
    form.intervalText = SettingsForm.fromConfig(cfg).intervalText;
    form.autostart = cfg.autostart;
    hotkey = form.hotkey;
    intervalText = form.intervalText;
    autostart = form.autostart;
  }

  function syncToForm() {
    form.hotkey = hotkey;
    form.intervalText = intervalText;
    form.autostart = autostart;
  }

  function startRecording() {
    hotkeyBeforeRecord = hotkey;
    recording = true;
    banner = "Press a key combination…";
  }
  function stopRecording() {
    recording = false;
  }

  const MODIFIER_KEYS = new Set([
    "Control",
    "Alt",
    "Shift",
    "Meta",
    "OS",
    "AltGraph",
  ]);

  function onWindowKeydown(e: KeyboardEvent) {
    if (!recording) return;
    e.preventDefault();
    const mods: Modifiers = {
      ctrl: e.ctrlKey,
      alt: e.altKey,
      shift: e.shiftKey,
      win: e.metaKey,
    };
    const keys = MODIFIER_KEYS.has(e.key) ? [] : [e.key];
    const outcome = decideCapture(mods, keys);
    switch (outcome.kind) {
      case "captured":
        hotkey = formatHotkey(outcome.hotkey);
        banner = "";
        stopRecording();
        break;
      case "cancel":
        hotkey = hotkeyBeforeRecord;
        banner = "";
        stopRecording();
        break;
      case "unsupported":
        banner = "Unsupported key, try another";
        break;
      case "waiting":
        break;
    }
  }

  async function onAutostartChange(e: Event) {
    const desired = (e.target as HTMLInputElement).checked;
    // Transactional: flip the OS, confirm, then persist only the autostart bit
    // on top of the last-saved config — unsaved hotkey/interval edits stay
    // unsaved. On failure, revert the checkbox and surface the error.
    if (settingsApi) {
      const result = await settingsApi.toggleAutostart(desired);
      if (result.ok) {
        persisted = { ...persisted, autostart: desired };
        autostart = desired;
        form.autostart = desired;
        banner = "";
      } else {
        autostart = !desired; // revert
        banner = "Could not change autostart";
      }
      return;
    }
    const staged: Config = { ...persisted, autostart: desired };
    const result = applyAutostart(provider, desired, () => persistConfig(staged));
    if (result.ok) {
      persisted = staged;
      autostart = desired;
      form.autostart = desired;
      banner = "";
    } else {
      autostart = !desired; // revert
      banner =
        result.error.kind === "backend"
          ? "Could not change autostart"
          : "Autostart changed but saving failed";
    }
  }

  async function save() {
    syncToForm();
    const result = form.toConfig();
    if (!result.ok) {
      banner = "Interval must be a positive number of minutes";
      return;
    }
    if (settingsApi) {
      await settingsApi.save(result.config);
    } else {
      persistConfig(result.config);
    }
    persisted = result.config;
    form.markClean();
    hotkey = form.hotkey;
    intervalText = form.intervalText;
    banner = "Saved";
  }

  function cancel() {
    // Revert to the persisted view. In Electron, Cancel closes the window
    // (spec §9); in the browser sub-app there's no window, so we just reset.
    hotkey = persisted.hotkey;
    intervalText = SettingsForm.fromConfig(persisted).intervalText;
    autostart = persisted.autostart;
    recording = false;
    banner = "";
    settingsApi?.close();
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

<main class="min-h-screen bg-zinc-950 text-white p-8">
  <div class="max-w-md mx-auto space-y-6">
    <h1 class="text-xl font-semibold">Nudge — Settings</h1>

    <div class="space-y-2">
      <label class="block text-sm text-zinc-400" for="settings-hotkey">
        Global hotkey
      </label>
      <div class="flex gap-2">
        <input
          id="settings-hotkey"
          data-testid="settings-hotkey"
          type="text"
          bind:value={hotkey}
          readonly={recording}
          placeholder="Ctrl+Shift+Space"
          class="flex-1 bg-zinc-900 rounded px-3 py-2 outline-none border border-zinc-700"
        />
        <button
          data-testid="settings-hotkey-record"
          onclick={startRecording}
          class="px-3 py-2 rounded bg-zinc-800 hover:bg-zinc-700 text-sm"
        >
          {recording ? "Recording…" : "Record"}
        </button>
      </div>
    </div>

    <div class="space-y-2">
      <label class="block text-sm text-zinc-400" for="settings-interval">
        Default interval (minutes)
      </label>
      <input
        id="settings-interval"
        data-testid="settings-interval"
        type="text"
        inputmode="decimal"
        bind:value={intervalText}
        class="w-full bg-zinc-900 rounded px-3 py-2 outline-none border border-zinc-700"
      />
    </div>

    <label class="flex items-center gap-3 text-sm">
      <input
        data-testid="settings-autostart"
        type="checkbox"
        checked={autostart}
        onchange={onAutostartChange}
      />
      Launch at login
    </label>

    {#if banner}
      <p data-testid="settings-banner" class="text-sm text-amber-400">{banner}</p>
    {/if}

    <div class="flex gap-3 pt-2">
      <button
        data-testid="settings-save"
        onclick={save}
        class="px-4 py-2 rounded bg-indigo-600 hover:bg-indigo-500 text-sm"
      >
        Save
      </button>
      <button
        data-testid="settings-cancel"
        onclick={cancel}
        class="px-4 py-2 rounded bg-zinc-800 hover:bg-zinc-700 text-sm"
      >
        Cancel
      </button>
    </div>
  </div>
</main>
