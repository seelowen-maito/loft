import AppKit
import Carbon.HIToolbox
import LoftKit
import SwiftUI

/// The Mac's three global shortcuts, recorded in Settings.
enum LoftShortcut: UInt32, CaseIterable {
    case quickSearch = 1, voicePanel, openLoft

    var pref: String {
        switch self {
        case .quickSearch: Prefs.quickSearchShortcut
        case .voicePanel: Prefs.voicePanelShortcut
        case .openLoft: Prefs.openLoftShortcut
        }
    }

    var fallback: MacShortcut? {
        switch self {
        case .quickSearch: .quickSearchDefault
        case .voicePanel: .voicePanelDefault
        case .openLoft: nil
        }
    }

    var name: String {
        switch self {
        case .quickSearch: "quick search"
        case .voicePanel: "voice panel"
        case .openLoft: "Open Loft"
        }
    }

    var current: MacShortcut? {
        #if DEBUG
        // UI tests (#569): only a shortcut passed as a launch argument (`-quickSearchShortcut control-option-command-40`),
        // never a default, so the app under test doesn't take the keys of the Loft open on this Mac.
        if ProcessInfo.processInfo.arguments.contains("-ui-testing") {
            return (UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)[pref] as? String).flatMap(MacShortcut.init(stored:))
        }
        #endif
        return MacShortcut.read(UserDefaults.standard.string(forKey: pref), default: fallback)
    }

    /// The window's menu shortcuts: a global one with the same keys would take them from Loft and every other app.
    static let menuKeys: [(MacShortcut, String)] =
        [(18, "Overview"), (19, "Agents"), (20, "Issues"), (21, "Needs you"), (23, "Settings"), (45, "New message"), (15, "Refresh"), (1, "Save")]
            .map { (MacShortcut(keyCode: $0.0, modifiers: MacShortcut.command), $0.1) }
            + [(MacShortcut(keyCode: 34, modifiers: MacShortcut.option | MacShortcut.command), "agent settings")]

    /// Why `s` can't be this shortcut, or nil: Loft or macOS already uses it, it has no modifier, or it can't be registered.
    func refusal(_ s: MacShortcut) -> String? {
        let others = Self.allCases.filter { $0 != self }.compactMap { o in o.current.map { ($0, o.name) } }
        if let why = s.refusal(taken: others + Self.menuKeys, system: Self.systemShortcuts()) { return why }
        return GlobalHotKey.canRegister(s) ? nil : "\(s.label) can't be a shortcut. Record another."
    }

    /// The shortcuts macOS keeps for itself and has on (Spotlight's ⌘Space, the input source's ⌃Space, Mission Control…).
    static func systemShortcuts() -> [MacShortcut] {
        var unmanaged: Unmanaged<CFArray>?
        guard CopySymbolicHotKeys(&unmanaged) == noErr, let all = unmanaged?.takeRetainedValue() as? [[String: Any]] else { return [] }
        return all.compactMap { d in
            guard (d[kHISymbolicHotKeyEnabled as String] as? Bool) == true, let code = d[kHISymbolicHotKeyCode as String] as? Int,
                  let mods = d[kHISymbolicHotKeyModifiers as String] as? Int else { return nil }
            return MacShortcut(keyCode: UInt32(code), modifiers: UInt32(mods) & (MacShortcut.command | MacShortcut.shift | MacShortcut.option | MacShortcut.control))
        }
    }
}

/// Settings' messages under a shortcut: one that couldn't be registered (macOS doesn't allow it, or Loft's other
/// shortcut held it), by hot key id, and a recorded combination that was refused. Also the one row recording now. Another app holding the same keys
/// isn't an error to Carbon: only one of the apps gets the presses.
@MainActor
@Observable
final class HotKeyFailures {
    static let shared = HotKeyFailures()
    var byID: [UInt32: MacShortcut] = [:]
    var refused: [UInt32: String] = [:]
    /// The hot key id whose row is recording: starting another row stops it.
    var recording: UInt32?
}

/// A system-wide shortcut through Carbon's hot keys: it works from any app and needs no Accessibility permission.
/// Each one has its own `id`, and passes the others' presses on.
final class GlobalHotKey {
    /// While Settings records a shortcut: gets the presses of Loft's own shortcuts, which never reach the window.
    @MainActor static var recorder: ((MacShortcut) -> Void)?

    private var ref: EventHotKeyRef?
    private var handler: EventHandlerRef?
    private var current: MacShortcut?
    private let id: UInt32
    private let action: () -> Void

    init(id: UInt32, action: @escaping () -> Void) {
        self.id = id
        self.action = action
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        InstallEventHandler(GetApplicationEventTarget(), { _, event, me in
            guard let event, let me else { return OSStatus(eventNotHandledErr) }
            let hotKey = Unmanaged<GlobalHotKey>.fromOpaque(me).takeUnretainedValue()
            var pressed = EventHotKeyID()
            GetEventParameter(event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID), nil,
                              MemoryLayout<EventHotKeyID>.size, nil, &pressed)
            guard pressed.id == hotKey.id else { return OSStatus(eventNotHandledErr) }
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    if let recorder = GlobalHotKey.recorder, let s = hotKey.current { recorder(s) } else { hotKey.action() }
                }
            }
            return noErr
        }, 1, &spec, Unmanaged.passUnretained(self).toOpaque(), &handler)
    }

    @MainActor
    func set(_ shortcut: MacShortcut?) {
        if let ref { UnregisterEventHotKey(ref) }
        ref = nil
        current = nil
        HotKeyFailures.shared.byID[id] = nil
        guard let shortcut else { return }
        let status = RegisterEventHotKey(shortcut.keyCode, shortcut.modifiers, EventHotKeyID(signature: Self.signature, id: id),
                                         GetApplicationEventTarget(), 0, &ref)
        if status == noErr { current = shortcut } else {
            ref = nil
            HotKeyFailures.shared.byID[id] = shortcut
        }
    }

    /// Whether macOS takes `s` as a hot key, tried and let go at once. Loft's own shortcuts are checked before this.
    static func canRegister(_ s: MacShortcut) -> Bool {
        var ref: EventHotKeyRef?
        guard RegisterEventHotKey(s.keyCode, s.modifiers, EventHotKeyID(signature: signature, id: 99), GetApplicationEventTarget(), 0, &ref) == noErr
        else { return false }
        if let ref { UnregisterEventHotKey(ref) }
        return true
    }

    private static let signature = OSType(0x4C_4F_46_54)
}

/// One of the global shortcuts, kept registered as Settings changes it.
@MainActor
final class ShortcutBinding {
    let shortcut: LoftShortcut
    private var hotKey: GlobalHotKey!
    private var applied: MacShortcut??

    init(_ shortcut: LoftShortcut, action: @escaping () -> Void) {
        self.shortcut = shortcut
        hotKey = GlobalHotKey(id: shortcut.rawValue, action: action)
        apply()
        NotificationCenter.default.addObserver(forName: UserDefaults.didChangeNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.apply() }
            // Again once every observer has run: the other shortcut may only now have let go of these keys.
            DispatchQueue.main.async { self?.apply() }
        }
    }

    /// What it is now, nil when off.
    var current: MacShortcut? { applied ?? nil }

    private func apply() {
        let s = shortcut.current
        // Tries again when it failed, as the clash may be gone.
        guard applied != .some(s) || HotKeyFailures.shared.byID[shortcut.rawValue] != nil else { return }
        applied = .some(s)
        hotKey.set(s)
    }
}

/// A shortcut's row in Settings: click the field and press a combination to record it (esc stops), Clear turns it
/// off, Reset puts the default back. A refused combination leaves the shortcut as it was and says why.
struct ShortcutRow: View {
    let shortcut: LoftShortcut
    @AppStorage private var stored: String
    @State private var monitor: Any?

    init(_ shortcut: LoftShortcut) {
        self.shortcut = shortcut
        _stored = AppStorage(wrappedValue: MacShortcut.stored(shortcut.fallback), shortcut.pref)
    }

    private var current: MacShortcut? { MacShortcut.read(stored, default: shortcut.fallback) }
    private var recording: Bool { HotKeyFailures.shared.recording == shortcut.rawValue }

    var body: some View {
        LabeledContent("Shortcut") {
            HStack(spacing: 6) {
                Button { recording ? stop() : start() } label: {
                    Text(recording ? "Type a shortcut…" : current?.label ?? "Off")
                        .frame(minWidth: 120)
                        .foregroundStyle(recording || current == nil ? .secondary : .primary)
                }
                .accessibilityLabel(recording ? "Recording shortcut, press keys" : "Shortcut \(current?.label ?? "off"). Click to record")
                .accessibilityIdentifier("\(shortcut.pref)Recorder")
                Button("Clear") { set(nil) }.disabled(current == nil).accessibilityIdentifier("\(shortcut.pref)Clear")
                Button("Reset") { set(shortcut.fallback) }.disabled(current == shortcut.fallback).accessibilityIdentifier("\(shortcut.pref)Reset")
            }
        }
        .onDisappear { stop() }
        // Another row started recording: this one lets go of the keys.
        .onChange(of: recording) { _, now in if !now { removeMonitor() } }
        if let why = HotKeyFailures.shared.refused[shortcut.rawValue] {
            Text(why).font(.caption).foregroundStyle(.red).accessibilityIdentifier("shortcutRefused")
        }
        ShortcutFailure(id: shortcut.rawValue)
    }

    private func set(_ s: MacShortcut?) {
        stop()
        HotKeyFailures.shared.refused[shortcut.rawValue] = nil
        stored = MacShortcut.stored(s)
    }

    private func start() {
        HotKeyFailures.shared.refused[shortcut.rawValue] = nil
        HotKeyFailures.shared.recording = shortcut.rawValue
        removeMonitor()
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { e in
            guard recording else { return e }
            let flags = e.modifierFlags.intersection(.deviceIndependentFlagsMask)
            if e.keyCode == UInt16(kVK_Escape) && flags.isEmpty { stop() } else { record(MacShortcut(keyCode: UInt32(e.keyCode), modifiers: Self.carbon(flags))) }
            return nil
        }
        GlobalHotKey.recorder = { record($0) }
    }

    private func stop() {
        if recording {
            HotKeyFailures.shared.recording = nil
            GlobalHotKey.recorder = nil
        }
        removeMonitor()
    }

    private func removeMonitor() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
    }

    private func record(_ s: MacShortcut) {
        if s == current { return stop() }
        if let why = shortcut.refusal(s) {
            stop()
            HotKeyFailures.shared.refused[shortcut.rawValue] = why
        } else {
            set(s)
        }
    }

    private static func carbon(_ f: NSEvent.ModifierFlags) -> UInt32 {
        (f.contains(.command) ? MacShortcut.command : 0) | (f.contains(.shift) ? MacShortcut.shift : 0)
            | (f.contains(.option) ? MacShortcut.option : 0) | (f.contains(.control) ? MacShortcut.control : 0)
    }
}

/// "⌥Space couldn't be set up…" under a shortcut, when it couldn't be registered.
struct ShortcutFailure: View {
    let id: UInt32

    var body: some View {
        if let failed = HotKeyFailures.shared.byID[id] {
            Text("\(failed.label) couldn't be set up: Loft's other shortcut or macOS may use it. Record another.")
                .font(.caption).foregroundStyle(.red)
                .accessibilityIdentifier("shortcutFailure")
        }
    }
}
