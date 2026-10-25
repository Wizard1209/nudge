<script lang="ts">
  import { onMount } from "svelte";
  import { SettingsForm, respell, type FormError } from "../shared/settingsForm";
  import { checkSpriteUrl } from "./browserSprite";
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

  // §9: the autostart row names the platform's own mechanism — the same
  // platform the hotkey spelling follows, so the two never disagree.
  const autostartLabel =
    platform === "macos" ? "Open at login" : "Launch with Windows";

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
  let form = SettingsForm.fromConfig(initial, platform);

  // Reactive mirrors of the form fields (Svelte 5 runes can't bind to a class
  // field directly, so we mirror and push changes back into the form).
  let hotkey = $state(form.hotkey);
  let intervalText = $state(form.intervalText);
  let autostart = $state(form.autostart);
  let spriteEnabled = $state(form.spriteEnabled);
  let spritePath = $state(form.spritePath);
  let spriteXText = $state(form.spriteXText);
  let spriteYText = $state(form.spriteYText);
  let cardWidthText = $state(form.cardWidthText);
  let spriteError = $state("");

  let recorder = $state<RecorderState>("idle");
  let banner = $state("");
  // §9: a save that reached the file says so, until the first edit after it.
  let saved = $state(false);

  // §9: Save is live only while the form differs from the file. Compared
  // against `persisted` — the on-disk view — rather than the form's own
  // baseline, so the answer does not depend on when the mirrors were synced.
  const dirty = $derived.by(() => {
    const probe = SettingsForm.fromConfig(persisted, platform);
    probe.hotkey = hotkey;
    probe.intervalText = intervalText;
    probe.autostart = autostart;
    probe.spriteEnabled = spriteEnabled;
    probe.spritePath = spritePath;
    probe.spriteXText = spriteXText;
    probe.spriteYText = spriteYText;
    probe.cardWidthText = cardWidthText;
    return probe.isDirty();
  });

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
    // §9: a stored value that parses is re-spelled for this platform on load
    // (the form does that as it seeds).
    form = SettingsForm.fromConfig(cfg, platform);
    syncFromForm();
  }

  function syncToForm() {
    form.hotkey = hotkey;
    form.intervalText = intervalText;
    form.autostart = autostart;
    form.spriteEnabled = spriteEnabled;
    form.spritePath = spritePath;
    form.spriteXText = spriteXText;
    form.spriteYText = spriteYText;
    form.cardWidthText = cardWidthText;
  }

  function syncFromForm() {
    hotkey = form.hotkey;
    intervalText = form.intervalText;
    autostart = form.autostart;
    spriteEnabled = form.spriteEnabled;
    spritePath = form.spritePath;
    spriteXText = form.spriteXText;
    spriteYText = form.spriteYText;
    cardWidthText = form.cardWidthText;
  }

  const FIELD_ERROR: Record<Exclude<FormError["field"], "hotkey">, string> = {
    interval: "Interval must be a positive number of minutes",
    sprite_x: "Sprite X must be a whole number of points",
    sprite_y: "Sprite Y must be a whole number of points",
    card_width: "Card width must be a positive whole number of points",
  };

  async function browseSprite() {
    const picked = await settingsApi?.browseSprite();
    if (picked) {
      spritePath = picked;
      onEdit(); // a pick is an edit, though it fires no input event
    }
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
        onEdit();
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

  /** The first edit after a save takes the `Saved` note away (§9). */
  function onEdit() {
    saved = false;
  }

  function keepsItsOwnReturn(target: EventTarget | null): boolean {
    return (
      target instanceof HTMLButtonElement ||
      (target instanceof HTMLInputElement && target.type === "checkbox")
    );
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (recorder !== "armed") {
      // §9: Return anywhere in the form is Save. A focused button or
      // checkbox keeps its own Return (the same rule as the Rust window,
      // where egui makes it a click), so Cancel stays Cancel. A Return that
      // ends an input-method composition is the composition's.
      if (e.key === "Enter" && !e.isComposing && !keepsItsOwnReturn(e.target)) {
        e.preventDefault();
        void save();
      }
      return;
    }
    // While armed the key is the recorder's: Return is a press, not a save.
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
    // §9: the same checks, the same write, the same messages from Return and
    // from the button — and nothing to do while nothing differs from the file.
    if (!dirty) return;
    syncToForm();
    const result = form.toConfig();
    if (!result.ok) {
      // §9: if any field fails the file is not touched at all, and the
      // hotkey's banner also states the accepted vocabulary. One message
      // line reports the one failure this save found: a sprite refusal from
      // an earlier save does not stand next to it.
      const err = result.error;
      if (err.field === "hotkey") {
        banner = `${describeParseError(err.error)}. ${vocabulary(platform)}`;
        spriteError = "";
      } else if (err.field === "interval") {
        banner = FIELD_ERROR.interval;
        spriteError = "";
      } else {
        spriteError = FIELD_ERROR[err.field];
        banner = "";
      }
      return;
    }
    // An enabled sprite must be a usable PNG; a disabled one saves as typed.
    // Electron checks the file in the main process on save; the browser
    // sub-app checks the path as a URL before persisting.
    const { sprite } = result.config;
    let error: string | null = null;
    if (settingsApi) {
      const saved = await settingsApi.save(result.config);
      if (!saved.ok) error = saved.error;
    } else {
      error = sprite.enabled ? await checkSpriteUrl(sprite.path) : null;
      if (!error) persistConfig(result.config);
    }
    if (error) {
      spriteError = error;
      banner = "";
      return;
    }
    persisted = result.config;
    form.markClean();
    syncFromForm();
    spriteError = "";
    banner = "";
    saved = true;
  }

  async function cancel() {
    // Revert to the saved config. In Electron, Cancel closes the window
    // (spec §9); in the browser sub-app there's no window, so we just reset.
    //
    // The revert re-reads rather than restoring the `persisted` snapshot: this
    // is a cancelling path like any other, and the recorder remembers nothing.
    // Re-seeding `persisted` first is what keeps the field the table restores
    // and the fields this function resets on the same value.
    const cfg = settingsApi ? await settingsApi.getConfig() : loadStored();
    persisted = cfg;
    await step({ kind: "cancel" });
    reseed(cfg);
    banner = "";
    spriteError = "";
    saved = false;
    settingsApi?.close();
  }
</script>

<svelte:window onkeydown={onWindowKeydown} onblur={onWindowBlur} />

<!-- §9 "Window chrome and layout": three groups of rows in rounded boxes, a
     hairline between rows, the label at the left of a row and the control at
     its right edge; the hotkey vocabulary is its group's footnote; the
     message line keeps its height above the buttons; Cancel, then the one
     filled button, Save, at the bottom right. The controls are the compact
     (py-1) size so a 44px row keeps air around them. -->
<main class="min-h-screen bg-zinc-950 text-white p-5" oninput={onEdit}>
  <div class="space-y-5">
    <div>
      <div class="rounded-[10px] border border-zinc-800 bg-zinc-900/50 divide-y divide-zinc-800">
        <div class="flex items-center justify-between gap-3 min-h-11 px-3">
          <label class="text-sm" for="settings-hotkey">Global hotkey</label>
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
              class="w-[200px] bg-zinc-900 rounded px-3 py-1 outline-none border {recorder ===
              'armed'
                ? 'border-indigo-500'
                : 'border-zinc-700'}"
            />
            <button
              data-testid="settings-hotkey-record"
              data-recorder={recorder}
              onclick={onRecordClick}
              class="px-3 py-1 rounded bg-zinc-800 hover:bg-zinc-700 text-sm"
            >
              {recorder === "armed" ? "Press a key…" : "Record"}
            </button>
          </div>
        </div>
      </div>
      <!-- §9: the accepted vocabulary is standing information under the
           field — the group's footnote. -->
      <p
        data-testid="settings-hotkey-vocabulary"
        class="mt-2 px-3 text-xs text-zinc-500"
      >
        {vocabulary(platform)}
      </p>
    </div>

    <div class="rounded-[10px] border border-zinc-800 bg-zinc-900/50 divide-y divide-zinc-800">
      <div class="flex items-center justify-between gap-3 min-h-11 px-3">
        <label class="text-sm" for="settings-interval">Default interval</label>
        <div class="flex items-center gap-2 text-sm text-zinc-400">
          <input
            id="settings-interval"
            data-testid="settings-interval"
            type="text"
            inputmode="decimal"
            bind:value={intervalText}
            class="w-[72px] bg-zinc-900 text-base rounded px-3 py-1 outline-none border border-zinc-700"
          />
          <span>min</span>
        </div>
      </div>
      <!-- §9: the row's text names the platform's own mechanism. -->
      <label class="flex items-center justify-between gap-3 min-h-11 px-3 cursor-default">
        <span class="text-sm">{autostartLabel}</span>
        <input
          data-testid="settings-autostart"
          type="checkbox"
          checked={autostart}
          onchange={onAutostartChange}
        />
      </label>
    </div>

    <div class="rounded-[10px] border border-zinc-800 bg-zinc-900/50 divide-y divide-zinc-800">
      <label class="flex items-center justify-between gap-3 min-h-11 px-3 cursor-default">
        <span class="text-sm">Sprite</span>
        <input
          data-testid="settings-sprite-enabled"
          type="checkbox"
          bind:checked={spriteEnabled}
        />
      </label>
      <div class="flex items-center justify-between gap-3 min-h-11 px-3">
        <span class="text-sm shrink-0">PNG</span>
        <div class="flex flex-1 min-w-0 gap-2">
          <input
            data-testid="settings-sprite-path"
            type="text"
            aria-label="Sprite PNG path"
            bind:value={spritePath}
            placeholder="Path to a PNG with a transparent background"
            class="flex-1 min-w-0 bg-zinc-900 rounded px-3 py-1 outline-none border border-zinc-700"
          />
          {#if settingsApi}
            <button
              data-testid="settings-sprite-browse"
              onclick={browseSprite}
              class="px-3 py-1 rounded bg-zinc-800 hover:bg-zinc-700 text-sm"
            >
              Browse…
            </button>
          {/if}
        </div>
      </div>
      <!-- §10: the three are in points; the unit is said once, on the row. -->
      <div class="flex items-center justify-between gap-3 min-h-11 px-3">
        <span data-testid="settings-sprite-position" class="text-sm">Position (pt)</span>
        <div class="flex items-center gap-3">
          <label class="flex items-center gap-1.5 text-sm text-zinc-400">
            <span>X</span>
            <input
              data-testid="settings-sprite-x"
              type="text"
              inputmode="numeric"
              bind:value={spriteXText}
              class="w-[72px] bg-zinc-900 text-base text-white rounded px-3 py-1 outline-none border border-zinc-700"
            />
          </label>
          <label class="flex items-center gap-1.5 text-sm text-zinc-400">
            <span>Y</span>
            <input
              data-testid="settings-sprite-y"
              type="text"
              inputmode="numeric"
              bind:value={spriteYText}
              class="w-[72px] bg-zinc-900 text-base text-white rounded px-3 py-1 outline-none border border-zinc-700"
            />
          </label>
          <label class="flex items-center gap-1.5 text-sm text-zinc-400">
            <span>Card width</span>
            <input
              data-testid="settings-card-width"
              type="text"
              inputmode="numeric"
              bind:value={cardWidthText}
              class="w-[72px] bg-zinc-900 text-base text-white rounded px-3 py-1 outline-none border border-zinc-700"
            />
          </label>
        </div>
      </div>
    </div>

    <!-- §9: the message line is reserved — it keeps its height whether or
         not it has anything to say, so the window never changes size and
         never scrolls. Tall enough for the longest message, a refused hotkey
         with the vocabulary after it, and nothing a message quotes can make
         one longer (`echoed`); a sprite refusal is a message like any other
         and sits on the same line. The height is fixed, not a minimum, and
         the line clips: the form's shape is the line's, never the message's. -->
    <div class="flex h-[100px] flex-col justify-end gap-1 overflow-hidden">
      {#if banner}
        <p data-testid="settings-banner" class="text-sm text-amber-400">{banner}</p>
      {/if}
      {#if spriteError}
        <p data-testid="settings-sprite-error" class="text-sm text-red-400">{spriteError}</p>
      {/if}
    </div>

    <div class="flex items-center justify-end gap-3">
      {#if saved}
        <span data-testid="settings-saved" class="text-sm text-zinc-400">Saved</span>
      {/if}
      <button
        data-testid="settings-cancel"
        onclick={cancel}
        class="px-4 py-2 rounded bg-zinc-800 hover:bg-zinc-700 text-sm"
      >
        Cancel
      </button>
      <!-- §9: greyed while there is nothing to save — the same fill, dimmed. -->
      <button
        data-testid="settings-save"
        onclick={save}
        disabled={!dirty}
        class="px-4 py-2 rounded bg-indigo-600 hover:bg-indigo-500 text-sm disabled:opacity-40"
      >
        Save
      </button>
    </div>
  </div>
</main>
