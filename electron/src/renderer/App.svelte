<script lang="ts">
  import { onMount } from "svelte";
  import {
    restartsTimer,
    INITIAL_TRIGGER_SOURCE,
    type TriggerSource,
  } from "../shared/nudgeFlow";

  let doing = $state("");
  let bullshit = $state("");
  let nextMinutes = $state("10");
  let visible = $state(true);

  // Spec §4: first launch behaves like the timer just fired (initial
  // trigger_source = "timer"). It flips to "manual" when the user opens
  // the popup themselves (tray/pill click) and back to "timer" when the
  // timer fires. Esc/Switch consult this to decide whether they have to
  // restart the timer (only when the timer is already at zero).
  let triggerSource: TriggerSource = $state(INITIAL_TRIGGER_SOURCE);

  // Browser-mode timer state
  let browserTimerId: ReturnType<typeof setTimeout> | null = null;
  let browserTimerTarget = $state(0);
  let countdownText = $state("");

  let doingInput: HTMLInputElement;
  let minutesInput: HTMLInputElement;

  // Spec §4: in Electron, Switch-on-blur only fires if the popup actually
  // received focus when it opened (over a fullscreen app the OS may keep us in
  // the background — a stray blur then must not hide an unfocused popup).
  // Browser mode has no such constraint, so this stays true there.
  let gotFocus = true;

  const isElectron = typeof window !== "undefined" && !!window.nudge?.save;

  onMount(() => {
    doingInput?.focus();

    // Electron mode: re-focus the first field on show, and record whether the
    // window took focus. doing/bullshit are not cleared here — Enter/Esc clear
    // at close time; Switch preserves them.
    const unsub = window.nudge?.onShow?.((payload) => {
      gotFocus = payload?.gotFocus ?? true;
      doingInput?.focus();
    });

    // A focus that arrives just after show still enables Switch.
    const onFocus = () => {
      gotFocus = true;
    };
    window.addEventListener("focus", onFocus);

    // Switch (spec §4): window loses focus → hide + restart timer, keep
    // doing/bullshit so the next open continues from where the user left off.
    window.addEventListener("blur", switchAway);

    // Browser mode: update countdown every second
    let countdownId: ReturnType<typeof setInterval> | undefined;
    if (!isElectron) {
      countdownId = setInterval(() => {
        if (browserTimerTarget <= 0) return;
        const remainingMs = Math.max(0, browserTimerTarget - Date.now());
        const totalSec = Math.ceil(remainingMs / 1000);
        const m = Math.floor(totalSec / 60);
        const s = totalSec % 60;
        countdownText = `${m}:${String(s).padStart(2, "0")}`;
      }, 1000);
    }

    return () => {
      unsub?.();
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", switchAway);
      if (countdownId) clearInterval(countdownId);
    };
  });

  function showForm(source: "timer" | "manual") {
    triggerSource = source;
    visible = true;
    // doing/bullshit intentionally not cleared: save/dismiss cleared at close
    // time; Switch deliberately preserves them per spec §4.
    requestAnimationFrame(() => doingInput?.focus());
  }

  function switchAway() {
    if (!visible) return;
    // §4: ignore blur if the popup never actually held focus (Electron only).
    if (isElectron && !gotFocus) return;
    if (isElectron) {
      // Main process knows the trigger source — it decides whether to restart.
      const minutes = parseFloat(nextMinutes) || 10;
      window.nudge!.switch({ nextMinutes: minutes });
    } else if (restartsTimer("switch", triggerSource)) {
      // Timer was at zero; closing without setting a new deadline would
      // re-open the popup instantly. Use current minutes for the new one.
      hideAndRestartTimer(parseFloat(nextMinutes) || 10);
    } else {
      // Manually opened — live timer is still ticking behind us. Just hide.
      visible = false;
    }
  }

  function hideAndRestartTimer(minutes: number) {
    visible = false;
    if (browserTimerId) clearTimeout(browserTimerId);
    const ms = minutes * 60_000;
    browserTimerTarget = Date.now() + ms;
    const totalSec = Math.ceil(ms / 1000);
    const m = Math.floor(totalSec / 60);
    const s = totalSec % 60;
    countdownText = `${m}:${String(s).padStart(2, "0")}`;
    browserTimerId = setTimeout(() => showForm("timer"), ms);
  }

  async function save() {
    const minutes = parseFloat(nextMinutes) || 10;
    if (isElectron) {
      try {
        await window.nudge!.save({ doing, bullshit, nextMinutes: minutes });
      } catch {
        // Main process already surfaced an error dialog. Keep popup visible
        // and leave typed data intact so the user can retry.
        return;
      }
    } else {
      console.log("save", { doing, bullshit, nextMinutes: minutes });
      hideAndRestartTimer(minutes);
    }
    doing = "";
    bullshit = "";
  }

  async function dismiss() {
    // Spec §4: Esc shares its row with Switch — both preserve doing/bullshit
    // so the next open resumes where the user left off. Only Enter (save)
    // clears the fields. They also share the timer rule: leave the timer
    // alone unless the popup was timer-opened (then we must restart it).
    if (isElectron) {
      const minutes = parseFloat(nextMinutes) || 10;
      await window.nudge!.dismiss({ nextMinutes: minutes });
    } else if (restartsTimer("dismiss", triggerSource)) {
      hideAndRestartTimer(parseFloat(nextMinutes) || 10);
    } else {
      visible = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      save();
    } else if (e.key === "Escape") {
      e.preventDefault();
      dismiss();
    } else if (e.key === "Tab") {
      // Trap focus within the three fields. The popup is a frameless,
      // always-on-top window: tabbing off the last field (or Shift+Tab off the
      // first) lets focus leave the window, which fires a blur → §4 Switch →
      // the popup hides ("closes after three Tabs"). Wrapping keeps focus
      // inside so the window never blurs from keyboard navigation.
      if (!e.shiftKey && document.activeElement === minutesInput) {
        e.preventDefault();
        doingInput?.focus();
      } else if (e.shiftKey && document.activeElement === doingInput) {
        e.preventDefault();
        minutesInput?.focus();
      }
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<main
  class="relative h-screen overflow-hidden"
  class:bg-zinc-950={!isElectron}
>
  {#if !isElectron}
    <!-- Dot matrix background — browser only, transparent in Electron -->
    <div
      class="absolute inset-0"
      style="
        background-image: radial-gradient(#52525b 1px, transparent 1px);
        background-size: 24px 24px;
      "
    ></div>
  {/if}
  {#if visible}
    <form
      onkeydown={handleKeydown}
      onsubmit={(e) => e.preventDefault()}
      class="absolute z-10 left-1/2 -translate-x-1/2 {isElectron ? 'top-0' : 'top-[25vh]'} bg-zinc-900/80 backdrop-blur-md text-white rounded-lg shadow-2xl w-[480px] overflow-hidden"
      data-testid="nudge"
    >
      <input
        bind:this={doingInput}
        bind:value={doing}
        type="text"
        placeholder="Что я делаю?"
        data-testid="field-doing"
        class="w-full bg-transparent focus:bg-white/8 text-white px-5 py-4 text-base outline-none placeholder-zinc-500 border-b border-zinc-700 transition-none"
      />
      <input
        bind:value={bullshit}
        type="text"
        placeholder="Хуйня?"
        data-testid="field-bullshit"
        class="w-full bg-transparent focus:bg-white/8 text-white px-5 py-4 text-base outline-none placeholder-zinc-500 border-b border-zinc-700 transition-none"
      />
      <input
        bind:this={minutesInput}
        bind:value={nextMinutes}
        type="text"
        inputmode="decimal"
        placeholder="Следующий через (мин)"
        data-testid="field-minutes"
        class="w-full bg-transparent focus:bg-white/8 text-white px-5 py-4 text-base outline-none placeholder-zinc-500 transition-none"
      />
    </form>
  {/if}

  <!-- Emulated tray (browser mode only) -->
  {#if !visible && !isElectron}
    <button
      onclick={() => showForm("manual")}
      data-testid="tray"
      class="fixed bottom-4 right-4 z-50 bg-zinc-800 text-zinc-300 text-sm px-4 py-2 rounded-full shadow-lg hover:bg-zinc-700 transition-colors cursor-pointer"
    >
      {countdownText}
    </button>
  {/if}
</main>
