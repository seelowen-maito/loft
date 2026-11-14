import Foundation

/// A global shortcut the Mac app records in Settings (spec #456): a key and its modifiers, in Carbon's terms so it can
/// be registered as is. Kept pure so its stored form, labels and rules can be tested without a keyboard.
///
/// Stored as the modifiers' names then the key code, joined by "-" ("control-command-49"); "off" is none. The old
/// fixed list's values ("option-space", "shift-option-space", …) read as the same shortcut.
public struct MacShortcut: Equatable, Sendable {
    /// Carbon's virtual key code (`kVK_…`).
    public var keyCode: UInt32
    /// Carbon's modifier bits (`cmdKey`, `shiftKey`, `optionKey`, `controlKey`).
    public var modifiers: UInt32

    public init(keyCode: UInt32, modifiers: UInt32) {
        self.keyCode = keyCode
        self.modifiers = modifiers
    }

    public static let command: UInt32 = 1 << 8, shift: UInt32 = 1 << 9, option: UInt32 = 1 << 11, control: UInt32 = 1 << 12
    public static let space: UInt32 = 49
    public static let off = "off"

    /// The defaults: quick search, the voice panel and Open Loft (off).
    public static let quickSearchDefault = MacShortcut(keyCode: space, modifiers: option)
    public static let voicePanelDefault = MacShortcut(keyCode: space, modifiers: shift | option)

    /// In the Mac's order: ⌃⌥⇧⌘.
    private static let names: [(UInt32, String, String)] = [(control, "control", "⌃"), (option, "option", "⌥"),
                                                            (shift, "shift", "⇧"), (command, "command", "⌘")]

    /// What's stored for `shortcut`: nil is off.
    public static func stored(_ shortcut: MacShortcut?) -> String { shortcut?.stored ?? off }

    /// The shortcut stored, nil when off. A missing or unreadable value is `fallback`.
    public static func read(_ stored: String?, default fallback: MacShortcut?) -> MacShortcut? {
        guard let stored else { return fallback }
        if stored == off { return nil }
        return MacShortcut(stored: stored) ?? fallback
    }

    public init?(stored: String) {
        var parts = stored.split(separator: "-").map(String.init)
        guard let key = parts.popLast(), let code = key == "space" ? Self.space : UInt32(key) else { return nil }
        var modifiers: UInt32 = 0
        for p in parts {
            guard let m = Self.names.first(where: { $0.1 == p }) else { return nil }
            modifiers |= m.0
        }
        self.init(keyCode: code, modifiers: modifiers)
    }

    public var stored: String { (Self.names.filter { modifiers & $0.0 != 0 }.map(\.1) + [String(keyCode)]).joined(separator: "-") }

    /// "⌃⌘Space".
    public var label: String { Self.names.filter { modifiers & $0.0 != 0 }.map(\.2).joined() + Self.keyName(keyCode) }

    public var isFunctionKey: Bool { Self.functionKeys[keyCode] != nil }

    /// Why it can't be a global shortcut, or nil. `taken` are Loft's other shortcuts with what each does, and `system`
    /// the ones macOS keeps for itself.
    public func refusal(taken: [(MacShortcut, String)], system: [MacShortcut] = []) -> String? {
        if modifiers & (Self.command | Self.option | Self.control | Self.shift) == 0 && !isFunctionKey {
            return "\(label) needs ⌘, ⌥, ⌃ or ⇧, unless it's a function key."
        }
        if let t = taken.first(where: { $0.0 == self }) { return "\(label) is already Loft's \(t.1) shortcut." }
        if system.contains(self) { return "macOS already uses \(label). Pick another." }
        return nil
    }

    private static let functionKeys: [UInt32: String] = [
        122: "F1", 120: "F2", 99: "F3", 118: "F4", 96: "F5", 97: "F6", 98: "F7", 100: "F8", 101: "F9", 109: "F10",
        103: "F11", 111: "F12", 105: "F13", 107: "F14", 113: "F15", 106: "F16", 64: "F17", 79: "F18", 80: "F19", 90: "F20",
    ]

    /// Key names on a US keyboard, as the Mac's menus write them.
    private static let keys: [UInt32: String] = [
        0: "A", 1: "S", 2: "D", 3: "F", 4: "H", 5: "G", 6: "Z", 7: "X", 8: "C", 9: "V", 11: "B", 12: "Q", 13: "W", 14: "E",
        15: "R", 16: "Y", 17: "T", 18: "1", 19: "2", 20: "3", 21: "4", 22: "6", 23: "5", 24: "=", 25: "9", 26: "7", 27: "-",
        28: "8", 29: "0", 30: "]", 31: "O", 32: "U", 33: "[", 34: "I", 35: "P", 36: "↩", 37: "L", 38: "J", 39: "'", 40: "K",
        41: ";", 42: "\\", 43: ",", 44: "/", 45: "N", 46: "M", 47: ".", 48: "⇥", 49: "Space", 50: "`", 51: "⌫", 53: "⎋",
        65: "Keypad .", 67: "Keypad *", 69: "Keypad +", 71: "⌧", 75: "Keypad /", 76: "⌤", 78: "Keypad -", 81: "Keypad =",
        82: "Keypad 0", 83: "Keypad 1", 84: "Keypad 2", 85: "Keypad 3", 86: "Keypad 4", 87: "Keypad 5", 88: "Keypad 6",
        89: "Keypad 7", 91: "Keypad 8", 92: "Keypad 9", 114: "Help", 115: "↖", 116: "⇞", 117: "⌦", 119: "↘", 121: "⇟",
        123: "←", 124: "→", 125: "↓", 126: "↑",
    ]

    static func keyName(_ code: UInt32) -> String { functionKeys[code] ?? keys[code] ?? "Key \(code)" }
}
