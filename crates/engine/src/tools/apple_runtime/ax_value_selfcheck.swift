// A self-check for the accessibility value write the helper uses.
//
// The helper puts text into a field with `AXUIElementSetAttributeValue`, because a value is an
// attribute and not an action. This runs a window of its own and checks that the write works:
// the field holds "before", the value is written, and it holds "after".
//
// Run it where the Accessibility permission is granted:
//
//   xcrun swiftc -O -o ax_value_selfcheck ax_value_selfcheck.swift && ./ax_value_selfcheck
//
// It prints:
//
//   value before: before
//   set: true
//   value after: after

import ApplicationServices
import AppKit

// A tiny window with a text field, in this process.
final class Delegate: NSObject, NSApplicationDelegate {
    var field: NSTextField!
    func applicationDidFinishLaunching(_ n: Notification) {
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 300, height: 120),
                         styleMask: [.titled], backing: .buffered, defer: false)
        w.title = "AXSelfTest"
        field = NSTextField(string: "")
        field.identifier = NSUserInterfaceItemIdentifier("theField")
        field.stringValue = "before"
        w.contentView?.addSubview(field)
        w.makeKeyAndOrderFront(nil)
        app.activate(ignoringOtherApps: true)
        // Give the AX tree a moment.
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
            let pid = getpid()
            let app = AXUIElementCreateApplication(pid)
            var v: CFTypeRef?
            let got = AXUIElementCopyAttributeValue(app, kAXChildrenAttribute as CFString, &v)
            print("children:", got == .success, (v as? [AXUIElement])?.count ?? -1)
            // Walk for the text field and try to set its value.
            func find(_ el: AXUIElement) -> AXUIElement? {
                var id: CFTypeRef?
                _ = AXUIElementCopyAttributeValue(el, kAXIdentifierAttribute as CFString, &id)
                if (id as? String) == "theField" { return el }
                var kids: CFTypeRef?
                _ = AXUIElementCopyAttributeValue(el, kAXChildrenAttribute as CFString, &kids)
                for k in (kids as? [AXUIElement]) ?? [] {
                    if let f = find(k) { return f }
                }
                return nil
            }
            if let f = find(app) {
                var before: CFTypeRef?
                _ = AXUIElementCopyAttributeValue(f, kAXValueAttribute as CFString, &before)
                print("value before:", before ?? "nil")
                let text: CFTypeRef = "after" as NSString
                let set = AXUIElementSetAttributeValue(f, kAXValueAttribute as CFString, text)
                print("set:", set == .success)
                var after: CFTypeRef?
                _ = AXUIElementCopyAttributeValue(f, kAXValueAttribute as CFString, &after)
                print("value after:", after ?? "nil")
            } else {
                print("field not found")
            }
            exit(0)
        }
        app.run()
    }
}
let d = Delegate()
NSApplication.shared.delegate = d
NSApplication.shared.run()
