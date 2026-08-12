<script lang="ts">
  import { onMount } from "svelte";
  import { SettingsForm, respell } from "../shared/settingsForm";
  import {
    describeParseError,
    formatHotkeyNative,
    vocabulary,
  } from "../shared/hotkey";
  import {
    classifyPress,
    hintText,
    recorderStep,
    type RecorderInput,
    type RecorderState,
  } from "../shared/hotkeyRecorder";
  import { detectPlatform } from "../shared/platform";
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

  /** Which spelling this build shows and writes (§5: one per platform). */
  const platform = detectPlatform();

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
  const form = SettingsForm.fromConfig(initial, platform);

  // Reactive mirrors of the form fields (Svelte 5 runes can't bind to a class
  // field directly, so we mirror and push changes back into the form).
  let hotkey = $state(form.hotkey);
  let intervalText = $state(form.intervalText);
  let autostart = $state(form.autostart);

  let recorder = $state<RecorderState>("idle");
  let banner = $state("");

  // Electron loads its config over IPC (async), so seed the fields after mount.
  onMount(() => {
    if (!settingsApi) return;
    void settingsApi.getConfig().then(reseed);
    // In Electron the focus loss comes from the native window, not from the
    // page: `window`'s own blur is the browser sub-app's only signal, and it
    // is the one a test harness suppresses.
    return settingsApi.onWindowBlur?.(() => onWindowBlur());
  });

  function reseed(cfg: Config) {
    persisted = cfg;
    // §9: a stored value that parses is re-spelled for this platform on load.
    form.hotkey = respell(cfg.hotkey, platform);
    form.intervalText = SettingsForm.fromConfig(cfg, platform).intervalText;
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

  // --- The recorder (docs/TASK-hotkey-native.md) ---
  //
  // This shell owns none of the decisions: it turns real events into the
  // table's inputs, hands them to `recorderStep`, and performs the effect it
  // gets back. The one thing it decides is *when* a keydown is a press edge,
  // and `classifyPress` answers even that.

  /** The saved hotkey, re-read on every cancelling path. The recorder
   * remembers nothing when it arms, so a value changed on disk (or by the
   * other surface) while the field was armed is the one that comes back. */
  async function savedHotkey(): Promise<string> {
    const cfg = settingsApi ? await settingsApi.getConfig() : loadStored();
    return respell(cfg.hotkey, platform);
  }

  async function step(input: RecorderInput) {
    const { state, effect } = recorderStep(recorder, input);
    recorder = state;
    switch (effect.kind) {
      case "none":
        break;
      case "captured":
        hotkey = formatHotkeyNative(effect.hotkey, platform);
        banner = "";
        break;
      case "restoreSaved":
        hotkey = await savedHotkey();
        banner = "";
        break;
      case "hint":
        banner = hintText(effect.keyName, effect.reason, platform);
        break;
    }
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (recorder !== "armed") return;
    // While armed the recorder owns the keyboard: nothing types into the
    // field, and ⌘C records as ⌘C instead of copying. (In the packaged app a
    // combination that matches a menu accelerator is eaten by the menu before
    // the renderer sees it — Electron has no app-local key monitor the way
    // the task doc's macOS one does. The prototype has no menu bar, so the
    // gap is invisible here; the Rust app must not rely on it.)
    e.preventDefault();
    e.stopPropagation();
    const verdict = classifyPress(e);
    // A chord of modifiers alone is not an event: holding ⌥⌘ neither accepts
    // nor refuses, and neither does an auto-repeat.
    if (verdict === null) return;
    void step({ kind: "press", verdict });
  }

  function onWindowBlur() {
    // Focus loss cancels: *which* shortcut took the focus is not observable.
    void step({ kind: "focusLost" });
  }

  function onRecordClick() {
    void step(recorder === "armed" ? { kind: "cancel" } : { kind: "record" });
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
      // §9: if either field fails the file is not touched at all, and the
      // hotkey's banner also states the accepted vocabulary — the banner is
      // the only place that list appears at the moment it is needed.
      banner =
        result.error.field === "interval"
          ? "Interval must be a positive number of minutes"
          : `${describeParseError(result.error.error)}. ${vocabulary(platform)}`;
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

  async function cancel() {
    // Revert to the persisted view. In Electron, Cancel closes the window
    // (spec §9); in the browser sub-app there's no window, so we just reset.
    // An armed recorder is cancelled through the table, which is what puts
    // the saved hotkey back in the field.
    await step({ kind: "cancel" });
    hotkey = respell(persisted.hotkey, platform);
    intervalText = SettingsForm.fromConfig(persisted, platform).intervalText;
    autostart = persisted.autostart;
    banner = "";
    settingsApi?.close();
  }
</script>

<svelte:window onkeydown={onWindowKeydown} onblur={onWindowBlur} />

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
          readonly={recorder === "armed"}
          placeholder={formatHotkeyNative(
            { ctrl: true, alt: false, shift: true, win: false, key: "SPACE" },
            platform,
          )}
          class="flex-1 bg-zinc-900 rounded px-3 py-2 outline-none border {recorder ===
          'armed'
            ? 'border-indigo-500'
            : 'border-zinc-700'}"
        />
        <button
          data-testid="settings-hotkey-record"
          data-recorder={recorder}
          onclick={onRecordClick}
          class="px-3 py-2 rounded bg-zinc-800 hover:bg-zinc-700 text-sm"
        >
          {recorder === "armed" ? "Press a key…" : "Record"}
        </button>
      </div>
      <!-- §9: the accepted vocabulary is printed under the field, not held
           back for an error — this is how a hotkey is set, so it is standing
           information. -->
      <p data-testid="settings-hotkey-vocabulary" class="text-xs text-zinc-500">
        {vocabulary(platform)}
      </p>
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
