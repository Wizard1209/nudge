export type Platform = "macos" | "windows";

export function detectPlatform(): Platform {
  if (typeof document !== "undefined" && typeof navigator !== "undefined") {
    return /Mac|iPhone|iPad/.test(navigator.userAgent) ? "macos" : "windows";
  }
  if (typeof process !== "undefined" && process.platform) {
    return process.platform === "darwin" ? "macos" : "windows";
  }
  return "windows";
}
