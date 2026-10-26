import { contextBridge, ipcRenderer } from "electron";

contextBridge.exposeInMainWorld("nudge", {
  platform: "electron" as const,

  // Whether the frosted material is under this window's card (macOS with the
  // backdrop addon built). Fixed for the window's life, and read before the
  // page's first paint so the fill never flashes the wrong alpha.
  backdrop: ipcRenderer.sendSync("nudge:backdrop") as boolean,

  save: (data: { doing: string; bullshit: string; nextMinutes: number }) =>
    ipcRenderer.invoke("nudge:save", data),

  dismiss: (data: { nextMinutes: number }) =>
    ipcRenderer.invoke("nudge:dismiss", data),

  switch: (data: { nextMinutes: number }) =>
    ipcRenderer.invoke("nudge:switch", data),

  getSprite: () => ipcRenderer.invoke("nudge:sprite"),

  onSprite: (callback: (sprite: unknown) => void) => {
    const handler = (_event: unknown, sprite: unknown) => callback(sprite);
    ipcRenderer.on("nudge:sprite", handler);
    return () => {
      ipcRenderer.removeListener("nudge:sprite", handler);
    };
  },

  onShow: (callback: (payload?: { gotFocus: boolean }) => void) => {
    const handler = (_event: unknown, payload?: { gotFocus: boolean }) =>
      callback(payload);
    ipcRenderer.on("nudge:show", handler);
    return () => {
      ipcRenderer.removeListener("nudge:show", handler);
    };
  },
});

// Settings window bridge — only meaningful in the framed Settings BrowserWindow,
// but exposed on every renderer (the popup just never calls it). Its presence
// is how Settings.svelte detects Electron vs. the browser sub-app.
contextBridge.exposeInMainWorld("nudgeSettings", {
  getConfig: () => ipcRenderer.invoke("settings:get-config"),
  save: (config: unknown) => ipcRenderer.invoke("settings:save", config),
  browseSprite: () => ipcRenderer.invoke("settings:browse-sprite"),
  toggleAutostart: (desired: boolean) =>
    ipcRenderer.invoke("settings:toggle-autostart", desired),
  close: () => ipcRenderer.send("settings:close"),

  // The native window lost focus. The hotkey recorder cancels on it.
  onWindowBlur: (callback: () => void) => {
    const handler = () => callback();
    ipcRenderer.on("settings:blur", handler);
    return () => {
      ipcRenderer.removeListener("settings:blur", handler);
    };
  },
});
