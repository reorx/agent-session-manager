// Print the CGWindowID of the first on-screen window owned by <process name>.
// With --bounds, print "id x y width height" (screen points) instead, for click automation.
import CoreGraphics
import Foundation
let name = CommandLine.arguments[1]
let wantBounds = CommandLine.arguments.contains("--bounds")
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as! [[String: Any]]
for w in list {
    if let owner = w[kCGWindowOwnerName as String] as? String, owner == name,
       let id = w[kCGWindowNumber as String] as? Int,
       let bounds = w[kCGWindowBounds as String] as? [String: Any],
       (bounds["Height"] as? Double ?? 0) > 100 {
        if wantBounds {
            let x = Int(bounds["X"] as? Double ?? 0), y = Int(bounds["Y"] as? Double ?? 0)
            let wd = Int(bounds["Width"] as? Double ?? 0), ht = Int(bounds["Height"] as? Double ?? 0)
            print("\(id) \(x) \(y) \(wd) \(ht)")
        } else {
            print(id)
        }
        exit(0)
    }
}
exit(1)
