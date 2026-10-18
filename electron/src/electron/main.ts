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
import { watch, readFileSync, type FSWatcher } from "node:fs";
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
  type SpriteConfig,
} from "./config";
import { normalizeSprite } from "../shared/config";
import { parseHotkey, hotkeyToAccelerator } from "../shared/hotkey";
import {
  restartsTimer,
  INITIAL_TRIGGER_SOURCE,
  type TriggerSource,
  type CloseAction,
} from "../shared/nudgeFlow";
import { applyAutostart } from "../shared/autostart";
import {
  CARD_WIDTH,
  CARD_HEIGHT,
  SPRITE_ERROR,
  checkSpritePng,
  spriteLayout,
  windowOrigin,
  type Rect,
  type Size,
  type SpriteView,
} from "../shared/sprite";
import { ElectronAutostartProvider } from "./autostart";
import { buildTrayIcon, daisyFrame, formatTooltip, type DaisyFrame } from "./trayIcon";

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
// The sprite the popup window is laid out for; null = plain card.
let sprite: SpriteView | null = null;
// A saved sprite config not yet applied: it waits until the popup is hidden.
let pendingSprite: SpriteConfig | null = null;
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

// Register the global hotkey from a config. Non-fatal per spec §5: a
// combination the toolkit refuses logs to stderr and leaves the app running
// with no hotkey (the tray still works), and never falls back to the default.
//
// What this cannot do is report a combination another application holds. §5
// measured that on macOS registration succeeds anyway and the keystroke simply
// goes to the other holder; Electron adds nothing here — `globalShortcut`
// wraps the same call, so a `false` return means "the toolkit would not take
// this string", not "someone else has it". Detecting a conflict needs the
// system's own shortcut table (`CopySymbolicHotKeys`), which Electron does not
// expose. The Rust app does that work; the prototype states the gap instead of
// faking a check.
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
      console.error(`Nudge: hotkey "${accel}" was refused by the toolkit`);
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
  if (changed.sprite) {
    pendingSprite = next.sprite;
    applyPendingSprite();
  }
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
  // Key status arrives asynchronously after focus(), so wait for it; if it
  // does not come, ask once more with the app's activation behind it.
  const shown = win;
  void (async () => {
    let gotFocus = await becameFocused(shown);
    if (!gotFocus && !shown.isDestroyed()) {
      app.focus({ steal: true });
      shown.focus();
      gotFocus = await becameFocused(shown);
    }
    if (!shown.isDestroyed()) shown.webContents.send("nudge:show", { gotFocus });
  })();
}

const FOCUS_WAIT_MS = 300;

function becameFocused(w: BrowserWindow): Promise<boolean> {
  if (w.isDestroyed()) return Promise.resolve(false);
  if (w.isFocused()) return Promise.resolve(true);
  return new Promise((resolve) => {
    const done = (v: boolean) => {
      clearTimeout(timer);
      w.removeListener("focus", onFocus);
      resolve(v);
    };
    const onFocus = () => done(true);
    const timer = setTimeout(() => done(!w.isDestroyed() && w.isFocused()), FOCUS_WAIT_MS);
    w.on("focus", onFocus);
  });
}

// Spec §1: horizontally centred on the monitor, top edge at 25% of its height
// plus 80 pt — the card grows downward from there. With a sprite the window
// spans card ∪ sprite and the card sits inside it at its layout offset; a
// sprite that would rise above the work area pins the window to its top and
// the card moves down (`windowOrigin`).
function positionCard() {
  if (!win) return;
  const cursor = screen.getCursorScreenPoint();
  const display = screen.getDisplayNearestPoint(cursor);
  const { x, y } = windowOrigin(display, cardRect());
  win.setPosition(x, y);
}

function hideNudge() {
  if (!win) return;
  win.hide();
  applyPendingSprite();
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

// A menu bar icon on macOS is 18 pt; a notification-area icon on Windows is
// 16 px at 100 % DPI (SM_CXSMICON) and grows with the DPI.
const MAC_TRAY_ICON_POINTS = 18;
const WIN_TRAY_ICON_PX = 16;

// The daisy drawn once per density the host will display it at, so nothing
// downstream resamples. macOS: a 1x and a 2x representation of an 18 pt
// image, AppKit picks by the display's backing scale. Windows: Electron
// turns the 1x bitmap into the HICON at its own pixel size and the shell
// draws that at the tray's size, so the one representation is drawn at
// exactly that size.
function trayImage(frame: number | DaisyFrame) {
  const img = nativeImage.createEmpty();
  if (process.platform === "darwin") {
    for (const scale of [1, 2]) {
      img.addRepresentation({ scaleFactor: scale, buffer: buildTrayIcon(frame, MAC_TRAY_ICON_POINTS * scale) });
    }
  } else {
    const px = Math.round(WIN_TRAY_ICON_PX * screen.getPrimaryDisplay().scaleFactor);
    img.addRepresentation({ scaleFactor: 1, buffer: buildTrayIcon(frame, px) });
  }
  return img;
}

function createTray() {
  tray = new Tray(trayImage(0));
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
    // useContentSize: the 460×640 is the *client* area, so the OS title bar
    // doesn't eat into it and clip the form (the bug was the title bar
    // shrinking a frame-inclusive height on Windows).
    width: 460,
    // Tall enough for the vocabulary line under the hotkey field (§9): it is
    // standing information, so it must fit without scrolling — and for the
    // sprite section below "Launch at login".
    height: 740,
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

  // The window's own focus loss, from the OS. The renderer also listens for a
  // `blur` on its `window` (the browser sub-app has nothing else), but that
  // event is suppressed whenever a test harness pins the page as focused —
  // and the recorder's focus-lost cell has to hold in the real app, where the
  // shortcut that stole the focus is exactly what cannot be observed.
  settingsWin.on("blur", () => {
    settingsWin?.webContents.send("settings:blur");
  });

  settingsWin.on("closed", () => {
    settingsWin = null;
  });
}

function updateTray() {
  if (!tray) return;
  // Spec §5: "While the popup is open the icon stays in whatever state it
  // was at the moment of opening" — freeze the icon while the popup is up; it
  // resumes when the popup closes (startTimer triggers a fresh updateTray).
  if (win?.isVisible()) return;
  const frame = daisyFrame(elapsedSinceTimerStart(), timerIntervalMs, PETAL_FADE_MS);
  tray.setImage(trayImage(frame));
  tray.setToolTip(formatTooltip(remainingMsForTooltip()));
}

// Refresh at 50ms (~20fps) so the petal-fade animation is smooth. The
// daisy is a tiny 64×64 PNG — encoding cost per frame is negligible, and
// the loop bails out instantly while the popup is open.
const trayRefreshId = setInterval(updateTray, 50);

// The only way the tray goes away. Stopping the refresh first matters: a
// Tray being torn down can still answer isDestroyed() false while its
// setImage() already throws, so a tick that lands mid-destroy would crash.
function destroyTray() {
  clearInterval(trayRefreshId);
  tray?.destroy();
  tray = null;
}

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
  const isMac = process.platform === "darwin";
  // A sprite needs a see-through window around the card, which Acrylic rules
  // out (see above): with a sprite the window is transparent everywhere.
  // TODO: the transparent area around the card still eats clicks — toggle
  // setIgnoreMouseEvents(…, { forward: true }) on card hover.
  const acrylic = isWindows && !sprite;
  const size = windowSize();
  win = new BrowserWindow({
    // macOS: a non-activating panel — it takes the keyboard without
    // activating the app, so another app grabbing activation cannot leave the
    // card visible but deaf (spec §1).
    type: isMac ? "panel" : undefined,
    width: size.width,
    height: size.height,
    useContentSize: true,
    frame: false,
    transparent: !acrylic,
    alwaysOnTop: true,
    resizable: false,
    skipTaskbar: true,
    show: false,
    backgroundColor: "#00000000",
    backgroundMaterial: acrylic ? "acrylic" : undefined,
    roundedCorners: true,
    hasShadow: false,
    paintWhenInitiallyHidden: true,
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });
  if (isMac) {
    win.setVisibleOnAllWorkspaces(true, {
      visibleOnFullScreen: true,
      skipTransformProcessType: true,
    });
  }

  if (isDev) {
    win.loadURL("http://localhost:5173");
  } else {
    win.loadFile(path.join(__dirname, "../renderer/index.html"));
  }
}

// --- Sprite ---

function cardRect(): Rect {
  return sprite?.layout.card ?? { x: 0, y: 0, width: CARD_WIDTH, height: CARD_HEIGHT };
}

function windowSize(): Size {
  return sprite?.layout.window ?? { width: CARD_WIDTH, height: CARD_HEIGHT };
}

type SpriteFile =
  | { ok: true; bytes: Buffer; width: number; height: number }
  | { ok: false; error: string };

/** Read and validate a sprite PNG ("a PNG without a background"). */
async function readSpriteFile(file: string): Promise<SpriteFile> {
  if (file === "") return { ok: false, error: SPRITE_ERROR.noPath };
  let bytes: Buffer;
  try {
    bytes = readFileSync(file);
  } catch {
    return { ok: false, error: `${SPRITE_ERROR.notFound}: ${file}` };
  }
  const check = await checkSpritePng(bytes, (png) => {
    const image = nativeImage.createFromBuffer(Buffer.from(png));
    return image.isEmpty() ? null : image.toBitmap();
  });
  return check.ok ? { ...check, bytes } : check;
}

/** The sprite to draw for a config, or null when it is off or unusable. */
async function spriteView(cfg: SpriteConfig): Promise<SpriteView | null> {
  if (!cfg.enabled) return null;
  const file = await readSpriteFile(cfg.path);
  if (!file.ok) {
    console.error(`Nudge: sprite not shown: ${file.error}`);
    return null;
  }
  return {
    url: "data:image/png;base64," + file.bytes.toString("base64"),
    layout: spriteLayout(
      { width: cfg.card_width, height: CARD_HEIGHT },
      { x: cfg.x, y: cfg.y, width: file.width, height: file.height },
    ),
  };
}

// Applies run one at a time, so a slow read can't overwrite a newer config.
let spriteApply = Promise.resolve();

/**
 * Apply a saved sprite change once the popup is hidden, so it never reshapes
 * under the user. Turning the sprite on or off recreates the window, because
 * `transparent` (and with it Acrylic) is fixed at creation; any other change
 * resizes the window and pushes the new layout to the renderer.
 */
function applyPendingSprite(): void {
  spriteApply = spriteApply.then(async () => {
    if (!pendingSprite || !win || win.isVisible()) return;
    const cfg = pendingSprite;
    pendingSprite = null;
    const next = await spriteView(cfg);
    if (win.isVisible()) {
      pendingSprite ??= cfg; // shown meanwhile: retry on the next hide
      return;
    }
    const recreate = !!sprite !== !!next;
    sprite = next;
    if (recreate) {
      win.destroy();
      createWindow();
    } else {
      const size = windowSize();
      win.setContentSize(size.width, size.height);
      win.webContents.send("nudge:sprite", sprite);
    }
  });
}

ipcMain.handle("nudge:sprite", () => sprite);

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

ipcMain.handle("settings:save", async (_event, raw) => {
  // The renderer's SettingsForm already validated; we stay defensive against a
  // malformed payload by coercing each field.
  const r = (raw ?? {}) as Partial<Config>;
  const config: Config = {
    hotkey: String(r.hotkey ?? currentConfig.hotkey),
    default_interval_minutes: parseMinutes(r.default_interval_minutes),
    autostart: Boolean(r.autostart),
    sprite: normalizeSprite(r.sprite),
  };
  // Only the file check lives here: an enabled sprite must be a usable PNG.
  if (config.sprite.enabled) {
    const file = await readSpriteFile(config.sprite.path);
    if (!file.ok) return { ok: false as const, error: file.error };
  }
  saveConfig(getConfigPath(), config);
  // Apply immediately so the live hotkey updates without waiting for the
  // watcher (which will also fire, but diff against currentConfig → no-op).
  applyConfig(config);
  return { ok: true as const };
});

ipcMain.handle("settings:browse-sprite", async () => {
  const options: Electron.OpenDialogOptions = {
    title: "Choose a sprite",
    filters: [{ name: "PNG image", extensions: ["png"] }],
    properties: ["openFile"],
  };
  const result = settingsWin
    ? await dialog.showOpenDialog(settingsWin, options)
    : await dialog.showOpenDialog(options);
  return result.canceled ? null : (result.filePaths[0] ?? null);
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

app.whenReady().then(async () => {
  // macOS: a menu-bar agent — no Dock icon, no Cmd-Tab entry (spec §1).
  if (process.platform === "darwin") {
    app.setActivationPolicy("accessory");
    app.dock?.hide();
  }
  // Load persisted config first so the timer interval and global hotkey
  // reflect the user's settings from the very first launch.
  const { config, error } = loadConfig(getConfigPath());
  if (error) {
    console.error("Nudge: config load failed, using defaults:", error);
  }
  currentConfig = config;

  sprite = await spriteView(currentConfig.sprite);
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
  destroyTray();
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
    show: () => showNudge("manual"),
    // Hide the popup (so the refresh does not early-out), take the tray down
    // the way quit does, then tick the refresh once: it must be a no-op.
    tickTrayAfterDestroy: () => {
      win?.hide();
      destroyTray();
      updateTray();
    },
    // The image handed to the tray, by representation: what the host picks
    // from, before it draws anything.
    trayImageReps: () => {
      const img = trayImage(0);
      // getSize() answers in points for every representation; the bitmap's
      // own pixel count is what says whether the 2x one is really 36 px.
      return img.getScaleFactors().map((scale) => ({
        scale,
        points: img.getSize(scale).width,
        pixels: Math.round(Math.sqrt(img.toBitmap({ scaleFactor: scale }).length / 4)),
      }));
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
