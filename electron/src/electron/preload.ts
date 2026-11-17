import { contextBridge, ipcRenderer } from "electron";

contextBridge.exposeInMainWorld("nudge", {
  platform: "electron" as const,

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

contextBridge.exposeInMainWorld("nudgeSettings", {
  getConfig: () => ipcRenderer.invoke("settings:get-config"),
  save: (config: unknown) => ipcRenderer.invoke("settings:save", config),
  browseSprite: () => ipcRenderer.invoke("settings:browse-sprite"),
  toggleAutostart: (desired: boolean) =>
    ipcRenderer.invoke("settings:toggle-autostart", desired),
  close: () => ipcRenderer.send("settings:close"),
  updateInfo: () => ipcRenderer.invoke("settings:update-info"),
  checkUpdate: () => ipcRenderer.invoke("settings:check-update"),
  openRelease: (url: string) => ipcRenderer.invoke("settings:open-release", url),

  onWindowBlur: (callback: () => void) => {
    const handler = () => callback();
    ipcRenderer.on("settings:blur", handler);
    return () => {
      ipcRenderer.removeListener("settings:blur", handler);
    };
  },
});
