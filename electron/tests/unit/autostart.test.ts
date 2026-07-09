import { describe, it, expect, vi } from "vitest";
import { FakeProvider, applyAutostart } from "../../src/shared/autostart";

describe("applyAutostart", () => {
  it("enables: flips the OS state and persists", () => {
    const provider = new FakeProvider(false);
    const persist = vi.fn();
    const result = applyAutostart(provider, true, persist);
    expect(result).toEqual({ ok: true });
    expect(provider.isEnabled()).toBe(true);
    expect(persist).toHaveBeenCalledTimes(1);
  });

  it("disables: flips the OS state and persists", () => {
    const provider = new FakeProvider(true);
    const persist = vi.fn();
    const result = applyAutostart(provider, false, persist);
    expect(result).toEqual({ ok: true });
    expect(provider.isEnabled()).toBe(false);
    expect(persist).toHaveBeenCalledTimes(1);
  });

  it("backend failure leaves the state untouched and never persists", () => {
    const provider = new FakeProvider(false, { failEnable: true });
    const persist = vi.fn();
    const result = applyAutostart(provider, true, persist);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error.kind).toBe("backend");
    expect(provider.isEnabled()).toBe(false);
    expect(persist).not.toHaveBeenCalled();
  });

  it("treats an unconfirmed change as a backend error", () => {
    // enable() succeeds but isEnabled keeps reporting false (the OS lied).
    const provider = new FakeProvider(false, { lieOnConfirm: true });
    const persist = vi.fn();
    const result = applyAutostart(provider, true, persist);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error.kind).toBe("backend");
    expect(persist).not.toHaveBeenCalled();
  });

  it("surfaces a persist failure after the OS change confirmed", () => {
    const provider = new FakeProvider(false);
    const persist = vi.fn(() => {
      throw new Error("disk full");
    });
    const result = applyAutostart(provider, true, persist);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error.kind).toBe("persist");
    // OS change already took effect even though config write failed.
    expect(provider.isEnabled()).toBe(true);
  });
});
