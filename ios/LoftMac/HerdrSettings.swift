import AppKit
import LoftKit
import SwiftUI

/// Settings › Herdr UI, as on the web: set it up on this machine, or copy the command for another one.
struct HerdrSettings: View {
    @Environment(LoftStore.self) private var store
    @State private var status: JSONValue?
    @State private var open = false
    @State private var busy = false
    @State private var result: JSONValue?
    @State private var error: String?
    @State private var copied = false

    private var origin: String { (store.api?.baseURL.absoluteString ?? "").replacingOccurrences(of: #"/+$"#, with: "", options: .regularExpression) }
    private var command: String { "curl -fsSL \(origin)/api/herdr/install.sh | sh -s -- \(origin)" }
    private var isLocal: Bool {
        let h = URL(string: origin)?.host() ?? ""
        return h == "localhost" || h == "::1" || h.hasPrefix("127.")
    }
    private var installed: Bool { status?["herdr"]?["path"]?.stringValue != nil }
    private var linked: Bool { status?["linked"]?["root"]?.stringValue != nil }

    var body: some View {
        Section("Herdr UI") {
            Text("See every agent in Herdr UI, the terminal multiplexer for coding agents: one tab per agent with its chat, slash commands, approvals and this inspector as a sidebar. Herdr shows each one as an agent, working, idle or waiting for you.")
                .font(.caption).foregroundStyle(.secondary)
            if !open {
                Button("Set up Herdr UI") { open = true }.buttonStyle(.accent).accessibilityIdentifier("herdrOpen")
            } else {
                VStack(alignment: .leading, spacing: 6) {
                    Text("On this machine").fontWeight(.medium)
                    Text(here).font(.caption).foregroundStyle(.secondary).accessibilityIdentifier("herdrHere")
                    Text("Builds the Herdr UI client, adds Loft's plugin to Herdr and opens a Loft workspace with your agents. The first build takes a minute.")
                        .font(.caption).foregroundStyle(.secondary)
                    Button(busy ? "Setting up…" : linked ? "Set up again" : "Set up on this machine") { Task { await setUp() } }
                        .disabled(busy || (status != nil && !installed) || !store.canWrite)
                        .accessibilityIdentifier("herdrSetUp")
                    if status != nil && !installed {
                        Text("Install it from [herdr.dev](https://herdr.dev), then come back.").font(.caption)
                    }
                    if case .array(let done)? = result?["done"] {
                        ForEach(Array(done.enumerated()), id: \.offset) { _, d in
                            Label(d.stringValue ?? "", systemImage: "checkmark").foregroundStyle(Theme.working).font(.caption)
                        }
                        Text(result?["message"]?.stringValue ?? "").font(.caption).accessibilityIdentifier("herdrResult")
                    }
                    if let error { Text(error).font(.caption).foregroundStyle(Theme.failureInk) }
                }
                VStack(alignment: .leading, spacing: 6) {
                    Text("On another machine").fontWeight(.medium)
                    Text("Run this in a terminal on the machine where you use Herdr:").font(.caption).foregroundStyle(.secondary)
                    HStack(alignment: .top) {
                        Text(command).font(.caption.monospaced()).textSelection(.enabled)
                        Spacer()
                        Button(copied ? "Copied" : "Copy") {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(command, forType: .string)
                            copied = true
                            Task { try? await Task.sleep(for: .seconds(1.5)); copied = false }
                        }
                    }
                    if isLocal {
                        Text("This Mac app talks to Loft at \(origin), which other machines can't reach. Use an address they can, such as its Tailscale name, to get the right command.")
                            .font(.caption).foregroundStyle(Theme.attentionInk)
                    } else {
                        Text("That machine talks to this Loft at \(origin). It downloads a ready-made build when it has the same system as this one\(status?["platform"]?.stringValue.map { " (\($0))" } ?? ""); otherwise it builds one with cargo.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
        }
        .task { await load() }
    }

    private var here: String {
        guard let status else { return " " }
        guard installed else { return "Herdr isn't installed on this machine." }
        let v = (status["herdr"]?["version"]?.stringValue ?? "").replacingOccurrences(of: #"^herdr\s*"#, with: "", options: .regularExpression)
        return "Herdr \(v) is installed here\(linked ? " and shows Loft's agents" : "")."
    }

    private func load() async {
        status = try? await store.herdrStatus()
        if linked { open = true }
    }

    private func setUp() async {
        busy = true
        defer { busy = false }
        error = nil
        result = nil
        do {
            result = try await store.herdrSetup()
            await load()
        } catch { self.error = APIError.from(error).title }
    }
}
