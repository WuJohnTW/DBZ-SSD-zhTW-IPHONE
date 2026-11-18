// Lists on-screen windows for run_wine_tests.py --gui: "<window id>\t<owner pid>\t<x>,<y>,<w>,<h>\t<title>" per line.
// The test screenshots one Wine window with `screencapture -l <window id>` (the window alone, not the screen).
// Reading other apps' window titles needs the Screen Recording permission; this tool never asks for it.
import CoreGraphics
import Foundation

let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
guard let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else { exit(1) }
for w in list {
    let id = w[kCGWindowNumber as String] as? Int ?? 0
    let pid = w[kCGWindowOwnerPID as String] as? Int ?? 0
    let name = (w[kCGWindowName as String] as? String ?? "").replacingOccurrences(of: "\t", with: " ")
    var r = CGRect.zero
    if let b = w[kCGWindowBounds as String] as? NSDictionary as CFDictionary? { CGRectMakeWithDictionaryRepresentation(b, &r) }
    print("\(id)\t\(pid)\t\(Int(r.minX)),\(Int(r.minY)),\(Int(r.width)),\(Int(r.height))\t\(name)")
}
