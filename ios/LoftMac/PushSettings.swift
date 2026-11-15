import LoftKit
import SwiftUI
import UniformTypeIdentifiers

/// Settings › Push notifications, as on the web: where pushes go (the Loft relay with a relay key, the user's own APNs
/// key, or nowhere), Sign in, the relay address, and the phones registered with Send test push and Remove.
struct PushSettings: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.openURL) private var openURL
    @State private var status: JSONValue?
    @State private var phones: [JSONValue] = []
    @State private var keyId = ""
    @State private var teamId = ""
    @State private var file: (name: String, text: String)?
    @State private var relayKey = ""
    @State private var relayAddr = ""
    @State private var picking = false
    @State private var busy = false
    @State private var message: (ok: Bool, text: String)?
    @State private var portalLink: URL?

    private static let chip = ["own_key": "Your own key", "relay": "Using the Loft relay", "none": "Sign in to get push"]
    private static let whereTo = ["own_key": "Pushes go straight to Apple with your own key.",
                                  "relay": "Pushes go through the Loft relay with your relay key.",
                                  "none": "Nothing is pushed yet: sign in to get push (paste your relay key), or add your own APNs key."]

    private func s(_ k: String) -> String { status?[k]?.stringValue ?? "" }
    private func b(_ k: String) -> Bool { status?[k]?.boolValue ?? false }
    private var path: String { s("path").isEmpty ? "none" : s("path") }
    /// The relay's portal, where the user manages keys, when Sign in is offered.
    private var portal: URL? { b("signin") && !s("relay_url").isEmpty ? URL(string: s("relay_url") + "/") : nil }

    var body: some View {
        Section {
            if status != nil { content } else { ProgressView().frame(maxWidth: .infinity) }
        } header: {
            HStack {
                Text("Push notifications")
                Spacer()
                if status != nil {
                    Text(Self.chip[path] ?? path).font(.caption).foregroundStyle(path == "none" ? .secondary : Theme.working)
                        .accessibilityIdentifier("pushPath")
                }
            }
        }
        .task { await load() }
        .task(id: b("signing_in")) {
            // After Sign in the backend collects the key; look until it has it or the code is gone.
            while b("signing_in"), !Task.isCancelled {
                try? await Task.sleep(for: .seconds(2))
                await load()
            }
        }
        .fileImporter(isPresented: $picking, allowedContentTypes: [UTType(filenameExtension: "p8") ?? .data, .data]) { r in
            guard case .success(let url) = r else { return }
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            guard let text = try? String(contentsOf: url, encoding: .utf8) else {
                message = (false, "Couldn't read \(url.lastPathComponent).")
                return
            }
            file = (url.lastPathComponent, text)
            // Apple names it AuthKey_<KEYID>.p8: fill the key id from it.
            if keyId.isEmpty, let m = url.lastPathComponent.firstMatch(of: /^AuthKey_([A-Za-z0-9]{10})\.p8$/) { keyId = String(m.1).uppercased() }
        }
    }

    @ViewBuilder private var content: some View {
        let off = path == "none"
        Text("Alerts reach the iPhone app even when it's closed. \(Self.whereTo[path] ?? "")").font(.caption).foregroundStyle(.secondary)
        if off, phones.count > 0 {
            Label("Push is off: Loft drops every alert for \(phones.count == 1 ? "your phone" : "your \(phones.count) phones"). Sign in below to get push through the Loft relay, or add your own APNs key.",
                  systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(Theme.attentionInk)
        }

        // The Loft relay.
        if b("relay_key") {
            LabeledContent {
                HStack {
                    if let portal { Link("Manage keys", destination: portal) }
                    Button("Remove relay key") { Task { await change(["remove_relay_key": true], "Relay key removed.") } }.disabled(busy)
                }
            } label: {
                Text("Loft relay")
                Text("A relay key is stored on this Mac (owner-only, never shown).\(s("relay_url").isEmpty ? " The relay isn't set up: add its address below." : "")")
            }
        } else {
            VStack(alignment: .leading, spacing: 8) {
                Text("Loft relay").fontWeight(.medium)
                if b("relay_refused") {
                    Text("Your relay key no longer works: Sign in again. Pushes through the relay are paused.").font(.caption).foregroundStyle(Theme.failureInk)
                }
                Text("Sign in to get push: the relay sends your alerts to Apple for you, with no Apple developer account. Agent names and titles pass through it and are never stored. \(portal != nil ? "Press Sign in and allow it on the portal; or paste a relay key." : "Paste the relay key you were given.")")
                    .font(.caption).foregroundStyle(.secondary)
                if portal != nil {
                    HStack {
                        Button("Sign in") { Task { await signIn() } }.buttonStyle(.accent).disabled(busy).accessibilityIdentifier("pushSignIn")
                        if b("signing_in") {
                            if !s("signin_code").isEmpty {
                                Text("Type **\(s("signin_code"))** on the portal, then Allow.").font(.caption)
                            } else {
                                Text("Waiting for you to allow it on the portal.").font(.caption).foregroundStyle(.secondary)
                            }
                            if let l = portalLink { Link("Open the page again", destination: l).font(.caption) }
                        }
                    }
                }
                HStack {
                    SecureField("Relay key", text: $relayKey, prompt: Text("lrk_…")).labelsHidden().font(.body.monospaced())
                    Button("Save relay key") { Task { await change(["relay_key": .string(relayKey)], "Relay key saved.") } }
                        .disabled(relayKey.isEmpty || busy)
                }
            }
        }
        LabeledContent {
            HStack {
                TextField("Relay address", text: $relayAddr,
                          prompt: Text(!s("relay_url").isEmpty && s("relay_address").isEmpty ? "\(s("relay_url")) (default)" : "https://…"))
                    .labelsHidden().font(.body.monospaced())
                Button("Save address") { Task { await change(["relay_url": .string(relayAddr)], "Address saved.") } }
                    .disabled(relayAddr.trimmingCharacters(in: .whitespaces) == s("relay_address") || busy)
            }
        } label: {
            Text("Relay address")
            Text("Your relay key and the alerts' text go to this address, so only use one you trust. https only.")
        }

        // Your own key.
        VStack(alignment: .leading, spacing: 4) {
            Text("Your own key").fontWeight(.medium)
            Text("Sends straight to Apple, without the relay. Apple Developer portal → Keys → +, tick “Apple Push Notifications service”. Apple lets you download the .p8 file once: keep a copy in your password manager.")
                .font(.caption).foregroundStyle(.secondary)
        }
        LabeledContent("Auth key (.p8)") {
            HStack {
                Text(file?.name ?? (b("key_present") ? "A key is stored on this Mac (owner-only, never shown)." : "No key yet."))
                    .font(.caption.monospaced()).foregroundStyle(.secondary)
                Button(b("key_present") ? "Replace key…" : "Choose .p8 file…") { picking = true }
            }
        }
        LabeledContent("Key ID") {
            TextField("Key ID", text: Binding(get: { keyId }, set: { keyId = $0.uppercased() }), prompt: Text("10 characters")).labelsHidden().font(.body.monospaced())
        }
        LabeledContent("Team ID") {
            TextField("Team ID", text: Binding(get: { teamId }, set: { teamId = $0.uppercased() }), prompt: Text("Membership details")).labelsHidden().font(.body.monospaced())
        }
        HStack {
            Button("Save key") { Task { await saveKey() } }
                .disabled(!(keyId != s("key_id") || teamId != s("team_id") || file != nil) || busy)
            if b("key_present") {
                Button("Remove key") { Task { await change(["key_id": "", "team_id": "", "remove_key": true], "Key removed.") } }.disabled(busy)
            }
            if let message {
                Text(message.text).font(.caption).foregroundStyle(message.ok ? Theme.working : Theme.failureInk).accessibilityIdentifier("pushMessage")
            }
        }

        // Phones.
        Text("Phones").fontWeight(.medium)
        if phones.isEmpty {
            Text("None yet. Open the iPhone app and allow notifications; it registers itself.").font(.caption).foregroundStyle(.secondary)
        }
        ForEach(Array(phones.enumerated()), id: \.offset) { _, d in
            let token = d["token"]?.stringValue ?? ""
            let name = (d["name"]?.stringValue).flatMap { $0.isEmpty ? nil : $0 } ?? "iPhone"
            LabeledContent {
                HStack {
                    Button("Send test push") { Task { await test(token, name) } }
                        .disabled(off)
                        .help(off ? "Add a relay key or your own key first" : "")
                        .accessibilityIdentifier("pushTest-\(name)")
                    Button(role: .destructive) { Task { await remove(token) } } label: { Image(systemName: "trash") }
                        .help("Remove this phone")
                        .accessibilityLabel("Remove this phone")
                }
            } label: {
                Label(name, systemImage: "iphone")
                Text(phoneLine(d))
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("pushPhone-\(name)")
        }
    }

    private func phoneLine(_ d: JSONValue) -> String {
        let env = d["environment"]?.stringValue == "production" ? "TestFlight / App Store" : "Xcode build (sandbox)"
        let seen = (d["last_seen_at"]?.stringValue).flatMap { try? Date($0, strategy: .iso8601) }.map { "seen \($0.shortAgo)" } ?? "seen never"
        let kinds: Int = { if case .array(let a)? = d["kinds"] { return a.count }; return 0 }()
        return "\(env) · \(seen) · \(kinds) kinds"
    }

    private func load() async {
        guard let st = try? await store.pushStatus() else { return }
        let first = status == nil
        status = st
        if first || !busy {
            keyId = s("key_id")
            teamId = s("team_id")
            relayAddr = s("relay_address")
        }
        if b("relay_key") { portalLink = nil }
        phones = (try? await store.pushDevices()) ?? []
    }

    /// Sends a change to the push settings and says where pushes go now.
    private func change(_ body: [String: JSONValue], _ done: String) async {
        busy = true
        defer { busy = false }
        message = nil
        do {
            status = try await store.configurePush(body)
            keyId = s("key_id")
            teamId = s("team_id")
            relayKey = ""
            relayAddr = s("relay_address")
            message = (true, "\(done) \(Self.whereTo[path] ?? "")")
        } catch { message = (false, APIError.from(error).title) }
    }

    private func saveKey() async {
        busy = true
        defer { busy = false }
        message = nil
        var body: [String: JSONValue] = ["key_id": .string(keyId), "team_id": .string(teamId)]
        if let file { body["key_p8"] = .string(file.text) }
        do {
            status = try await store.configurePush(body)
            file = nil
            message = (true, path == "own_key" ? "Saved. Pushes go straight to Apple with your key." : "Saved.")
        } catch { message = (false, APIError.from(error).title) }
    }

    private func signIn() async {
        busy = true
        defer { busy = false }
        message = nil
        do {
            let r = try await store.pushSignIn()
            portalLink = PushDestination.portal(r["url"]?.stringValue, relay: s("relay_url"))
            if let u = portalLink { openURL(u) } else { message = (false, "The relay sent a sign-in link that isn't on its own https address.") }
        } catch { message = (false, APIError.from(error).title) }
        // A relay without a portal is remembered: Sign in goes away.
        await load()
    }

    private func test(_ token: String, _ name: String) async {
        message = nil
        do {
            try await store.testPush(token: token)
            message = (true, "Sent to \(name). It should arrive in a few seconds, even with the app closed.")
        } catch {
            message = (false, APIError.from(error).title)
            await load()
        }
    }

    private func remove(_ token: String) async {
        try? await store.removePushDevice(token: token)
        await load()
    }
}
