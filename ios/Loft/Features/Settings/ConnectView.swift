import SwiftUI

/// Enter, test and save the Loft server address.
struct ConnectView: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let isOnboarding: Bool
    @State private var address = UserDefaults.standard.string(forKey: Prefs.serverURL) ?? ""
    @State private var testing = false
    @State private var result: Result<ServerInfo, APIError>?

    private var url: URL? { LiveLoftAPI.normalize(address) }

    var body: some View {
        Form {
            if isOnboarding {
                Section {
                    VStack(alignment: .leading, spacing: 10) {
                        Image(systemName: "house.lodge.fill")
                            .font(.system(size: 40))
                            .foregroundStyle(Theme.accent)
                        Text("Connect to Loft").font(.title2.weight(.bold))
                        Text("Loft runs on your Mac. This app talks to it privately over your Tailscale network.")
                            .foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 6)
                }
                .listRowBackground(Color.clear)
            }
            Section {
                TextField("https://your-mac.your-tailnet.ts.net", text: $address)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
                    .textContentType(.URL)
                    .accessibilityIdentifier("serverAddressField")
                    .onChange(of: address) { result = nil }
            } header: {
                Text("Server address")
            } footer: {
                Text("On the Mac, run `tailscale serve --bg 7420` and use the https address it prints. This iPhone needs the Tailscale app, signed in to the same tailnet.")
            }
            if let url, url.scheme == "http", !Self.isLocal(url) {
                Section {
                    Label("Plain http isn't allowed except for a development server on this device's network. Use the https address from `tailscale serve`.", systemImage: "lock.slash")
                        .font(.footnote)
                        .foregroundStyle(Theme.attention)
                }
            }
            Section {
                Button {
                    Task { await test() }
                } label: {
                    HStack {
                        Text("Test connection")
                        Spacer()
                        if testing { ProgressView() }
                    }
                }
                .disabled(url == nil || testing)
                .accessibilityIdentifier("testConnection")
                switch result {
                case .success(let info):
                    Label("Reached Loft \(info.version ?? "")\(info.host.map { " on \($0)" } ?? "")", systemImage: "checkmark.circle.fill")
                        .foregroundStyle(Theme.working)
                    if !info.supports(.tasks) {
                        Label("This server is older and doesn't support tasks or the activity feed. Update Loft on the Mac for the full app.", systemImage: "info.circle")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                case .failure(let e):
                    VStack(alignment: .leading, spacing: 4) {
                        Label(e.title, systemImage: "xmark.octagon.fill").foregroundStyle(Theme.failure)
                        Text(e.suggestion).font(.footnote).foregroundStyle(.secondary)
                    }
                case nil:
                    EmptyView()
                }
            }
            Section {
                Button("Save and connect") { save() }
                    .bold()
                    .disabled(url == nil)
                    .accessibilityIdentifier("saveServer")
            } footer: {
                Text("Loft has no login of its own: only devices on your tailnet can reach it. Keep it off the public internet.")
            }
            if isOnboarding {
                Section {
                    Button { store.enterDemo() } label: { Label("Try the demo", systemImage: "play.circle") }
                        .accessibilityIdentifier("tryDemo")
                } footer: {
                    Text("Sample data on this iPhone, with no server: nothing is sent anywhere, and it resets when you leave.")
                }
            }
        }
        .navigationTitle(isOnboarding ? "" : "Server")
        .navigationBarTitleDisplayMode(.inline)
    }

    static func isLocal(_ url: URL) -> Bool {
        guard let h = url.host()?.lowercased() else { return false }
        return h == "localhost" || h == "127.0.0.1" || h.hasSuffix(".local")
    }

    private func test() async {
        guard let url else { return }
        testing = true
        defer { testing = false }
        do {
            result = .success(try await LiveLoftAPI(baseURL: url).info())
        } catch {
            result = .failure(APIError.from(error))
        }
    }

    private func save() {
        guard let url else { return }
        UserDefaults.standard.set(url.absoluteString, forKey: Prefs.serverURL)
        UserDefaults.standard.removeObject(forKey: "serverURL-override")
        store.configure(LiveLoftAPI(baseURL: url))
        store.activate()
        if !isOnboarding { dismiss() }
    }
}
