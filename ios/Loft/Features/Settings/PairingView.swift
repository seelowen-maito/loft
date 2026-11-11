import SwiftUI

/// "Pair this iPhone" (spec #164): shows the code to compare on a paired device or at the Mac, polls with the
/// secret, and keeps the token in the Keychain. In the Simulator and the demo it only says why it can't.
struct PairingView: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dismiss) private var dismiss

    private enum Phase: Equatable {
        case starting
        case waiting(code: String, secret: String)
        case denied
        case expired
        case failed(String)
    }

    @State private var phase = Phase.starting
    /// Bumped by Try again, which restarts the pairing task.
    @State private var attempt = 0

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 20) {
                    if let why = store.pairingRefusal {
                        refused(why)
                    } else {
                        pairing
                    }
                }
                .padding(24)
                .frame(maxWidth: .infinity)
            }
            .navigationTitle("Pair this iPhone")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Not now") { close() }.accessibilityIdentifier("pairingNotNow")
                }
            }
        }
        .task(id: attempt) {
            guard store.pairingRefusal == nil else { return }
            await run()
        }
    }

    private func refused(_ why: String) -> some View {
        VStack(spacing: 16) {
            Image(systemName: "lock.iphone").font(.system(size: 44)).foregroundStyle(.secondary)
                .accessibilityHidden(true)
            Text("Pairing is off here").font(.title2.weight(.bold))
            Text(why).multilineTextAlignment(.center).foregroundStyle(.secondary)
                .accessibilityIdentifier("pairingRefusal")
            if !store.isDemo {
                Button("Open the demo") {
                    store.putOffPairing()
                    store.enterDemo()
                }
                .buttonStyle(.accent)
                .accessibilityIdentifier("pairingOpenDemo")
            }
        }
    }

    @ViewBuilder private var pairing: some View {
        Image(systemName: "iphone.badge.checkmark").font(.system(size: 44)).foregroundStyle(Theme.accent)
            .accessibilityHidden(true)
        switch phase {
        case .starting:
            ProgressView("Asking Loft…")
        case .waiting(let code, _):
            Text("Allow this iPhone on a paired device or at the Mac, after checking it shows this code:")
                .multilineTextAlignment(.center)
            Text(code)
                .font(.system(.largeTitle, design: .monospaced).weight(.bold))
                .kerning(8)
                .accessibilityLabel("Code \(code.map(String.init).joined(separator: " "))")
                .accessibilityIdentifier("pairingCode")
            ProgressView().accessibilityLabel("Waiting for approval")
            Text("Only allow it if the codes match.").font(.footnote).foregroundStyle(.secondary)
        case .denied:
            outcome("Pairing was denied", "Nothing was paired. Try again if that wasn't you.")
        case .expired:
            outcome("Nobody allowed it in time", "The code expired after two minutes.")
        case .failed(let why):
            outcome("Couldn't pair", why)
        }
    }

    private func outcome(_ title: String, _ detail: String) -> some View {
        VStack(spacing: 12) {
            Text(title).font(.headline).accessibilityIdentifier("pairingOutcome")
            Text(detail).multilineTextAlignment(.center).foregroundStyle(.secondary)
            Button("Try again") { attempt += 1 }
                .buttonStyle(.accent)
                .accessibilityIdentifier("pairingTryAgain")
        }
    }

    /// Asks to pair, then polls every two seconds until Loft answers. Leaving the screen gives the pairing up.
    private func run() async {
        guard let api = store.api else { return }
        phase = .starting
        let start: PairingStart
        do {
            start = try await api.startPairing(name: UIDevice.current.name)
        } catch {
            phase = .failed(APIError.from(error).title)
            return
        }
        phase = .waiting(code: start.code, secret: start.secret)
        while !Task.isCancelled {
            try? await Task.sleep(for: .seconds(2))
            if Task.isCancelled { break }
            guard let poll = try? await api.pollPairing(secret: start.secret) else { continue }
            switch poll.status {
            case "allowed":
                if let token = poll.token, store.finishPairing(token: token) {
                    dismiss()
                } else {
                    phase = .failed("This iPhone couldn't keep its pairing in the Keychain.")
                }
                return
            case "denied": phase = .denied; return
            case "gone": phase = .expired; return
            default: continue
            }
        }
        // Cancelled while waiting: Loft drops the pending pairing.
        Task { try? await api.cancelPairing(secret: start.secret) }
    }

    private func close() {
        store.putOffPairing()
        dismiss()
    }
}
