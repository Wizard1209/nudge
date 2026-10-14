// Real OS input for the macOS native tests: CGEvents posted at the HID tap,
// so they travel the path a user's mouse and keyboard take (key-window
// routing, global hotkeys, the WindowManager's wallpaper click).
//   click X Y                  left click at (X, Y) in global screen points
//   key CODE [ctrl|shift|alt|cmd ...]   key down + up by virtual keycode
//   activate BUNDLE_ID [MS]    activate the app; with MS, repeat every MS ms until killed
//   front                      pid of the frontmost application
//   windows                    on-screen windows, one JSON object per line
//   locked                     1 if the session's screen is locked, else 0
import AppKit
import CoreGraphics

let args = Array(CommandLine.arguments.dropFirst())
let src = CGEventSource(stateID: .hidSystemState)

func post(_ e: CGEvent?) { e?.post(tap: .cghidEventTap) }

func click(_ p: CGPoint) {
  post(CGEvent(mouseEventSource: src, mouseType: .mouseMoved, mouseCursorPosition: p, mouseButton: .left))
  usleep(100_000)
  post(CGEvent(mouseEventSource: src, mouseType: .leftMouseDown, mouseCursorPosition: p, mouseButton: .left))
  usleep(60_000)
  post(CGEvent(mouseEventSource: src, mouseType: .leftMouseUp, mouseCursorPosition: p, mouseButton: .left))
}

func key(_ code: CGKeyCode, _ mods: [String]) {
  var flags: CGEventFlags = []
  for m in mods {
    switch m {
    case "ctrl": flags.insert(.maskControl)
    case "shift": flags.insert(.maskShift)
    case "alt": flags.insert(.maskAlternate)
    case "cmd": flags.insert(.maskCommand)
    default: fatalError("unknown modifier \(m)")
    }
  }
  for down in [true, false] {
    let e = CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: down)
    e?.flags = flags
    post(e)
    usleep(30_000)
  }
}

func activate(_ bundle: String) {
  NSRunningApplication.runningApplications(withBundleIdentifier: bundle).first?.activate()
}

func windows() {
  let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
  for w in list {
    let b = w[kCGWindowBounds as String] as? [String: Double] ?? [:]
    let o: [String: Any] = [
      "owner": w[kCGWindowOwnerName as String] ?? "", "name": w[kCGWindowName as String] ?? "",
      "pid": w[kCGWindowOwnerPID as String] ?? 0, "layer": w[kCGWindowLayer as String] ?? 0,
      "x": b["X"] ?? 0, "y": b["Y"] ?? 0, "width": b["Width"] ?? 0, "height": b["Height"] ?? 0,
    ]
    let data = try! JSONSerialization.data(withJSONObject: o, options: [.sortedKeys])
    print(String(data: data, encoding: .utf8)!)
  }
}

switch args.first {
case "click": click(CGPoint(x: Double(args[1])!, y: Double(args[2])!))
case "key": key(CGKeyCode(args[1])!, Array(args.dropFirst(2)))
case "activate" where args.count > 2:
  signal(SIGTERM) { _ in exit(0) }
  while true { activate(args[1]); usleep(useconds_t(Int(args[2])! * 1000)) }
case "activate": activate(args[1])
case "front": print(NSWorkspace.shared.frontmostApplication?.processIdentifier ?? -1)
case "windows": windows()
case "locked":
  let d = CGSessionCopyCurrentDictionary() as? [String: Any]
  print((d?["CGSSessionScreenIsLocked"] as? Bool ?? false) ? 1 : 0)
default: fatalError("usage: click X Y | key CODE [mods] | activate BUNDLE [MS] | front | windows | locked")
}
