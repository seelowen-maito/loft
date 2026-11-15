#if DEBUG
import AppKit
import LoftKit

/// `-demo -render-app-store <folder>`: the Mac App Store screenshots (#490), drawn from the demo's sample data at exactly
/// 2880 × 1800 px (a 1440 × 900 pt window at 2×), most telling first, then quits. `scripts/appstore-mac.sh` runs it.
/// The window's sidebar of sections is hidden for them: its glass draws blank offscreen.
enum AppStoreShots {
    static var isRendering: Bool { ProcessInfo.processInfo.arguments.contains("-render-app-store") }
    static let size = NSSize(width: 1440, height: 900)

    @MainActor
    static func render(store: LoftStore, nav: MacNav, quickSearch: QuickSearchController, voicePanel: VoicePanelController, to folder: String) async {
        for _ in 0..<80 where store.agents.isEmpty || store.needsYou == nil { try? await Task.sleep(for: .milliseconds(100)) }
        for _ in 0..<50 where quickSearch.showWindow == nil { try? await Task.sleep(for: .milliseconds(100)) }
        quickSearch.showWindow?()
        NSApp.activate(ignoringOtherApps: true)
        var main: NSWindow?
        for _ in 0..<50 {
            main = NSApp.windows.first { $0.isVisible && $0.frame.width > 600 }
            if main != nil { break }
            try? await Task.sleep(for: .milliseconds(100))
        }
        guard let window = main else { return NSApp.terminate(nil) }
        window.setFrame(NSRect(origin: window.frame.origin, size: size), display: true)
        func settle() async { try? await Task.sleep(for: .seconds(2)) }
        func png(_ name: String, over panels: [NSWindow] = []) {
            let out = rep(size)
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: out)
            if let frameView = window.contentView?.superview {
                draw(frameView, in: NSRect(origin: .zero, size: size))
            }
            for p in panels {
                guard let view = p.contentView else { continue }
                // Its layer, not cacheDisplay, which paints the panel's clear edges white.
                guard let layer = view.layer, let cg = NSGraphicsContext.current?.cgContext else { continue }
                cg.saveGState()
                let origin = NSPoint(x: p.frame.minX - window.frame.minX, y: p.frame.minY - window.frame.minY)
                cg.translateBy(x: origin.x, y: origin.y + p.frame.height)
                cg.scaleBy(x: 1, y: -1)
                layer.render(in: cg)
                cg.restoreGState()
            }
            NSGraphicsContext.restoreGraphicsState()
            try? out.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: folder).appendingPathComponent("\(name).png"))
        }

        // 1. An agent's chat beside the agents list: the Lead, whose demo chat is the longest.
        nav.openChat(store.orchestratorId ?? store.agents.first?.id ?? "")
        await settle()
        png("1-chat")
        // 2. The Issues board with the specs' columns.
        nav.section = .issues
        await settle()
        png("2-issues")
        // 3. Needs you.
        nav.section = .needsYou
        await settle()
        png("3-needs-you")
        // 4. Quick search over the chat, where Spotlight would put it.
        nav.openChat(store.orchestratorId ?? store.agents.first?.id ?? "")
        await settle()
        quickSearch.show()
        quickSearch.model.query = "agent"
        if let panel = NSApp.windows.first(where: { $0 is NSPanel && $0.isVisible }) {
            panel.setFrameTopLeftPoint(NSPoint(x: window.frame.midX - panel.frame.width / 2, y: window.frame.maxY - size.height / 5))
            await settle()
            png("4-quick-search", over: [panel])
        }
        quickSearch.close()
        // 5. The voice panel over the chat, with Loft's cleaned-up text and its "Send to …?" suggestion.
        await showVoicePanel(voicePanel, store: store, fixture: "review")
        if let panel = NSApp.windows.first(where: { $0 is NSPanel && $0.isVisible }) {
            panel.setFrameTopLeftPoint(NSPoint(x: window.frame.midX - panel.frame.width / 2, y: window.frame.maxY - 180))
            await settle()
            png("5-voice-panel", over: [panel])
        }
        voicePanel.close()
        // 6. Overview.
        nav.section = .overview
        await settle()
        png("6-overview")
        NSApp.terminate(nil)
    }

    /// A bitmap of `size` points at 2×.
    private static func rep(_ size: NSSize) -> NSBitmapImageRep {
        let r = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(size.width * 2), pixelsHigh: Int(size.height * 2), bitsPerSample: 8,
                                 samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        r.size = size
        return r
    }

    /// Draws `view` at 2× into `rect` of the current context.
    private static func draw(_ view: NSView, in rect: NSRect) {
        let r = rep(view.bounds.size)
        view.cacheDisplay(in: view.bounds, to: r)
        r.draw(in: rect)
    }
}
#endif
