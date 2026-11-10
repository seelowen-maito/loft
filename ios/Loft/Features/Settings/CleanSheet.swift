import SwiftUI

/// Settings › Disk's clean: what Clean (one agent, or all with nil) would remove, by agent, with folders to untick;
/// then each removal as it happens and the summary. Opened on a log, it shows that one.
struct CleanSheet: View {
    enum Start: Identifiable {
        case preview(agentId: String?)
        case log(CleanLog)
        var id: String {
            switch self {
            case .preview(let a): "preview-\(a ?? "all")"
            case .log: "log"
            }
        }
    }

    let start: Start
    /// Agent names by id; a deleted agent's workspace shows its id.
    let names: [String: String]
    /// After a clean, with the agents it cleaned, so their sizes refresh.
    var onCleaned: (Set<String>) async -> Void
    @Environment(LoftStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var plan: CleanPlan?
    @State private var unticked: Set<String> = []
    @State private var log: CleanLog?
    @State private var error: APIError?
    @State private var copied = false

    private var agentId: String? {
        if case .preview(let a) = start { return a }
        return nil
    }
    private var kept: [CleanPlan.Folder] { plan?.folders.filter { !unticked.contains($0.path) } ?? [] }

    var body: some View {
        NavigationStack {
            List {
                if let log {
                    logSections(log)
                } else if let plan {
                    previewSections(plan)
                } else {
                    ProgressView("Measuring workspaces…").frame(maxWidth: .infinity)
                }
            }
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { toolbar }
        }
        .interactiveDismissDisabled(log?.running == true)
        .errorAlert($error)
        .task {
            switch start {
            case .log(let l): log = l
            case .preview(let a):
                do { plan = try await store.cleanPreview(agentId: a) } catch { self.error = APIError.from(error) }
            }
        }
    }

    private var title: String {
        if let log { return log.running ? "Cleaning…" : log.what.isEmpty ? "Clean" : log.what }
        return agentId.map { "Clean \(name($0))?" } ?? "Clean all?"
    }

    private func name(_ id: String) -> String { names[id] ?? "Deleted agent \(id.prefix(8))" }
    private func bytes(_ n: Int64) -> String { n.formatted(.byteCount(style: .file)) }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        if let log {
            ToolbarItem(placement: .confirmationAction) {
                Button("Done") { dismiss() }.disabled(log.running)
            }
        } else {
            ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            ToolbarItem(placement: .confirmationAction) {
                Button(plan == nil ? "Remove" : "Remove \(bytes(kept.reduce(0) { $0 + $1.bytes }))", role: .destructive) { confirm() }
                    .disabled(kept.isEmpty)
                    .accessibilityIdentifier("cleanConfirm")
            }
        }
    }

    @ViewBuilder
    private func previewSections(_ plan: CleanPlan) -> some View {
        Section {
            let agents = Set(kept.map(\.agent)).count
            Text(plan.folders.isEmpty ? "Nothing to remove."
                : "Removes \(kept.count) folder\(kept.count == 1 ? "" : "s") from \(agents) agent\(agents == 1 ? "" : "s"); their next build starts from scratch.")
                .foregroundStyle(.secondary)
        }
        ForEach(groupedByAgent(plan.folders, \.agent), id: \.agent) { group in
            Section {
                ForEach(group.items) { f in
                    Toggle(isOn: Binding(get: { !unticked.contains(f.path) }, set: { on in tick([f.path], on) })) {
                        Text(f.rel).font(.footnote.monospaced()).lineLimit(1).truncationMode(.middle)
                        Text("\(bytes(f.bytes)) · changed \(f.changed.formatted(.relative(presentation: .named)))")
                        Text(f.command).font(.caption2.monospaced()).textSelection(.enabled)
                    }
                }
            } header: {
                let paths = group.items.map(\.path)
                Toggle(isOn: Binding(get: { paths.allSatisfy { !unticked.contains($0) } }, set: { on in tick(paths, on) })) {
                    Text("\(name(group.agent)) · \(bytes(group.items.reduce(0) { $0 + $1.bytes }))")
                }
            }
        }
        commandsSection(kept.map { ($0.agent, $0.command) })
        skippedSection(plan.skipped)
    }

    /// The equivalent commands, labelled as such, with "Copy commands".
    @ViewBuilder
    private func commandsSection(_ items: [(agent: String, command: String)]) -> some View {
        if !items.isEmpty {
            Section {
                Button(copied ? "Copied" : "Copy commands") {
                    UIPasteboard.general.string = commandsText(items, name: name)
                    copied = true
                }
                .accessibilityIdentifier("copyCommands")
            } footer: {
                Text("Commands are the equivalent of each removal; your Mac removes the paths itself.")
            }
        }
    }

    private func tick(_ paths: [String], _ on: Bool) {
        for p in paths {
            if on { unticked.remove(p) } else { unticked.insert(p) }
        }
    }

    @ViewBuilder
    private func logSections(_ log: CleanLog) -> some View {
        Section {
            Text("Freed \(bytes(log.freedBytes)) · \(log.removed) removed · \(log.skipped.count) skipped\(log.failed > 0 ? " · \(log.failed) failed" : "")")
                .accessibilityIdentifier("cleanSummary")
            if let at = log.at, !log.running {
                Text(at.formatted(date: .abbreviated, time: .shortened)).foregroundStyle(.secondary)
            }
        }
        ForEach(groupedByAgent(log.lines, \.agent), id: \.agent) { group in
            Section(name(group.agent)) {
                ForEach(group.items) { l in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(l.error.map { "failed \(l.path): \($0)" } ?? "remove \(l.path) (\(bytes(l.bytes)))")
                            .font(.caption.monospaced())
                            .foregroundStyle(l.error == nil ? Color.primary : Theme.failure)
                        if !l.command.isEmpty {
                            Text(l.command).font(.caption2.monospaced()).foregroundStyle(.secondary)
                        }
                    }
                    .textSelection(.enabled)
                }
            }
        }
        if !log.running && log.lines.isEmpty {
            Text("Nothing was removed.").foregroundStyle(.secondary)
        }
        if !log.running { commandsSection(log.lines.filter { !$0.command.isEmpty }.map { ($0.agent, $0.command) }) }
        skippedSection(log.skipped)
    }

    @ViewBuilder
    private func skippedSection(_ skipped: [CleanSkip]) -> some View {
        if !skipped.isEmpty {
            Section("Skipped") {
                ForEach(Array(skipped.enumerated()), id: \.offset) { _, k in
                    LabeledContent {
                        Text(k.reason)
                    } label: {
                        Text(name(k.agent))
                        if !k.rel.isEmpty { Text(k.rel).font(.caption.monospaced()) }
                    }
                }
            }
        }
    }

    private func confirm() {
        let paths = kept.map(\.path)
        copied = false
        log = CleanLog(at: nil, what: "", running: true, lines: [], skipped: [], freedBytes: 0, removed: 0, failed: 0)
        Task {
            // The log fills in as the Mac removes each folder.
            let poll = Task {
                while !Task.isCancelled {
                    try? await Task.sleep(for: .milliseconds(500))
                    if let l = try? await store.cleanLog(), l.running, !Task.isCancelled { log = l }
                }
            }
            defer { poll.cancel() }
            do {
                let done = try await store.clean(agentId: agentId, paths: paths)
                log = done
                await onCleaned(Set(done.lines.map(\.agent)))
            } catch {
                log = nil
                self.error = APIError.from(error)
            }
        }
    }
}
