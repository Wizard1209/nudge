import {
  app,
  BrowserWindow,
  dialog,
  globalShortcut,
  ipcMain,
  screen,
  shell,
  Tray,
  Menu,
  nativeImage,
} from "electron";
import path from "node:path";
import { watch, readFileSync, type FSWatcher } from "node:fs";
import { dirname, basename } from "node:path";
import { ulid } from "ulid";
import { LATEST_URL, REPO, describeCheckFailure, type Release } from "../shared/updateCheck";
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
import { displayFor } from "../shared/display";
import { parseHotkey, hotkeyToAccelerator } from "../shared/hotkey";
import {
  restartsTimer,
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
  spritePoints,
  windowOrigin,
  cardWidthOn,
  type Rect,
  type Size,
  type SpriteLayout,
  type SpriteView,
} from "../shared/sprite";
import { ElectronAutostartProvider } from "./autostart";
import { backdropAttached, backdropState, frontWindowBounds, hideBackdrop, syncBackdrop } from "./backdrop";
import { buildTrayIcon, daisyFrame, formatTooltip, type DaisyFrame } from "./trayIcon";

// Electron's own uncaughtException listener shows an error dialog and keeps the app running,
// unless another uncaughtException listener is registered.
// proof: https://github.com/electron/electron/blob/v41.2.0/lib/browser/init.ts#L21-L38
process.on("uncaughtException", (err) => {
  console.error("Nudge: uncaught exception in the main process", err);
  app.exit(1);
});

const isDev = !app.isPackaged && !process.env.NUDGE_FORCE_PROD;
const DEFAULT_MINUTES = 10;
const PETAL_FADE_MS = 250;

let win: BrowserWindow | null = null;
let settingsWin: BrowserWindow | null = null;
let spriteSource: SpriteSource | null = null;
let sprite: SpriteView | null = null;
let pendingSprite: SpriteConfig | null = null;
let tray: Tray | null = null;
let timerId: ReturnType<typeof setTimeout> | null = null;
let deadlineOwed = false;
let timerStartMs = 0;
let timerIntervalMs = 0;
let lastTriggerSource: TriggerSource = "timer";

const autostartProvider = new ElectronAutostartProvider();
let currentConfig: Config = { ...DEFAULT_CONFIG };
let registeredAccelerator: string | null = null;
let configWatcher: FSWatcher | null = null;
let watcherDebounce: ReturnType<typeof setTimeout> | null = null;

function configuredMinutes(): number {
  return parseMinutes(currentConfig.default_interval_minutes);
}

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

function applyConfig(next: Config): void {
  const changed = diffConfig(currentConfig, next);
  currentConfig = next;
  if (changed.hotkey) registerHotkey(next);
  if (changed.sprite) {
    pendingSprite = next.sprite;
    applyPendingSprite();
  }
}

// On Linux and macOS, fs.watch on a path follows its inode: after a new file is renamed over
// the path, events for that new file are not emitted.
// proof: https://nodejs.org/docs/v24.14.0/api/fs.md
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

function startTimer(minutes: number) {
  if (timerId) clearTimeout(timerId);
  deadlineOwed = false;
  timerIntervalMs = minutes * 60_000;
  timerStartMs = Date.now();
  timerId = setTimeout(deadlineDue, timerIntervalMs + PETAL_FADE_MS);
  updateTray();
}

function deadlineDue() {
  if (win?.isVisible()) {
    deadlineOwed = true;
    return;
  }
  deadlineOwed = false;
  showNudge("timer");
}

function showNudge(source: TriggerSource) {
  if (!win) return;
  if (win.isVisible() && win.isFocused()) return;
  lastTriggerSource = source;
  positionCard();
  win.show();
  win.focus();
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

function positionCard(display: Electron.Display = currentDisplay()) {
  if (!win) return;
  layoutSpriteFor(display);
  const { x, y } = windowOrigin(display, cardRect());
  win.setPosition(x, y);
  syncBackdrop(win, cardRect());
}

function currentDisplay(): Electron.Display {
  return displayFor(frontWindowBounds(), screen.getCursorScreenPoint(), screen.getAllDisplays());
}

function hideNudge() {
  if (!win) return;
  win.hide();
  hideBackdrop();
  applyPendingSprite();
}

function parseMinutes(value: unknown): number {
  const n = Number(value);
  if (!Number.isFinite(n) || n <= 0) return DEFAULT_MINUTES;
  return n;
}

function elapsedSinceTimerStart(): number {
  if (timerStartMs === 0) return 0;
  return Date.now() - timerStartMs;
}

function remainingMsForTooltip(): number {
  if (timerStartMs === 0) return configuredMinutes() * 60_000;
  return timerStartMs + timerIntervalMs - Date.now();
}

const MAC_TRAY_ICON_POINTS = 18;
const WIN_TRAY_ICON_PX = 16;

function trayImage(frame: number | DaisyFrame) {
  const img = nativeImage.createEmpty();
  if (process.platform === "darwin") {
    for (const scale of [1, 2]) {
      img.addRepresentation({ scaleFactor: scale, buffer: buildTrayIcon(frame, MAC_TRAY_ICON_POINTS * scale, { mark: true }) });
    }
  } else {
    const px = Math.round(WIN_TRAY_ICON_PX * screen.getPrimaryDisplay().scaleFactor);
    img.addRepresentation({ scaleFactor: 1, buffer: buildTrayIcon(frame, px, { mark: true }) });
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

const SETTINGS_SIZE = { width: 560, height: 672 };

function openSettings() {
  if (settingsWin) {
    settingsWin.focus();
    return;
  }
  settingsWin = new BrowserWindow({
    width: SETTINGS_SIZE.width,
    height: SETTINGS_SIZE.height,
    useContentSize: true,
    title: "Nudge — Settings",
    resizable: false,
    minimizable: false,
    maximizable: false,
    skipTaskbar: false,
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

  settingsWin.on("blur", () => {
    settingsWin?.webContents.send("settings:blur");
  });

  settingsWin.on("closed", () => {
    settingsWin = null;
  });
}

function updateTray() {
  if (!tray) return;
  if (win?.isVisible()) return;
  const frame = daisyFrame(elapsedSinceTimerStart(), timerIntervalMs, PETAL_FADE_MS);
  tray.setImage(trayImage(frame));
  tray.setToolTip(formatTooltip(remainingMsForTooltip()));
}

const trayRefreshId = setInterval(updateTray, 50);

function destroyTray() {
  clearInterval(trayRefreshId);
  tray?.destroy();
  tray = null;
}

function createWindow() {
  const isWindows = process.platform === "win32";
  const isMac = process.platform === "darwin";
  const acrylic = isWindows && !sprite;
  const size = windowSize();
  win = new BrowserWindow({
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
    hasShadow: isMac,
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
  syncBackdrop(win, cardRect());
}

function cardRect(): Rect {
  return sprite?.layout.card ?? { x: 0, y: 0, width: CARD_WIDTH, height: CARD_HEIGHT };
}

function windowSize(): Size {
  return sprite?.layout.window ?? { width: CARD_WIDTH, height: CARD_HEIGHT };
}

interface SpriteSource {
  url: string;
  png: Size;
  placement: SpriteConfig;
}

function spriteLayoutOn(source: SpriteSource, display: Electron.Display): SpriteLayout {
  const { card_width, x, y } = source.placement;
  return spriteLayout(
    { width: cardWidthOn(card_width, display.workArea), height: CARD_HEIGHT },
    { x, y, ...spritePoints(source.png, display.scaleFactor) },
  );
}

function sameLayout(a: SpriteLayout, b: SpriteLayout): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

function layoutSpriteFor(display: Electron.Display): void {
  if (!spriteSource || !sprite || !win) return;
  const layout = spriteLayoutOn(spriteSource, display);
  if (sameLayout(layout, sprite.layout)) return;
  sprite = { url: sprite.url, layout };
  win.setContentSize(layout.window.width, layout.window.height);
  win.webContents.send("nudge:sprite", sprite);
}

type SpriteFile =
  | { ok: true; bytes: Buffer; width: number; height: number }
  | { ok: false; error: string };

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

async function loadSpriteSource(cfg: SpriteConfig): Promise<SpriteSource | null> {
  if (!cfg.enabled) return null;
  const file = await readSpriteFile(cfg.path);
  if (!file.ok) {
    console.error(`Nudge: sprite not shown: ${file.error}`);
    return null;
  }
  return {
    url: "data:image/png;base64," + file.bytes.toString("base64"),
    png: { width: file.width, height: file.height },
    placement: cfg,
  };
}

function setSpriteSource(source: SpriteSource | null): void {
  spriteSource = source;
  sprite = source && { url: source.url, layout: spriteLayoutOn(source, currentDisplay()) };
}

let spriteApply = Promise.resolve();

// Electron reads a window's `transparent` option once, at construction, into a const field.
// proof: https://github.com/electron/electron/blob/v41.2.0/shell/browser/native_window.h#L499-L500
function applyPendingSprite(): void {
  spriteApply = spriteApply.then(async () => {
    if (!pendingSprite || !win || win.isVisible()) return;
    const cfg = pendingSprite;
    pendingSprite = null;
    const next = await loadSpriteSource(cfg);
    if (win.isVisible()) {
      pendingSprite ??= cfg;
      return;
    }
    const recreate = !!sprite !== !!next;
    setSpriteSource(next);
    if (recreate) {
      win.destroy();
      createWindow();
    } else {
      const size = windowSize();
      win.setContentSize(size.width, size.height);
      win.webContents.send("nudge:sprite", sprite);
      syncBackdrop(win, cardRect());
    }
  });
}

ipcMain.handle("nudge:sprite", () => sprite);

ipcMain.on("nudge:backdrop", (event) => {
  event.returnValue = backdropAttached();
});

ipcMain.handle("nudge:save", (_event, data) => {
  const minutes = parseMinutes(data?.nextMinutes);
  const doing = String(data?.doing ?? "");
  const bullshit = String(data?.bullshit ?? "");
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
      throw err;
    }
  }
  hideNudge();
  startTimer(minutes);
});

function dismissOrSwitch(action: CloseAction, data: unknown) {
  hideNudge();
  if (restartsTimer(action, lastTriggerSource)) {
    startTimer(parseMinutes((data as { nextMinutes?: unknown } | undefined)?.nextMinutes));
  } else if (deadlineOwed) {
    deadlineDue();
  }
}

ipcMain.handle("nudge:dismiss", (_event, data) => dismissOrSwitch("dismiss", data));
ipcMain.handle("nudge:switch", (_event, data) => dismissOrSwitch("switch", data));

ipcMain.handle("settings:get-config", () => currentConfig);

ipcMain.handle("settings:save", async (_event, raw) => {
  const r = (raw ?? {}) as Partial<Config>;
  const config: Config = {
    hotkey: String(r.hotkey ?? currentConfig.hotkey),
    default_interval_minutes: parseMinutes(r.default_interval_minutes),
    autostart: Boolean(r.autostart),
    sprite: normalizeSprite(r.sprite),
  };
  if (config.sprite.enabled) {
    const file = await readSpriteFile(config.sprite.path);
    if (!file.ok) return { ok: false as const, error: file.error };
  }
  saveConfig(getConfigPath(), config);
  applyConfig(config);
  return { ok: true as const };
});

let queuedBrowsePick: string | null = null;

ipcMain.handle("settings:browse-sprite", async () => {
  if (process.env.NUDGE_E2E && queuedBrowsePick !== null) {
    const picked = queuedBrowsePick;
    queuedBrowsePick = null;
    return picked;
  }
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

function appVersion(): string {
  if (app.isPackaged) return app.getVersion();
  return JSON.parse(readFileSync(path.join(__dirname, "../../package.json"), "utf8")).version;
}

ipcMain.handle("settings:update-info", () => ({ version: appVersion() }));

type CheckResult = { ok: true; release: Release } | { ok: false; error: string };
let queuedCheck: CheckResult | null = null;

ipcMain.handle("settings:check-update", async () => {
  if (process.env.NUDGE_E2E && queuedCheck !== null) {
    const result = queuedCheck;
    queuedCheck = null;
    return result;
  }
  let res: Response;
  try {
    res = await fetch(LATEST_URL, {
      headers: {
        Accept: "application/vnd.github+json",
        "User-Agent": `nudge/${appVersion()}`,
      },
    });
  } catch {
    return { ok: false as const, error: describeCheckFailure({ kind: "offline" }) };
  }
  if (!res.ok) {
    return { ok: false as const, error: describeCheckFailure({ kind: "status", status: res.status }) };
  }
  try {
    const body = (await res.json()) as Partial<Release>;
    if (typeof body.tag_name !== "string" || typeof body.html_url !== "string") throw new Error();
    const release: Release = {
      tag_name: body.tag_name,
      html_url: body.html_url,
      assets: Array.isArray(body.assets) ? body.assets : [],
    };
    return { ok: true as const, release };
  } catch {
    return { ok: false as const, error: describeCheckFailure({ kind: "unreadable" }) };
  }
});

const openedReleases: string[] = [];

ipcMain.handle("settings:open-release", (_event, url: string) => {
  if (typeof url !== "string" || !url.startsWith(`https://${REPO}/`)) return;
  if (process.env.NUDGE_E2E) openedReleases.push(url);
  else void shell.openExternal(url);
});

app.whenReady().then(async () => {
  if (process.platform === "darwin") {
    app.setActivationPolicy("accessory");
    app.dock?.hide();
  }
  const { config, error } = loadConfig(getConfigPath());
  if (error) {
    console.error("Nudge: config load failed, using defaults:", error);
  }
  currentConfig = config;

  setSpriteSource(await loadSpriteSource(currentConfig.sprite));
  createWindow();
  createTray();
  registerHotkey(currentConfig);
  startConfigWatcher();
  screen.on("display-metrics-changed", () => {
    if (win?.isVisible()) positionCard(screen.getDisplayMatching(win.getBounds()));
  });
  startTimer(configuredMinutes());
});

app.on("will-quit", () => {
  destroyTray();
  globalShortcut.unregisterAll();
  configWatcher?.close();
});

if (process.env.NUDGE_E2E) {
  (globalThis as Record<string, unknown>).__nudgeTest = {
    openSettings: () => openSettings(),
    queueCheck: (result: CheckResult) => {
      queuedCheck = result;
    },
    openedReleases: () => openedReleases,
    queueBrowsePick: (p: string) => {
      queuedBrowsePick = p;
    },
    show: () => showNudge("manual"),
    tickTrayAfterDestroy: () => {
      win?.hide();
      destroyTray();
      updateTray();
    },
    trayImageReps: () => {
      const img = trayImage(0);
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
    backdrop: () => backdropState(),
    frontWindowBounds: () => frontWindowBounds(),
    currentDisplay: () => currentDisplay(),
  };
}

app.on("window-all-closed", () => {
});
