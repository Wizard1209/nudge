// The popup's open/close flow rules from spec §4, as pure functions shared by
// the Electron main process (src/electron/main.ts) and the browser renderer
// (src/renderer/App.svelte). Keeping the decision here stops the two surfaces
// from diverging — which is exactly how the "timer dies after first launch"
// bug arose (main showed the first popup as "manual" while the renderer used
// "timer", so Esc/Switch on launch never armed the timer in Electron).

export type TriggerSource = "timer" | "manual";
export type CloseAction = "save" | "dismiss" | "switch";

// The source of the popup the browser mode opens on page load. The Electron
// app launches in Waiting (§4) and shows nothing until a deadline or a
// summon; the browser mode is a harness for the card and starts as if the
// interval had elapsed, so every close path (Enter/Esc/Switch) arms the timer
// — opening it as "manual" is what killed the timer when the first popup was
// closed with Esc/Switch.
export const INITIAL_TRIGGER_SOURCE: TriggerSource = "timer";

// Does closing the popup with `action` (when it was opened by `source`) restart
// the timer? Enter (save) always does. Esc/Switch only do when the popup was
// timer-triggered — a timer-opened popup's deadline is at zero, so not rearming
// would re-open it instantly; a manually-opened popup has a live timer behind
// it and must be left alone.
export function restartsTimer(action: CloseAction, source: TriggerSource): boolean {
  if (action === "save") return true;
  return source === "timer";
}
