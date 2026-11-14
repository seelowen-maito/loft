import AppIntents
import Security
import SwiftUI
import WidgetKit

/// Proves a `loft://talk` came from Loft's own widget or control: any app or web page can open a bare one, and only
/// the signed one starts the mic (Security on #467). A random key in the Keychain group the apps and the widgets
/// share (see `SharedServer`), made by whichever side needs it first. Without the group (an unsigned build) there is
/// no key and Talk links only open the app.
public enum TalkKey {
    private static let service = "loft.talk-key"

    /// The link the widget and the control open.
    public static func url(key: String?) -> URL {
        guard let key else { return DeepLink.talk.url }
        return URL(string: "loft://talk?key=\(key)")!
    }

    /// Whether `url` is a Talk link signed with `key`.
    public static func signs(_ url: URL, key: String?) -> Bool {
        guard DeepLink(url: url) == .talk, let key, !key.isEmpty else { return false }
        return URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?.first { $0.name == "key" }?.value == key
    }

    /// The key, made and saved on first use; nil when the Keychain group isn't there.
    public static var current: String? {
        if let k = read() { return k }
        var bytes = [UInt8](repeating: 0, count: 32)
        guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else { return nil }
        let key = bytes.map { String(format: "%02x", $0) }.joined()
        var q = query
        q[kSecValueData] = Data(key.utf8)
        q[kSecAttrAccessible] = kSecAttrAccessibleAfterFirstUnlock
        // The other process may have saved one meanwhile: then that one is the key.
        return SecItemAdd(q as CFDictionary, nil) == errSecSuccess ? key : read()
    }

    private static func read() -> String? {
        var q = query
        q[kSecReturnData] = true
        q[kSecMatchLimit] = kSecMatchLimitOne
        var out: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess, let data = out as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    private static var query: [CFString: Any] {
        var q: [CFString: Any] = [kSecClass: kSecClassGenericPassword, kSecAttrService: service]
        #if os(macOS)
        q[kSecUseDataProtectionKeychain] = true
        #endif
        return q
    }
}

/// Opens the signed `loft://talk`: the Talk control's tap. The app starts listening (iPhone) or opens Talk (Mac).
@available(iOS 18.0, macOS 26.0, *)
public struct OpenTalkIntent: AppIntent {
    public static let title: LocalizedStringResource = "Talk to Loft"
    public static let description = IntentDescription("Start talking to Loft.")
    public static let isDiscoverable = false

    public init() {}

    public func perform() async throws -> some IntentResult & OpensIntent { .result(opensIntent: OpenURLIntent(TalkKey.url(key: TalkKey.current))) }
}

/// The Talk control (spec #462): Control Center and the Lock Screen on the iPhone, Control Center on the Mac.
@available(iOS 18.0, macOS 26.0, *)
public struct TalkControl: ControlWidget {
    public init() {}

    public var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: "loft.talk.control") {
            ControlWidgetButton(action: OpenTalkIntent()) { Label("Talk to Loft", systemImage: "mic.fill") }
        }
        .displayName("Talk to Loft")
        .description("Start talking to Loft.")
    }
}

struct TalkEntry: TimelineEntry {
    let date: Date
    /// The signed link the tap opens.
    let link: URL
}

/// Only the key to read: the widget is a button.
struct TalkProvider: TimelineProvider {
    func placeholder(in context: Context) -> TalkEntry { TalkEntry(date: .now, link: DeepLink.talk.url) }
    func getSnapshot(in context: Context, completion: @escaping (TalkEntry) -> Void) { completion(entry) }
    func getTimeline(in context: Context, completion: @escaping (Timeline<TalkEntry>) -> Void) {
        completion(Timeline(entries: [entry], policy: .never))
    }

    private var entry: TalkEntry { TalkEntry(date: .now, link: TalkKey.url(key: TalkKey.current)) }
}

/// The one-tap Talk widget (spec #462): small on the Home Screen and the desktop, round on the Lock Screen.
public struct TalkWidget: Widget {
    public init() {}

    public var body: some WidgetConfiguration {
        StaticConfiguration(kind: "loft.talk", provider: TalkProvider()) { entry in
            TalkEntryView()
                .containerBackground(.background, for: .widget)
                .widgetURL(entry.link)
        }
        .configurationDisplayName("Talk")
        .description("One tap to start talking to Loft.")
        .supportedFamilies(Self.families)
    }

    static var families: [WidgetFamily] {
        #if os(iOS)
        [.systemSmall, .accessoryCircular]
        #else
        [.systemSmall]
        #endif
    }
}

private struct TalkEntryView: View {
    @Environment(\.widgetFamily) private var family

    var body: some View { TalkWidgetView(round: family != .systemSmall) }
}

/// A mic in the accent circle and "Talk to Loft"; on the Lock Screen (`round`), just the mic.
struct TalkWidgetView: View {
    let round: Bool

    var body: some View {
        if !round {
            VStack(spacing: 10) {
                mic.frame(width: 64, height: 64).background(Theme.accent, in: Circle())
                Text("Talk to Loft").font(.headline)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ZStack {
                AccessoryWidgetBackground()
                mic
            }
            .accessibilityLabel("Talk to Loft")
        }
    }

    private var mic: some View {
        Image(systemName: "mic.fill")
            .font(.title.weight(.semibold))
            .foregroundStyle(round ? AnyShapeStyle(.primary) : AnyShapeStyle(Theme.onAccent))
    }
}
