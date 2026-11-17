import { describe, it, expect } from "vitest";
import { restartsTimer, INITIAL_TRIGGER_SOURCE } from "../../src/shared/nudgeFlow";

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

describe("browser mode's opening popup (§4)", () => {
  it("treats the opening popup as a timer fire, so Esc/Switch arm the timer", () => {
    expect(INITIAL_TRIGGER_SOURCE).toBe("timer");
    expect(restartsTimer("dismiss", INITIAL_TRIGGER_SOURCE)).toBe(true);
    expect(restartsTimer("switch", INITIAL_TRIGGER_SOURCE)).toBe(true);
  });
});
