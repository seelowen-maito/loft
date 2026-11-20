import LoftKit
import SwiftUI

/// Settings › Disk's Clean, a full screen: every workspace's top-level items in one list with agent, type, size, last
/// change and status, sorted by size, agent or last change, to tick what goes; a main clone opens to its folders,
/// one level at a time. An idle agent's leftover processes and Stop them sit before its first row. Then each removal
/// as it happens and the summary. Opened on a log, it shows that one.
struct CleanSheet: View {
    enum Start: Identifiable {
        case check
        case log(CleanLog)
        var id: String {
            switch self {
            case .check: "check"
            case .log: "log"
            }
        }
    }

    let start: Start
    /// Agent names by id; a deleted agent's workspace shows its id.
    let names: [String: String]
    /// After a clean, so the sizes behind refresh.
    var onCleaned: () async -> Void
    @Environment(LoftStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var items: [CleanItem]?
    @State private var ticked: Set<String> = []
    /// Folders listed inside opened items, by path; nil while loading.
    @State private var inside: [String: [CleanItem]?] = [:]
    @State private var log: CleanLog?
    @State private var error: APIError?
    @State private var copied = false
    /// Check results by path; nil while checking. Git runs only when the user taps, and nothing gets ticked.
    @State private var checks: [String: GitCheck?] = [:]
    @State private var sort: CleanSort = .size
    /// What idle agents left running, by agent id, and the agent whose Stop them is working.
    @State private var left: [String: [LeftProcess]] = [:]
    @State private var stoppingId: String?
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
            // Large, so the agent's full name fits: inline, the Remove button squeezes it to "Clean Loft…".
            .navigationBarTitleDisplayMode(.large)
            .toolbar { toolbar }
        }
        .interactiveDismissDisabled(log?.running == true)
        .errorAlert($error)
        .task {
            switch start {
            case .log(let l): log = l
            case .check: await load()
            }
        }
    }

    /// Lists every agent's items; nothing is ticked.
    private func load() async {
        do {
            if store.info?.supports(.leftProcesses) == true { left = (try? await store.leftRunning()) ?? [:] }
            items = try await store.cleanPreview(agentId: nil)
            ticked = []
        } catch { self.error = APIError.from(error) }
    }

    private var title: String {
        if let log { return log.title ?? (log.running ? "Cleaning…" : log.what.isEmpty ? "Clean" : log.what) }
        return "Clean"
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
            ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } }
            ToolbarItem(placement: .confirmationAction) {
                Button("Delete selected", role: .destructive) { confirm() }
                    .disabled(chosen.isEmpty)
                    .accessibilityIdentifier("cleanConfirm")
            }
        }
    }

    @ViewBuilder
    private func previewSections(_ items: [CleanItem]) -> some View {
        Section {
            let total = chosen.reduce(0) { $0 + $1.bytes }
            Text(chosen.isEmpty ? (items.isEmpty ? "Nothing in any workspace." : "Nothing selected") : "\(chosen.count) selected · \(bytes(total))")
                .foregroundStyle(.secondary)
                .accessibilityIdentifier("cleanSummary")
            Picker("Sort by", selection: $sort) {
                ForEach(CleanSort.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .accessibilityIdentifier("cleanSort")
        }
        let sorted = sortedClean(items, by: sort, agentName: name)
        Section {
            ForEach(Array(sorted.enumerated()), id: \.element.id) { n, i in
                if !sorted[..<n].contains(where: { $0.agent == i.agent }), let ps = left[i.agent], !ps.isEmpty, i.blocked != "Working" {
                    leftRunning(i.agent, ps)
                }
                rows([i], depth: 0)
            }
        }
        commandsSection(chosen.map { ($0.agent, $0.command) })
    }

    /// What an idle agent left running, and a button to stop it.
    private func leftRunning(_ agent: String, _ ps: [LeftProcess]) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("\(name(agent)): \(ps.count) process\(ps.count == 1 ? "" : "es") still running").font(.subheadline).lineLimit(nil).fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 8)
                Button(stoppingId == agent ? "Stopping…" : "Stop them") {
                    stoppingId = agent
                    Task {
                        try? await store.stopLeftRunning(agentId: agent)
                        await load()
                        stoppingId = nil
                    }
                }
                .buttonStyle(.bordered).controlSize(.small)
                .disabled(stoppingId != nil || !store.canWrite)
                .accessibilityIdentifier("stop-\(agent)")
            }
            ForEach(ps) { p in
                HStack(alignment: .firstTextBaseline) {
                    Text(p.command).font(.caption.monospaced()).lineLimit(2).truncationMode(.middle)
                    Spacer(minLength: 8)
                    Text(p.ageText).font(.caption).foregroundStyle(.secondary)
                }
                .foregroundStyle(.secondary)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("left-\(agent)")
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
            .disabled(i.status.locked)
            .accessibilityLabel("Select \(i.rel)")
            .accessibilityValue(on ? "ticked" : "not ticked")
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline) {
                    Text(i.name).font(.subheadline.monospaced()).lineLimit(1).truncationMode(.middle)
                    Spacer(minLength: 4)
                    Text(bytes(i.bytes)).font(.subheadline).monospacedDigit()
                }
                Text("\(depth == 0 ? name(i.agent) + " · " : "")\(i.typeText) · changed \(i.changed.formatted(.relative(presentation: .named)))")
                    .font(.caption).foregroundStyle(.secondary)
                Text(i.status.text).font(.caption).foregroundStyle(i.status.locked ? Theme.attentionInk : Color.green)
                    .accessibilityIdentifier("status-\(i.rel)")
                if !i.status.locked && !i.label.isEmpty { Text(i.label).font(.caption).foregroundStyle(.secondary) }
                if on && !i.warning.isEmpty && checks[i.path] == nil { Text(i.warning).font(.caption).foregroundStyle(Theme.attentionInk) }
                if i.canCheck { checkLine(i) }
                if !i.note.isEmpty { Text(i.note).font(.caption).foregroundStyle(.secondary) }
                if !i.shownCommand.isEmpty { Text(i.shownCommand).font(.caption2.monospaced()).foregroundStyle(.secondary).textSelection(.enabled) }
            }
        }
        .padding(.leading, CGFloat(depth) * 16)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("cleanItem-\(i.rel)")
    }

    @ViewBuilder
    private func checkLine(_ i: CleanItem) -> some View {
        switch checks[i.path] {
        case .some(.none):
            HStack(spacing: 6) { ProgressView().controlSize(.mini); Text("Checking git…") }.font(.caption).foregroundStyle(.secondary)
        case .some(.some(let c)):
            Text(c.text).font(.caption)
                .foregroundStyle(c.state == "safe" ? Color.green : c.state == "unsafe" ? Theme.attentionInk : Color.secondary)
                .accessibilityIdentifier("check-\(i.rel)")
            checkButton("Check again", i)
        case .none:
            checkButton("Check", i)
        }
    }

    /// A 44 pt tall tap target, whatever the caption's size.
    private func checkButton(_ title: String, _ i: CleanItem) -> some View {
        Button { check(i) } label: {
            Text(title).font(.caption).frame(minWidth: 44, minHeight: 44, alignment: .leading).contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .accessibilityIdentifier("checkButton-\(i.rel)")
    }

    private func check(_ i: CleanItem) {
        Task { await runCheck(i) }
    }

    private func runCheck(_ i: CleanItem) async {
        checks[i.path] = .some(nil)
        let c: GitCheck
        do { c = try await store.cleanCheck(agentId: i.agent, rel: i.rel) } catch {
            c = GitCheck(state: "unknown", lines: ["Can't check: \(APIError.from(error).localizedDescription)"])
        }
        checks[i.path] = .some(c)
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
                        Text(l.text)
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
        log = CleanLog(at: nil, what: "", title: "Cleaning…", running: true, lines: [], skipped: [], freedBytes: 0, removed: 0, failed: 0)
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
                let done = try await store.clean(agentId: nil, paths: paths)
                log = done
                await onCleaned()
            } catch {
                log = nil
                self.error = APIError.from(error)
            }
        }
    }
}
