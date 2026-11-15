import LoftKit
import SwiftUI

/// Loft's address: this Mac by default, or another Mac's Tailscale address. Saved only once Loft answers there.
struct AddressForm: View {
    @Environment(LoftStore.self) private var store
    let isOnboarding: Bool

    static let local = "http://localhost:7420"

    @State private var address = UserDefaults.standard.string(forKey: Prefs.serverURL) ?? AddressForm.local
    @State private var checking = false
    @State private var failure: APIError?

    private var url: URL? { LiveLoftAPI.normalize(address) }
    private var saved: String? { UserDefaults.standard.string(forKey: Prefs.serverURL) }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if isOnboarding {
                Image(systemName: "house.lodge.fill").font(.system(size: 40)).foregroundStyle(.tint).accessibilityHidden(true)
                Text("Connect to Loft").font(.largeTitle.bold())
                Text("Loft runs on this Mac by default. To use a Loft running on another Mac, enter its Tailscale address.")
                    .foregroundStyle(.secondary)
            } else {
                Text("Loft address").font(.headline)
            }
            HStack {
                TextField("Loft address", text: $address)
                    .textFieldStyle(.roundedBorder)
                    .accessibilityIdentifier("serverAddressField")
                    .onSubmit { Task { await connect() } }
                    .onChange(of: address) { failure = nil }
                Button("This Mac") { address = Self.local }
                    .help("Loft on this Mac: \(Self.local)")
            }
            Text("This Mac: \(Self.local). Another Mac: its https address from `tailscale serve`, such as https://your-mac.your-tailnet.ts.net.")
                .font(.footnote)
                .foregroundStyle(.secondary)
            if let failure {
                VStack(alignment: .leading, spacing: 4) {
                    Label("Can't reach Loft at \(url?.absoluteString ?? address)", systemImage: "xmark.octagon.fill")
                        .foregroundStyle(Theme.failureInk)
                        .accessibilityIdentifier("connectError")
                    Text("\(failure.title). Check that Loft is running there and the address is right.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Theme.failure.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
            }
            HStack {
                Button {
                    Task { await connect() }
                } label: {
                    Text(isOnboarding ? "Connect" : "Save and connect")
                }
                .keyboardShortcut(.defaultAction)
                .disabled(url == nil || checking || (!isOnboarding && url?.absoluteString == saved))
                .accessibilityIdentifier("saveServer")
                if checking { ProgressView().controlSize(.small) }
            }
            if !isOnboarding, store.api != nil {
                Divider()
                LabeledContent("Connection", value: store.connection.label)
                Button("Forget this Loft", role: .destructive) { store.forgetServer() }
                    .help("Removes the address. You enter it again next time.")
            }
        }
    }

    /// Asks Loft for its info first: a wrong address or a Loft that isn't running is said here, nothing is saved.
    private func connect() async {
        guard let url else { return }
        checking = true
        defer { checking = false }
        do {
            _ = try await LiveLoftAPI(baseURL: url).info()
            store.connect(address: url.absoluteString)
        } catch {
            failure = APIError.from(error)
        }
    }
}
