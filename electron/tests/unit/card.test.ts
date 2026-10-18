import { describe, it, expect } from "vitest";

import {
  CARD_RADIUS,
  CARD_CONTENT_HEIGHT,
  ROW_HEIGHT,
  SEPARATOR_HEIGHT,
  cardFillAlpha,
  cardMaterial,
} from "../../src/shared/card";
import { CARD_HEIGHT } from "../../src/shared/sprite";

// Spec §2/§3 numbers, as the reference app renders them.
describe("card metrics", () => {
  it("rounds the corners at 16", () => {
    expect(CARD_RADIUS).toBe(16);
  });

  it("a row is 42.5: 16px type with a 12px inset above and below", () => {
    expect(ROW_HEIGHT).toBe(42.5);
  });

  it("three rows and two 1px separators make a 129.5 card", () => {
    expect(SEPARATOR_HEIGHT).toBe(1);
    expect(CARD_CONTENT_HEIGHT).toBe(129.5);
  });

  it("the window is the whole number of points that holds the card", () => {
    expect(CARD_HEIGHT).toBe(130);
  });
});

// The fill's alpha depends on what is behind it (spec §2): over the macOS
// glass it only tints; over Windows Acrylic the measured result is ~95%; with
// nothing behind it the fill is the surface and is 95% itself.
describe("cardFillAlpha", () => {
  it("tints the glass at 120/255", () => {
    expect(cardFillAlpha("glass")).toBeCloseTo(120 / 255, 6);
  });

  it("keeps 0.8 over Acrylic", () => {
    expect(cardFillAlpha("acrylic")).toBe(0.8);
  });

  it("is a flat 0.95 with no material behind it", () => {
    expect(cardFillAlpha("none")).toBe(0.95);
  });
});

describe("cardMaterial", () => {
  it("is glass in the Electron app on macOS, sprite or not", () => {
    expect(cardMaterial({ electron: true, platform: "macos", sprite: false })).toBe("glass");
    expect(cardMaterial({ electron: true, platform: "macos", sprite: true })).toBe("glass");
  });

  it("is Acrylic in the Electron app on Windows without a sprite", () => {
    expect(cardMaterial({ electron: true, platform: "windows", sprite: false })).toBe("acrylic");
  });

  it("is nothing on Windows with a sprite: the window is transparent, and Acrylic cannot be", () => {
    expect(cardMaterial({ electron: true, platform: "windows", sprite: true })).toBe("none");
  });

  it("is nothing in the browser build", () => {
    expect(cardMaterial({ electron: false, platform: "macos", sprite: false })).toBe("none");
    expect(cardMaterial({ electron: false, platform: "windows", sprite: false })).toBe("none");
  });
});
