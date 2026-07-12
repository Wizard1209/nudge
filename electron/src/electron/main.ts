import {
  app,
  BrowserWindow,
  dialog,
  globalShortcut,
  ipcMain,
  screen,
  Tray,
  Menu,
  nativeImage,
} from "electron";
import path from "node:path";
import { watch, type FSWatcher } from "node:fs";
import { dirname, basename } from "node:path";
import { ulid } from "ulid";
import {
  writeEvent,
  getJournalPath,
  formatTimestamp,
  isJournalable,
  JournalValidationError,
  type JournalEvent,
} from "./journal";
import {
  loadConfig,
  saveConfig,
  getConfigPath,
  diffConfig,
  DEFAULT_CONFIG,
  type Config,
} from "./config";
import { parseHotkey, hotkeyToAccelerator } from "../shared/hotkey";
import {
  restartsTimer,
  INITIAL_TRIGGER_SOURCE,
  type TriggerSource,
  type CloseAction,
} from "../shared/nudgeFlow";
import { applyAutostart } from "../shared/autostart";
import { ElectronAutostartProvider } from "./autostart";
import { buildTrayIcon, daisyFrame, formatTooltip } from "./trayIcon";

// Load the dev server in development, the built files when packaged. The
// NUDGE_FORCE_PROD escape hatch lets native e2e tests drive an unpackaged
// build against the *shipped* renderer files (the same path the portable exe
// takes) instead of needing a Vite dev server running.
const isDev = !app.isPackaged && !process.env.NUDGE_FORCE_PROD;
const DEFAULT_MINUTES = 10;
// Spec §5: a petal fades out over ~250ms after it detaches; the popup appears
// "not at the moment the last petal detaches, but after its fall has played
// out completely".
const PETAL_FADE_MS = 250;

let win: BrowserWindow | null = null;
let settingsWin: BrowserWindow | null = null;
let tray: Tray | null = null;
let timerId: ReturnType<typeof setTimeout> | null = null;
let timerStartMs = 0;
let timerIntervalMs = 0;
let lastTriggerSource: TriggerSource = INITIAL_TRIGGER_SOURCE;

// --- Config / hotkey / autostart ---

const autostartProvider = new ElectronAutostartProvider();
let currentConfig: Config = { ...DEFAULT_CONFIG };
let registeredAccelerator: string | null = null;
let configWatcher: FSWatcher | null = null;
let watcherDebounce: ReturnType<typeof setTimeout> | null = null;

/** Default interval comes from config now, falling back to the constant. */
function configuredMinutes(): number {
  return parseMinutes(currentConfig.default_interval_minutes);
}

// Register the global hotkey from a config. Non-fatal per spec §5: a bad or
// already-taken combo logs to stderr and leaves the app running with no
// hotkey (the tray still works).
function registerHotkey(config: Config): void {
  unregisterHotkey();
  const parsed = parseHotkey(config.hotkey);
  if (!parsed.ok) {
    console.error(
      `Nudge: ignoring invalid hotkey "${config.hotkey}" (${parsed.error.kind})`,
    );
    return;
  }
  const accel = hotkeyToAccelerator(parsed.hotkey);
  try {
    const ok = globalShortcut.register(accel, () => showNudge("manual"));
    if (!ok) {
      console.error(`Nudge: hotkey "${accel}" is unavailable (already taken)`);
      return;
    }
    registeredAccelerator = accel;
  } catch (err) {
    console.error(`Nudge: failed to register hotkey "${accel}":`, err);
  }
}

function unregisterHotkey(): void {
  if (registeredAccelerator) {
    globalShortcut.unregister(registeredAccelerator);
    registeredAccelerator = null;
  }
}

// Re-apply a freshly-loaded config. Per spec §9 only the hotkey has a live
// effect; interval/autostart are cached for the next nudge / next launch.
function applyConfig(next: Config): void {
  const changed = diffConfig(currentConfig, next);
  currentConfig = next;
  if (changed.hotkey) registerHotkey(next);
}

// Watch the PARENT dir, not the file: an atomic save (tmp + rename) swaps the
// inode, which a file-level watch would miss after the first write.
function startConfigWatcher(): void {
  const configPath = getConfigPath();
  const dir = dirname(configPath);
  const file = basename(configPath);
  try {
    configWatcher = watch(dir, (_event, changed) => {
      if (changed !== file) return;
      if (watcherDebounce) clearTimeout(watcherDebounce);
      watcherDebounce = setTimeout(() => {
        const { config, error } = loadConfig(configPath);
        if (error) {
          console.error("Nudge: config reload failed, keeping previous:", error);
          return;
        }
        applyConfig(config);
      }, 100);
    });
  } catch (err) {
    console.error("Nudge: could not watch config dir:", err);
  }
}

// --- Timer ---

function startTimer(minutes: number) {
  if (timerId) clearTimeout(timerId);
  timerIntervalMs = minutes * 60_000;
  timerStartMs = Date.now();
  // The 12th petal detaches at t = interval and finishes its fall at
  // t = interval + PETAL_FADE_MS; the popup waits for the daisy to be
  // fully empty before showing up.
  timerId = setTimeout(() => showNudge("timer"), timerIntervalMs + PETAL_FADE_MS);
  updateTray();
}

function showNudge(source: TriggerSource) {
  if (!win) return;
  lastTriggerSource = source;
  positionCard();
  win.show();
  win.focus();
  // Spec §4: Switch-on-blur must only fire if the popup actually took focus
  // when it opened — over a fullscreen app the OS may keep us in the
  // background. Tell the renderer whether we got focus; it gates blur on this.
  win.webContents.send("nudge:show", { gotFocus: win.isFocused() });
}

// Spec §1: horizontally centered, top edge at 25% of screen height — card
// grows downward from there (upper third, eye level; clearly above the
// geometric centre).
function positionCard() {
  if (!win) return;
  const cursor = screen.getCursorScreenPoint();
  const { workArea } = screen.getDisplayNearestPoint(cursor);
  const [w] = win.getSize();
  const x = Math.round(workArea.x + (workArea.width - w) / 2);
  const y = Math.round(workArea.y + workArea.height * 0.25);
  win.setPosition(x, y);
}

function hideNudge() {
  if (!win) return;
  win.hide();
}

function parseMinutes(value: unknown): number {
  const n = Number(value);
  if (!Number.isFinite(n) || n <= 0) return DEFAULT_MINUTES;
  return n;
}

// --- Tray ---

function elapsedSinceTimerStart(): number {
  // Before the first close the timer is idle — daisy renders fully on stem.
  if (timerStartMs === 0) return 0;
  return Date.now() - timerStartMs;
}

function remainingMsForTooltip(): number {
  // Tooltip references the user-visible interval, not the +fade extension —
  // we don't want the "now" label to lag behind the visible expiry.
  if (timerStartMs === 0) return configuredMinutes() * 60_000;
  return timerStartMs + timerIntervalMs - Date.now();
}

function createTray() {
  const icon = nativeImage.createFromBuffer(buildTrayIcon(0));
  tray = new Tray(icon);
  tray.setToolTip(formatTooltip(configuredMinutes() * 60_000));
  tray.setContextMenu(
    Menu.buildFromTemplate([
      { label: "Show Nudge", click: () => showNudge("manual") },
      { label: "Settings", click: () => openSettings() },
      { type: "separator" },
      { label: "Quit", click: () => app.quit() },
    ]),
  );
  tray.on("click", () => showNudge("manual"));
}

// --- Settings window (spec §9) ---

// Framed, standard-chrome window. Single instance: focus the existing one
// rather than opening a second.
function openSettings() {
  if (settingsWin) {
    settingsWin.focus();
    return;
  }
  settingsWin = new BrowserWindow({
    // useContentSize: the 460×440 is the *client* area, so the OS title bar
    // doesn't eat into it and clip the form (the bug was the title bar
    // shrinking a frame-inclusive height on Windows).
    width: 460,
    height: 440,
    useContentSize: true,
    title: "Nudge — Settings",
    resizable: false,
    minimizable: false,
    maximizable: false,
    skipTaskbar: false,
    // Dark surface + show-on-ready so the window never flashes as a half-
    // painted gray rectangle on first open (spec §9 keeps the frame/title bar;
    // only the empty-white default is the bug).
    show: false,
    backgroundColor: "#09090b",
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });
  settingsWin.setMenuBarVisibility(false);
  settingsWin.once("ready-to-show", () => settingsWin?.show());

  if (isDev) {
    settingsWin.loadURL("http://localhost:5173/settings.html");
  } else {
    settingsWin.loadFile(path.join(__dirname, "../renderer/settings.html"));
  }

  settingsWin.on("closed", () => {
    settingsWin = null;
  });
}

function updateTray() {
  // The 50ms refresh loop keeps firing during shutdown, after Electron has
  // destroyed the Tray — touching a destroyed Tray throws "Object has been
  // destroyed" and surfaces as a crash on quit. Bail if it's gone.
  if (!tray || tray.isDestroyed()) return;
  // Spec §5: "While the popup is open the icon stays in whatever state it
  // was at the moment of opening" — freeze the icon while the popup is up; it
  // resumes when the popup closes (startTimer triggers a fresh updateTray).
  if (win?.isVisible()) return;
  const frame = daisyFrame(elapsedSinceTimerStart(), timerIntervalMs, PETAL_FADE_MS);
  tray.setImage(nativeImage.createFromBuffer(buildTrayIcon(frame)));
  tray.setToolTip(formatTooltip(remainingMsForTooltip()));
}

// Refresh at 50ms (~20fps) so the petal-fade animation is smooth. The
// daisy is a tiny 64×64 PNG — encoding cost per frame is negligible, and
// the loop bails out instantly while the popup is open. Cleared on will-quit
// so it can't fire against a destroyed tray during shutdown.
const trayRefreshId = setInterval(updateTray, 50);

// --- Window ---

function createWindow() {
  // Spec §1/§2: a frameless, frosted-glass card showing the blurred desktop
  // behind it. On Windows that "blur the desktop" effect is the DWM's Acrylic
  // backdrop (`backgroundMaterial: "acrylic"`) — and Acrylic only composites if
  // the window is NOT `transparent`. Setting both (the old config) makes the
  // backdrop fail to paint until something forces a repaint — the popup renders
  // as a gray/half rectangle on first show and only "snaps" correct after you
  // interact (Tab). So on Windows we drop `transparent` and let Acrylic own the
  // backdrop; everywhere else (dev/Linux, no Acrylic) we keep a real
  // transparent window. Acrylic is a no-op off-Windows, so this changes nothing
  // there.
  const isWindows = process.platform === "win32";
  win = new BrowserWindow({
    width: 480,
    height: 170,
    useContentSize: true,
    frame: false,
    transparent: !isWindows,
    alwaysOnTop: true,
    resizable: false,
    skipTaskbar: true,
    show: false,
    backgroundColor: "#00000000",
    backgroundMaterial: isWindows ? "acrylic" : undefined,
    roundedCorners: true,
    hasShadow: false,
    paintWhenInitiallyHidden: true,
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });

  if (isDev) {
    win.loadURL("http://localhost:5173");
  } else {
    win.loadFile(path.join(__dirname, "../renderer/index.html"));
  }
}

// --- IPC ---

ipcMain.handle("nudge:save", (_event, data) => {
  const minutes = parseMinutes(data?.nextMinutes);
  const doing = String(data?.doing ?? "");
  const bullshit = String(data?.bullshit ?? "");
  // Spec §4: Enter with both fields empty is the "change interval without
  // journaling" path. Hide and restart the timer, but skip writeEvent — the
  // journal stays clean.
  if (isJournalable(doing, bullshit)) {
    const event: JournalEvent = {
      schema_version: 1,
      event_type: "submitted",
      entry_id: ulid(),
      captured_at: formatTimestamp(),
      implementation: "electron",
      trigger_source: lastTriggerSource,
      doing,
      bullshit,
      next_interval_minutes: minutes,
    };
    try {
      writeEvent(event);
    } catch (err) {
      if (err instanceof JournalValidationError) {
        dialog.showErrorBox(
          "Nudge: journal validation error",
          `${err.message}\n\nThis is a bug — please report it. Your entry was not saved.`,
        );
      } else {
        dialog.showErrorBox(
          "Nudge: could not write journal",
          `${getJournalPath()}\n\n${(err as Error).message}`,
        );
      }
      throw err; // propagate → renderer await rejects → form stays visible
    }
  }
  hideNudge();
  startTimer(minutes);
});

// Esc + Switch share one §4 row: hide, preserve doing/bullshit, leave the
// timer alone — *unless* the popup was opened by the timer itself. In that
// case the deadline is already at zero, and skipping a restart would re-open
// the popup instantly. Manually-opened popups still have a live timer behind
// them, so we must not reset it.
function dismissOrSwitch(action: CloseAction, data: unknown) {
  hideNudge();
  if (restartsTimer(action, lastTriggerSource)) {
    startTimer(parseMinutes((data as { nextMinutes?: unknown } | undefined)?.nextMinutes));
  }
}

ipcMain.handle("nudge:dismiss", (_event, data) => dismissOrSwitch("dismiss", data));
ipcMain.handle("nudge:switch", (_event, data) => dismissOrSwitch("switch", data));

// --- Settings IPC (spec §9) ---

ipcMain.handle("settings:get-config", () => currentConfig);

ipcMain.handle("settings:save", (_event, raw) => {
  // The renderer's SettingsForm already validated; we stay defensive against a
  // malformed payload by coercing each field.
  const r = (raw ?? {}) as Partial<Config>;
  const config: Config = {
    hotkey: String(r.hotkey ?? currentConfig.hotkey),
    default_interval_minutes: parseMinutes(r.default_interval_minutes),
    autostart: Boolean(r.autostart),
  };
  saveConfig(getConfigPath(), config);
  // Apply immediately so the live hotkey updates without waiting for the
  // watcher (which will also fire, but diff against currentConfig → no-op).
  applyConfig(config);
});

ipcMain.handle("settings:toggle-autostart", (_event, desired: boolean) => {
  const staged: Config = { ...currentConfig, autostart: !!desired };
  const result = applyAutostart(autostartProvider, !!desired, () => {
    saveConfig(getConfigPath(), staged);
  });
  if (result.ok) {
    applyConfig(staged);
    return { ok: true as const };
  }
  return {
    ok: false as const,
    error:
      result.error.kind === "backend"
        ? result.error.message
        : String(result.error.cause),
  };
});

ipcMain.on("settings:close", () => settingsWin?.close());

// --- App lifecycle ---

app.whenReady().then(() => {
  // Load persisted config first so the timer interval and global hotkey
  // reflect the user's settings from the very first launch.
  const { config, error } = loadConfig(getConfigPath());
  if (error) {
    console.error("Nudge: config load failed, using defaults:", error);
  }
  currentConfig = config;

  createWindow();
  createTray();
  registerHotkey(currentConfig);
  startConfigWatcher();
  // First-launch rule (spec §4): show popup immediately, start timer only
  // after the first close (Enter / Esc / Switch). Tray tooltip stays on the
  // default interval until then.
  win?.once("ready-to-show", () => showNudge(INITIAL_TRIGGER_SOURCE));
});

app.on("will-quit", () => {
  clearInterval(trayRefreshId);
  globalShortcut.unregisterAll();
  configWatcher?.close();
});

// Native e2e affordance (gated on NUDGE_E2E): expose the Settings opener on the
// main-process global so a test can open it via electronApp.evaluate() without
// having to synthesize a tray click (which Electron exposes no API for). No
// effect in production where NUDGE_E2E is unset.
if (process.env.NUDGE_E2E) {
  (globalThis as Record<string, unknown>).__nudgeTest = {
    openSettings: () => openSettings(),
    // Reproduce the quit-time crash deterministically: hide the popup (so the
    // updateTray loop doesn't early-out), destroy the tray, then tick the
    // refresh loop once. With the guard in updateTray this is a no-op; without
    // it, `tray.setImage` on a destroyed Tray throws "Object has been destroyed".
    tickTrayAfterDestroy: () => {
      win?.hide();
      tray?.destroy();
      updateTray();
    },
    getState: () => ({
      lastTriggerSource,
      timerArmed: timerId !== null,
      popupVisible: !!win?.isVisible(),
    }),
  };
}

// App lives in tray — don't quit when window is hidden
app.on("window-all-closed", () => {
  // no-op
});
