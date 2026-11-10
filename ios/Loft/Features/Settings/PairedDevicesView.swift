import SwiftUI

/// Settings → Paired devices (#170): every browser and iPhone paired with Loft. Forget this device unpairs one,
/// Unpair all every one but this iPhone. An unpaired client goes back to its pairing screen on its next request.
struct PairedDevicesView: View {
    @Environment(LoftStore.self) private var store
    @State private var clients: [PairedClient] = []
    @State private var error: String?
    @State private var confirmingUnpairAll = false

    private var others: Int { clients.filter { !$0.this }.count }

    var body: some View {
        List {
            Section {
                ForEach(clients) { c in
                    row(c)
                }
            } footer: {
                Text("Browsers and phones that can change Loft. An unpaired one goes back to its pairing screen on its next request.")
            }
            Section {
                Button("Unpair all", role: .destructive) { confirmingUnpairAll = true }
                    .disabled(others == 0)
                    .accessibilityIdentifier("unpairAll")
            } footer: {
                if let error { Text(error).foregroundStyle(.red) }
            }
        }
        .navigationTitle("Paired devices")
        .navigationBarTitleDisplayMode(.inline)
        .confirmationDialog("Unpair \(others) other device\(others == 1 ? "" : "s")?", isPresented: $confirmingUnpairAll, titleVisibility: .visible) {
            Button("Unpair all", role: .destructive) { act { try await store.unpairAll() } }
        } message: {
            Text("Each one goes back to its pairing screen. This iPhone stays paired.")
        }
        .task { await load() }
        .refreshable { await load() }
    }

    private func row(_ c: PairedClient) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    Image(systemName: c.name.contains("iPhone") || c.name.contains("iOS") ? "iphone" : "desktopcomputer")
                        .foregroundStyle(.secondary)
                        .accessibilityHidden(true)
                    Text(c.name)
                    if c.this {
                        Text("This device")
                            .font(.caption)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(.quaternary, in: Capsule())
                    }
                }
                Text("Paired \(c.pairedAt.formatted(.relative(presentation: .named))) · used \(c.lastUsedAt.formatted(.relative(presentation: .named)))")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("pairedClient-\(c.id)")
            Button("Forget this device", role: .destructive) { act { try await store.forgetClient(id: c.id) } }
                .font(.subheadline)
                .buttonStyle(.borderless)
                .accessibilityIdentifier("forget-\(c.id)")
        }
        .padding(.vertical, 2)
    }

    private func act(_ f: @escaping () async throws -> Void) {
        Task {
            do { try await f() } catch { self.error = APIError.from(error).localizedDescription }
            // Forgetting this iPhone makes the reload a 401, which brings back the pairing screen.
            await load()
        }
    }

    private func load() async {
        do {
            clients = try await store.pairedClients()
            error = nil
        } catch {
            self.error = APIError.from(error).localizedDescription
        }
    }
}
