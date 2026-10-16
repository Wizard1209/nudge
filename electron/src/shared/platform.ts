/**
 * Which platform's spelling of a hotkey this build shows and writes (§5:
 * "one canonical spelling per platform"). Every spelling parses everywhere;
 * only the way one is written back changes.
 */
export type Platform = "macos" | "windows";

/**
 * The platform this code is running on. The renderer has no `process` (context
 * isolation), so it reads the user agent; the Electron main process and the
 * unit tests read `process.platform`.
 *
 * Anything that is neither a Mac nor Windows (a Linux dev box, the browser
 * sub-app under a Linux CI) gets the Windows spelling — it is the word form,
 * which reads correctly on a keyboard without Apple's glyphs.
 */
export function detectPlatform(): Platform {
  // A DOM means a renderer, where the user agent is the only witness. Node
  // also defines `navigator` (with a "Node.js/24" user agent), so the DOM —
  // not `navigator` — is what tells the two apart.
  if (typeof document !== "undefined" && typeof navigator !== "undefined") {
    return /Mac|iPhone|iPad/.test(navigator.userAgent) ? "macos" : "windows";
  }
  if (typeof process !== "undefined" && process.platform) {
    return process.platform === "darwin" ? "macos" : "windows";
  }
  return "windows";
}
