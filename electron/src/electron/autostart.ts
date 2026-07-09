import { app } from "electron";
import type { AutostartProvider } from "../shared/autostart";

/**
 * Real autostart backed by Electron's login-item settings (the Windows
 * registry Run key under the hood). Synchronous, matching the
 * AutostartProvider contract `applyAutostart` relies on.
 */
export class ElectronAutostartProvider implements AutostartProvider {
  enable(): void {
    app.setLoginItemSettings({ openAtLogin: true });
  }
  disable(): void {
    app.setLoginItemSettings({ openAtLogin: false });
  }
  isEnabled(): boolean {
    return app.getLoginItemSettings().openAtLogin;
  }
}
