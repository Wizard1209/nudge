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
    UPDATE_FOOTNOTE,
    buttonFor,
    statusText,
    updateStep,
    type UpdateInput,
    type UpdateState,
  } from "../shared/updateCheck";
  import {
    DEFAULT_CONFIG,
    normalize,
    type Config,
  } from "../shared/config";

  const settingsApi =
    typeof window !== "undefined" ? window.nudgeSettings : undefined;

  const STORAGE_KEY = "nudge-config";

  const platform = detectPlatform();

  const autostartLabel =
    platform === "macos" ? "Open at login" : "Launch with Windows";

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

  const initial = loadStored();
  let persisted = $state<Config>(initial);
  const provider = new FakeProvider(initial.autostart);
  let form = SettingsForm.fromConfig(initial, platform);

  let hotkey = $state(form.hotkey);
  let intervalText = $state(form.intervalText);
  let autostart = $state(form.autostart);
  let spriteEnabled = $state(form.spriteEnabled);
  let spritePath = $state(form.spritePath);
  let spriteXText = $state(form.spriteXText);
  let spriteYText = $state(form.spriteYText);
  let cardWidthText = $state(form.cardWidthText);
  let spriteError = $state("");

  let version = $state("");
  let update = $state<UpdateState>({ kind: "idle" });

  let recorder = $state<RecorderState>("idle");
  let banner = $state("");
  let saved = $state(false);

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

  onMount(() => {
    if (!settingsApi) return;
    void settingsApi.getConfig().then(reseed);
    void settingsApi.updateInfo().then((info) => (version = info.version));
    return settingsApi.onWindowBlur?.(() => onWindowBlur());
  });

  function reseed(cfg: Config) {
    persisted = cfg;
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
      onEdit();
    }
  }

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

  async function updateInput(input: UpdateInput) {
    if (!settingsApi) return;
    const out = updateStep(update, input, version, "openPage");
    update = out.state;
    if (out.effect.kind === "check") {
      const result = await settingsApi.checkUpdate();
      await updateInput({ kind: "checked", result });
    } else if (out.effect.kind === "openPage") {
      await settingsApi.openRelease(out.effect.url);
    }
  }

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
      if (e.key === "Enter" && !e.isComposing && !keepsItsOwnReturn(e.target)) {
        e.preventDefault();
        void save();
      }
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    const verdict = classifyPress(e);
    if (verdict === null) return;
    void step({ kind: "press", verdict });
  }

  function onWindowBlur() {
    void step({ kind: "focusLost" });
  }

  function onRecordClick() {
    void step(recorder === "armed" ? { kind: "cancel" } : { kind: "record" });
  }

  async function onAutostartChange(e: Event) {
    const desired = (e.target as HTMLInputElement).checked;
    if (settingsApi) {
      const result = await settingsApi.toggleAutostart(desired);
      if (result.ok) {
        persisted = { ...persisted, autostart: desired };
        autostart = desired;
        form.autostart = desired;
        banner = "";
      } else {
        autostart = !desired;
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
      autostart = !desired;
      banner =
        result.error.kind === "backend"
          ? "Could not change autostart"
          : "Autostart changed but saving failed";
    }
  }

  async function save() {
    if (!dirty) return;
    syncToForm();
    const result = form.toConfig();
    if (!result.ok) {
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
      <div class={["flex items-center justify-between gap-3 min-h-11 px-3", !spriteEnabled && "*:opacity-50"]}>
        <span class="text-sm shrink-0">PNG</span>
        <div class="flex flex-1 min-w-0 gap-2">
          <input
            data-testid="settings-sprite-path"
            type="text"
            aria-label="Sprite PNG path"
            bind:value={spritePath}
            placeholder="Path to a PNG with a transparent background"
            class="flex-1 min-w-0 bg-zinc-900 text-sm rounded px-3 py-1 outline-none border border-zinc-700"
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
      <div class={["flex items-center justify-between gap-3 min-h-11 px-3", !spriteEnabled && "*:opacity-50"]}>
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

    {#if settingsApi}
      <div>
        <div class="rounded-[10px] border border-zinc-800 bg-zinc-900/50">
          <div class="flex items-center justify-between gap-3 min-h-11 px-3">
            <span data-testid="settings-version" class="text-sm">Nudge {version}</span>
            <div class="flex items-center gap-3">
              <span data-testid="settings-update-status" class="text-sm text-zinc-400">
                {statusText(update)}
              </span>
              <button
                data-testid="settings-update"
                onclick={() => updateInput({ kind: "press" })}
                disabled={!buttonFor(update, "openPage").enabled}
                class="px-3 py-1 rounded bg-zinc-800 hover:bg-zinc-700 text-sm disabled:opacity-40"
              >
                {buttonFor(update, "openPage").label}
              </button>
            </div>
          </div>
        </div>
        <p data-testid="settings-update-footnote" class="mt-2 px-3 text-xs text-zinc-500">
          {UPDATE_FOOTNOTE}
        </p>
      </div>
    {/if}

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
