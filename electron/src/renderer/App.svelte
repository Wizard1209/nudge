<script lang="ts">
  import { onMount } from "svelte";
  import {
    restartsTimer,
    INITIAL_TRIGGER_SOURCE,
    type TriggerSource,
  } from "../shared/nudgeFlow";
  import {
    CARD_WIDTH,
    CARD_HEIGHT,
    CARD_TOP_OFFSET,
    spriteLayout,
    spritePoints,
    type SpriteView,
  } from "../shared/sprite";
  import { cardFillAlpha, cardMaterial } from "../shared/card";
  import { detectPlatform } from "../shared/platform";

  let doing = $state("");
  let bullshit = $state("");
  let nextMinutes = $state("10");
  let visible = $state(true);

  let triggerSource: TriggerSource = $state(INITIAL_TRIGGER_SOURCE);

  let browserTimerId: ReturnType<typeof setTimeout> | null = null;
  let browserTimerTarget = $state(0);
  let countdownText = $state("");

  let doingInput: HTMLInputElement;
  let minutesInput: HTMLInputElement;

  let gotFocus = true;

  const isElectron = typeof window !== "undefined" && !!window.nudge?.save;

  let sprite: SpriteView | null = $state(null);

  const platform = detectPlatform();
  const backdrop = isElectron && !!window.nudge?.backdrop;
  const cardAlpha = $derived(
    cardFillAlpha(cardMaterial({ electron: isElectron, platform, sprite: !!sprite, backdrop })),
  );

  async function browserSprite(): Promise<SpriteView | null> {
    const query = new URLSearchParams(location.search);
    const url = query.get("sprite");
    if (!url) return null;
    const int = (key: string, fallback: number) => {
      const n = Number(query.get(key));
      return query.has(key) && Number.isInteger(n) ? n : fallback;
    };
    const img = new Image();
    img.src = url;
    await img.decode();
    return {
      url,
      layout: spriteLayout(
        { width: int("cardWidth", CARD_WIDTH), height: CARD_HEIGHT },
        {
          x: int("x", 0),
          y: int("y", 0),
          ...spritePoints({ width: img.naturalWidth, height: img.naturalHeight }, window.devicePixelRatio),
        },
      ),
    };
  }

  const stageLeft = (s: SpriteView) =>
    isElectron ? "0px" : `calc(50% - ${s.layout.card.width / 2 + s.layout.card.x}px)`;
  const stageTop = (s: SpriteView) =>
    isElectron ? "0px" : `calc(25vh + ${CARD_TOP_OFFSET - s.layout.card.y}px)`;

  onMount(() => {
    doingInput?.focus();
    const unsubSprite = isElectron
      ? window.nudge!.onSprite?.((s) => (sprite = s))
      : undefined;
    (isElectron ? window.nudge!.getSprite?.() : browserSprite())?.then(
      (s) => (sprite = s ?? null),
    );

    const unsub = window.nudge?.onShow?.((payload) => {
      gotFocus = payload?.gotFocus ?? true;
      doingInput?.focus();
    });

    const onFocus = () => {
      gotFocus = true;
    };
    window.addEventListener("focus", onFocus);

    window.addEventListener("blur", switchAway);

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
      unsubSprite?.();
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", switchAway);
      if (countdownId) clearInterval(countdownId);
    };
  });

  function showForm(source: "timer" | "manual") {
    triggerSource = source;
    visible = true;
    requestAnimationFrame(() => doingInput?.focus());
  }

  function switchAway() {
    if (!visible) return;
    if (isElectron && !gotFocus) return;
    if (isElectron) {
      const minutes = parseFloat(nextMinutes) || 10;
      window.nudge!.switch({ nextMinutes: minutes });
    } else if (restartsTimer("switch", triggerSource)) {
      hideAndRestartTimer(parseFloat(nextMinutes) || 10);
    } else {
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
    <div
      class="absolute inset-0"
      style="
        background-image: radial-gradient(#52525b 1px, transparent 1px);
        background-size: 24px 24px;
      "
    ></div>
  {/if}
  {#if visible}
    {#if sprite}
      <div
        class="absolute pointer-events-none"
        style:left={stageLeft(sprite)}
        style:top={stageTop(sprite)}
        style:width="{sprite.layout.window.width}px"
        style:height="{sprite.layout.window.height}px"
      >
        <img
          src={sprite.url}
          alt=""
          data-testid="sprite"
          class="absolute z-20 select-none max-w-none"
          style:left="{sprite.layout.sprite.x}px"
          style:top="{sprite.layout.sprite.y}px"
          style:width="{sprite.layout.sprite.width}px"
          style:height="{sprite.layout.sprite.height}px"
        />
      </div>
    {/if}
    <form
      onkeydown={handleKeydown}
      onsubmit={(e) => e.preventDefault()}
      class="absolute z-10 {sprite ? '' : `left-1/2 -translate-x-1/2 w-[480px] ${isElectron ? 'top-0' : 'top-[calc(25vh+80px)]'}`} card overflow-hidden"
      class:card-browser={!isElectron}
      style:--card-alpha={cardAlpha}
      style:left={sprite ? `calc(${stageLeft(sprite)} + ${sprite.layout.card.x}px)` : undefined}
      style:top={sprite ? `calc(${stageTop(sprite)} + ${sprite.layout.card.y}px)` : undefined}
      style:width={sprite ? `${sprite.layout.card.width}px` : undefined}
      data-testid="nudge"
    >
      <div class="row separated">
        <input
          bind:this={doingInput}
          bind:value={doing}
          type="text"
          placeholder="Что я делаю?"
          data-testid="field-doing"
          class="transition-none"
        />
      </div>
      <div class="row separated">
        <input
          bind:value={bullshit}
          type="text"
          placeholder="Хуйня?"
          data-testid="field-bullshit"
          class="transition-none"
        />
      </div>
      <div class="row">
        <input
          bind:this={minutesInput}
          bind:value={nextMinutes}
          type="text"
          inputmode="decimal"
          placeholder="Следующий через (мин)"
          data-testid="field-minutes"
          class="transition-none"
        />
      </div>
    </form>
  {/if}

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
