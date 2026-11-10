import SwiftUI

/// Settings › Disk's clean: each workspace's top-level items (one agent, or all with nil), by agent, to tick what goes;
/// a main clone opens to its folders, one level at a time. Then each removal as it happens and the summary. Opened on
/// a log, it shows that one.
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
    @State private var items: [CleanItem]?
    @State private var ticked: Set<String> = []
    /// Folders listed inside opened items, by path; nil while loading.
    @State private var inside: [String: [CleanItem]?] = [:]
    @State private var log: CleanLog?
    @State private var error: APIError?
    @State private var copied = false

    private var agentId: String? {
        if case .preview(let a) = start { return a }
        return nil
    }
    /// Every item listed so far, opened folders' included, and those ticked.
    private var known: [CleanItem] { (items ?? []) + inside.values.flatMap { $0 ?? [] } }
    private var chosen: [CleanItem] { known.filter { ticked.contains($0.path) } }

    var body: some View {
        NavigationStack {
            List {
                if let log {
                    logSections(log)
                } else if let items {
                    previewSections(items)
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
                do {
                    let listed = try await store.cleanPreview(agentId: a)
                    ticked = Set(listed.filter(\.ticked).map(\.path))
                    items = listed
                } catch { self.error = APIError.from(error) }
            }
        }
    }

    private var title: String {
        if let log { return log.title ?? (log.running ? "Cleaning…" : log.what.isEmpty ? "Clean" : log.what) }
        return agentId.map { "Clean \(name($0))?" } ?? "Clean all?"
    }

    private func name(_ id: String) -> String { names[id] ?? "Deleted agent \(id.prefix(8))" }
    private func bytes(_ n: Int64) -> String { bytesText(n) }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        if let log {
            ToolbarItem(placement: .confirmationAction) {
                Button("Done") { dismiss() }.disabled(log.running)
            }
        } else {
            ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            ToolbarItem(placement: .confirmationAction) {
                Button(items == nil ? "Remove" : "Remove \(bytes(chosen.reduce(0) { $0 + $1.bytes }))", role: .destructive) { confirm() }
                    .disabled(chosen.isEmpty)
                    .accessibilityIdentifier("cleanConfirm")
            }
        }
    }

    @ViewBuilder
    private func previewSections(_ items: [CleanItem]) -> some View {
        Section {
            let total = chosen.reduce(0) { $0 + $1.bytes }
            let kept = max(0, items.reduce(0) { $0 + $1.bytes } - total)
            Text(items.isEmpty ? "Nothing to remove." : "Removes \(chosen.count) item\(chosen.count == 1 ? "" : "s") (\(bytes(total))) · keeps \(bytes(kept)).")
                .foregroundStyle(.secondary)
                .accessibilityIdentifier("cleanSummary")
        }
        ForEach(groupedByAgent(items, \.agent), id: \.agent) { group in
            Section("\(name(group.agent)) · \(bytes(group.items.reduce(0) { $0 + $1.bytes }))") {
                rows(group.items, depth: 0)
            }
        }
        commandsSection(chosen.map { ($0.agent, $0.command) })
    }

    /// The items at `depth`, each followed by its folders when opened.
    private func rows(_ items: [CleanItem], depth: Int) -> AnyView {
        AnyView(ForEach(items) { i in
            row(i, depth: depth)
            if let sub = inside[i.path] {
                if let sub {
                    if sub.isEmpty {
                        Text("No folders inside.").font(.footnote).foregroundStyle(.secondary).padding(.leading, CGFloat(depth + 1) * 16 + 28)
                    } else {
                        rows(sub, depth: depth + 1)
                    }
                } else {
                    ProgressView().frame(maxWidth: .infinity)
                }
            }
        })
    }

    private func row(_ i: CleanItem, depth: Int) -> some View {
        let on = ticked.contains(i.path)
        return HStack(alignment: .top, spacing: 8) {
            if i.opens {
                Button { open(i) } label: {
                    Image(systemName: "chevron.right").rotationEffect(.degrees(inside[i.path] != nil ? 90 : 0)).frame(width: 18, height: 22)
                }
                .buttonStyle(.borderless)
                .accessibilityLabel(inside[i.path] != nil ? "Close \(i.name)" : "Open \(i.name)")
            } else {
                Color.clear.frame(width: 18, height: 1)
            }
            Button { tick(i.path, !on) } label: {
                Image(systemName: on ? "checkmark.square.fill" : "square").font(.title3)
            }
            .buttonStyle(.borderless)
            .disabled(!i.canTick)
            .accessibilityLabel("Remove \(i.rel)")
            .accessibilityValue(on ? "ticked" : "not ticked")
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline) {
                    Text(i.name).font(.subheadline.monospaced()).lineLimit(1).truncationMode(.middle)
                    Spacer(minLength: 4)
                    Text(bytes(i.bytes)).font(.subheadline).monospacedDigit()
                }
                Text("\(i.kind == "clone" ? "git clone" : i.kind) · changed \(i.changed.formatted(.relative(presentation: .named)))")
                    .font(.caption).foregroundStyle(.secondary)
                if !i.label.isEmpty { Text(i.label).font(.caption).foregroundStyle(Theme.accent) }
                if !i.blocked.isEmpty { Text(i.blocked).font(.caption).foregroundStyle(Theme.attentionInk) }
                if on && !i.warning.isEmpty { Text(i.warning).font(.caption).foregroundStyle(Theme.attentionInk) }
                if !i.note.isEmpty { Text(i.note).font(.caption).foregroundStyle(.secondary) }
                if !i.command.isEmpty { Text(i.command).font(.caption2.monospaced()).foregroundStyle(.secondary).textSelection(.enabled) }
            }
        }
        .padding(.leading, CGFloat(depth) * 16)
        .accessibilityIdentifier("cleanItem-\(i.rel)")
    }

    private func open(_ i: CleanItem) {
        if inside[i.path] != nil {
            inside[i.path] = nil
            return
        }
        inside[i.path] = .some(nil)
        Task {
            do { inside[i.path] = .some(try await store.cleanFolders(agentId: i.agent, rel: i.rel)) } catch {
                inside[i.path] = nil
                self.error = APIError.from(error)
            }
        }
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

    private func tick(_ path: String, _ on: Bool) {
        if on { ticked.insert(path) } else { ticked.remove(path) }
    }

    @ViewBuilder
    private func logSections(_ log: CleanLog) -> some View {
        Section {
            Text("\(log.running && log.lines.isEmpty ? "Removing…" : "Freed \(bytes(log.freedBytes)) · \(log.removed) removed") · \(log.skipped.count) skipped\(log.failed > 0 ? " · \(log.failed) failed" : "")")
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
        let paths = chosen.map(\.path)
        copied = false
        log = CleanLog(at: nil, what: "", title: agentId.map { "Cleaning \(name($0))…" } ?? "Cleaning all agents…", running: true, lines: [], skipped: [], freedBytes: 0, removed: 0, failed: 0)
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
