import { app } from "electron";
import type { AutostartProvider } from "../shared/autostart";

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
