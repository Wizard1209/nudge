import { describe, it, expect } from "vitest";
import { restartsTimer, INITIAL_TRIGGER_SOURCE } from "../../src/shared/nudgeFlow";

// Spec §4: Enter (save) always (re)starts the timer. Esc/Switch leave the timer
// alone *unless* the popup was opened by the timer itself — a timer-opened
// popup's deadline is already at zero, so closing without rearming would re-open
// it instantly. A manually-opened popup has a live timer ticking behind it.
describe("restartsTimer (§4 close → restart decision)", () => {
  it("save always restarts the timer, whatever opened the popup", () => {
    expect(restartsTimer("save", "manual")).toBe(true);
    expect(restartsTimer("save", "timer")).toBe(true);
  });

  it("dismiss/switch restart only when the popup was timer-triggered", () => {
    expect(restartsTimer("dismiss", "timer")).toBe(true);
    expect(restartsTimer("switch", "timer")).toBe(true);
    expect(restartsTimer("dismiss", "manual")).toBe(false);
    expect(restartsTimer("switch", "manual")).toBe(false);
  });
});

// Spec §4: the first popup shown on launch behaves as if the timer had just
// fired, so ANY way of closing it (Enter, Esc, Switch) arms the timer for the
// next cycle. The bug: the app opened it as "manual", so Esc/Switch on the very
// first popup left it with no timer at all — "the timer dies after first
// launch".
describe("first-launch rule (§4)", () => {
  it("treats the first popup as a timer fire, so Esc/Switch arm the timer", () => {
    expect(INITIAL_TRIGGER_SOURCE).toBe("timer");
    expect(restartsTimer("dismiss", INITIAL_TRIGGER_SOURCE)).toBe(true);
    expect(restartsTimer("switch", INITIAL_TRIGGER_SOURCE)).toBe(true);
  });
});
