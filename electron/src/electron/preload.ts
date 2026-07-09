import { contextBridge, ipcRenderer } from "electron";

contextBridge.exposeInMainWorld("nudge", {
  platform: "electron" as const,

  save: (data: { doing: string; bullshit: string; nextMinutes: number }) =>
    ipcRenderer.invoke("nudge:save", data),

  dismiss: (data: { nextMinutes: number }) =>
    ipcRenderer.invoke("nudge:dismiss", data),

  switch: (data: { nextMinutes: number }) =>
    ipcRenderer.invoke("nudge:switch", data),

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
  toggleAutostart: (desired: boolean) =>
    ipcRenderer.invoke("settings:toggle-autostart", desired),
  close: () => ipcRenderer.send("settings:close"),
});
