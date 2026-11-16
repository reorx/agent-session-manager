// Print the CGWindowID of the first on-screen window owned by <process name>.
// With --bounds, print "id x y width height" (screen points) instead, for click automation.
// With --pid N, only a window of that process counts (another instance may share the name); the
// name "-" then matches any owner, so the process alone decides: an .app's windows are owned by
// its display name ("Agent Session Manager"), not by its binary's name.
// `winid --menu --pid N` prints "id x y width height" of that process's open menu (a window at
// the pop-up menu level or above); exit 1 when none is on screen.
// `winid --at X,Y` prints "pid owner" of the frontmost window under that point (screen points
// from the top-left of the main display): a script makes sure a click lands in its own app.
// `winid --screen` prints "width height" of the main display in points (menu bar and Dock strips).
import CoreGraphics
import Foundation

let args = CommandLine.arguments
func value(_ flag: String) -> String? {
    guard let i = args.firstIndex(of: flag), i + 1 < args.count else { return nil }
    return args[i + 1]
}
func number(_ any: Any?) -> Double { (any as? NSNumber)?.doubleValue ?? 0 }
func bounds(_ w: [String: Any]) -> CGRect {
    let b = w[kCGWindowBounds as String] as? [String: Any] ?? [:]
    return CGRect(x: number(b["X"]), y: number(b["Y"]), width: number(b["Width"]), height: number(b["Height"]))
}
func describe(_ id: Int, _ r: CGRect) -> String {
    "\(id) \(Int(r.minX)) \(Int(r.minY)) \(Int(r.width)) \(Int(r.height))"
}

if args.count > 1 && args[1] == "--screen" {
    let size = CGDisplayBounds(CGMainDisplayID()).size
    print("\(Int(size.width)) \(Int(size.height))")
    exit(0)
}
// front to back
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as! [[String: Any]]
let pid = value("--pid").flatMap { Int($0) }
func owner(_ w: [String: Any]) -> Int { Int(number(w[kCGWindowOwnerPID as String])) }
func windowID(_ w: [String: Any]) -> Int { Int(number(w[kCGWindowNumber as String])) }

if let at = value("--at") {
    let xy = at.split(separator: ",").compactMap { Double($0.trimmingCharacters(in: .whitespaces)) }
    guard xy.count == 2 else { exit(2) }
    let point = CGPoint(x: xy[0], y: xy[1])
    for w in list where number(w[kCGWindowAlpha as String]) > 0 && bounds(w).contains(point) {
        print("\(owner(w)) \(w[kCGWindowOwnerName as String] as? String ?? "")")
        exit(0)
    }
    exit(1)
}

if args.contains("--menu") {
    guard let pid else { exit(2) }
    // kCGPopUpMenuWindowLevel is 101; the menu bar item's own window is far below (25)
    for w in list where owner(w) == pid && number(w[kCGWindowLayer as String]) >= 100 {
        print(describe(windowID(w), bounds(w)))
        exit(0)
    }
    exit(1)
}

guard args.count > 1 else { exit(2) }
let name = args[1]
let wantBounds = args.contains("--bounds")
for w in list {
    let r = bounds(w)
    if name == "-" ? pid != nil : (w[kCGWindowOwnerName as String] as? String) == name,
       pid == nil || owner(w) == pid,
       number(w[kCGWindowLayer as String]) == 0,
       r.height > 100 {
        print(wantBounds ? describe(windowID(w), r) : String(windowID(w)))
        exit(0)
    }
}
exit(1)
