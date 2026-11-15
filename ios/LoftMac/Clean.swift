import AppKit
import LoftKit
import SwiftUI

/// Settings › Disk › Check space: every agent's workspace items in one table, to filter, sort and tick what goes. A
/// main clone opens to its folders one level at a time; a clone's git can be checked on a click; an idle agent's
/// leftover processes and Stop them sit before its first row. Delete selected shows each removal as it happens, then
/// the summary. Opened on a log, it shows that one.
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
    @State private var error: String?
    @State private var copied = false
    /// Git check results by path; nil while checking.
    @State private var checks: [String: GitCheck?] = [:]
    @State private var sort: CleanSort = .size
    @State private var agentFilter = ""
    @State private var deletableOnly = false
    @State private var left: [String: [LeftProcess]] = [:]
    @State private var stoppingId: String?

    private var known: [CleanItem] { (items ?? []) + inside.values.flatMap { $0 ?? [] } }
    private var chosen: [CleanItem] { known.filter { ticked.contains($0.path) } }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(title).font(.title2.bold())
                if items == nil && log == nil { ProgressView().controlSize(.small) }
                Spacer()
            }
            if let error { Text(error).foregroundStyle(Theme.failureInk).accessibilityIdentifier("cleanError") }
            if let log {
                logView(log)
            } else if let items {
                picker(items)
            } else {
                Text("Measuring workspaces…").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .padding(20)
        .frame(width: 940, height: 620)
        .interactiveDismissDisabled(log?.running == true)
        .task {
            switch start {
            case .log(let l): log = l
            case .check: await load()
            }
        }
    }

    private var title: String {
        if let log { return log.title ?? (log.running ? "Cleaning…" : log.what.isEmpty ? "Clean" : log.what) }
        return "Clean"
    }

    private func name(_ id: String) -> String { names[id] ?? "Deleted agent \(id.prefix(8))" }

    /// Lists every agent's items; nothing is ticked.
    private func load() async {
        do {
            if store.info?.supports(.leftProcesses) == true { left = (try? await store.leftRunning()) ?? [:] }
            items = try await store.cleanPreview(agentId: nil)
            ticked = []
            inside = [:]
            error = nil
        } catch { self.error = APIError.from(error).title }
    }

    // MARK: - The table

    @ViewBuilder private func picker(_ items: [CleanItem]) -> some View {
        HStack(spacing: 12) {
            Picker("Agent", selection: $agentFilter) {
                Text("All agents").tag("")
                ForEach(Array(Set(items.map(\.agent))).sorted { name($0) < name($1) }, id: \.self) { Text(name($0)).tag($0) }
            }
            .frame(width: 220)
            Picker("Sort by", selection: $sort) {
                ForEach(CleanSort.allCases) { Text($0.rawValue).tag($0) }
            }
            .frame(width: 200)
            Toggle("Deletable only", isOn: $deletableOnly)
            Spacer()
        }
        let shown = sortedClean(items.filter { (agentFilter.isEmpty || $0.agent == agentFilter) && (!deletableOnly || $0.canTick) }, by: sort, agentName: name)
        List {
            if items.isEmpty { Text("Nothing in any workspace.").foregroundStyle(.secondary) }
            ForEach(Array(shown.enumerated()), id: \.element.id) { n, i in
                if !shown[..<n].contains(where: { $0.agent == i.agent }), let ps = left[i.agent], !ps.isEmpty, i.blocked != "Working" {
                    leftRunning(i.agent, ps)
                }
                rows([i], depth: 0)
            }
        }
        .listStyle(.bordered(alternatesRowBackgrounds: true))
        .accessibilityIdentifier("cleanTable")
        if !items.isEmpty {
            Text("Commands are the equivalent of each removal; Loft removes the paths itself.").font(.caption).foregroundStyle(.secondary)
        }
        HStack {
            let total = chosen.reduce(0) { $0 + $1.bytes }
            Text(chosen.isEmpty ? "Nothing selected" : "\(chosen.count) selected · \(bytesText(total))")
                .foregroundStyle(.secondary)
                .accessibilityIdentifier("cleanSummary")
            Spacer()
            copyButton(chosen.map { ($0.agent, $0.command) })
            Button("Close") { dismiss() }.keyboardShortcut(.cancelAction)
            Button("Delete selected", role: .destructive) { confirm() }
                .buttonStyle(.accent)
                .disabled(chosen.isEmpty || !store.canWrite)
                .accessibilityIdentifier("cleanConfirm")
        }
    }

    /// What an idle agent left running, and Stop them.
    private func leftRunning(_ agent: String, _ ps: [LeftProcess]) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text("\(name(agent)): \(ps.count) process\(ps.count == 1 ? "" : "es") still running").fontWeight(.medium)
                Spacer()
                Button(stoppingId == agent ? "Stopping…" : "Stop them") {
                    stoppingId = agent
                    Task {
                        do { try await store.stopLeftRunning(agentId: agent) } catch { self.error = APIError.from(error).title }
                        await load()
                        stoppingId = nil
                    }
                }
                .controlSize(.small)
                .disabled(stoppingId != nil || !store.canWrite)
                .accessibilityIdentifier("stop-\(agent)")
            }
            ForEach(ps) { p in
                HStack {
                    Text(p.command).font(.caption.monospaced()).lineLimit(1).truncationMode(.middle)
                    Spacer()
                    Text(p.ageText).font(.caption)
                }
                .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("left-\(agent)")
    }

    private func rows(_ items: [CleanItem], depth: Int) -> AnyView {
        AnyView(ForEach(items) { i in
            row(i, depth: depth)
            if let sub = inside[i.path] {
                if let sub {
                    if sub.isEmpty {
                        Text("No folders inside.").font(.caption).foregroundStyle(.secondary).padding(.leading, CGFloat(depth + 1) * 18 + 40)
                    } else {
                        rows(sub, depth: depth + 1)
                    }
                } else {
                    ProgressView().controlSize(.small).padding(.leading, CGFloat(depth + 1) * 18 + 40)
                }
            }
        })
    }

    private func row(_ i: CleanItem, depth: Int) -> some View {
        let status = i.status
        return HStack(alignment: .top, spacing: 10) {
            Text(depth == 0 ? name(i.agent) : "").foregroundStyle(.secondary).frame(width: 130, alignment: .leading).lineLimit(1)
            HStack(alignment: .top, spacing: 6) {
                Toggle("", isOn: Binding(get: { ticked.contains(i.path) }, set: { on in if on { ticked.insert(i.path) } else { ticked.remove(i.path) } }))
                    .toggleStyle(.checkbox)
                    .labelsHidden()
                    .disabled(status.locked || log != nil)
                    .accessibilityLabel("Select \(name(i.agent)) \(i.rel)")
                    .accessibilityIdentifier("tick-\(i.rel)")
                if i.opens {
                    Button { open(i) } label: {
                        Image(systemName: "chevron.right").rotationEffect(.degrees(inside[i.path] != nil ? 90 : 0))
                    }
                    .buttonStyle(.borderless)
                    .accessibilityLabel(inside[i.path] != nil ? "Close \(i.name)" : "Open \(i.name)")
                }
                VStack(alignment: .leading, spacing: 2) {
                    Text(i.name).font(.body.monospaced()).lineLimit(1).truncationMode(.middle)
                    if ticked.contains(i.path), !i.warning.isEmpty, checks[i.path] == nil {
                        Text(i.warning).font(.caption).foregroundStyle(Theme.attentionInk)
                    }
                    if i.canCheck, depth == 0 { checkLine(i) }
                    if !i.note.isEmpty { Text(i.note).font(.caption).foregroundStyle(.secondary) }
                }
            }
            .padding(.leading, CGFloat(depth) * 18)
            .frame(maxWidth: .infinity, alignment: .leading)
            Text(i.typeText).foregroundStyle(.secondary).frame(width: 90, alignment: .leading)
            Text(bytesText(i.bytes)).monospacedDigit().frame(width: 80, alignment: .trailing)
            Text(i.changed.shortAgo).foregroundStyle(.secondary).frame(width: 70, alignment: .leading)
            VStack(alignment: .leading, spacing: 2) {
                Text(status.text).foregroundStyle(status.locked ? Theme.attentionInk : Theme.working).lineLimit(3)
                if !status.locked, !i.label.isEmpty { Text(i.label).font(.caption).foregroundStyle(.secondary) }
            }
            .frame(width: 180, alignment: .leading)
            .accessibilityIdentifier("status-\(i.rel)")
        }
        .font(.callout)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("cleanItem-\(i.rel)")
    }

    @ViewBuilder private func checkLine(_ i: CleanItem) -> some View {
        switch checks[i.path] {
        case .some(.none):
            HStack(spacing: 6) { ProgressView().controlSize(.mini); Text("Checking git…") }.font(.caption).foregroundStyle(.secondary)
        case .some(.some(let c)):
            Text(c.text).font(.caption)
                .foregroundStyle(c.state == "safe" ? Theme.working : c.state == "unsafe" ? Theme.attentionInk : Color.secondary)
                .accessibilityIdentifier("check-\(i.rel)")
            Button("Check again") { check(i) }.buttonStyle(.link).foregroundStyle(Theme.accent).font(.caption)
        case .none:
            Button("Check git") { check(i) }.buttonStyle(.link).foregroundStyle(Theme.accent).font(.caption).accessibilityIdentifier("checkButton-\(i.rel)")
        }
    }

    private func check(_ i: CleanItem) {
        checks[i.path] = .some(nil)
        Task {
            let c: GitCheck
            do { c = try await store.cleanCheck(agentId: i.agent, rel: i.rel) } catch {
                c = GitCheck(state: "unknown", lines: ["Can't check: \(APIError.from(error).title)"])
            }
            checks[i.path] = .some(c)
        }
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
                self.error = APIError.from(error).title
            }
        }
    }

    private func copyButton(_ items: [(agent: String, command: String)]) -> some View {
        Button(copied ? "Copied" : "Copy commands") {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(commandsText(items.filter { !$0.command.isEmpty }, name: name) + "\n", forType: .string)
            copied = true
        }
        .disabled(items.allSatisfy { $0.command.isEmpty })
    }

    // MARK: - The log

    @ViewBuilder private func logView(_ log: CleanLog) -> some View {
        Text("\(log.running && log.lines.isEmpty ? "Removing…" : "Freed \(bytesText(log.freedBytes)) · \(log.removed) removed") · \(log.skipped.count) skipped\(log.failed > 0 ? " · \(log.failed) failed" : "")")
            .accessibilityIdentifier("cleanLogSummary")
        List {
            ForEach(groupedByAgent(log.lines, \.agent), id: \.agent) { group in
                Section(name(group.agent)) {
                    ForEach(group.items) { l in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(l.text).font(.callout.monospaced()).foregroundStyle(l.error == nil ? Color.primary : Theme.failureInk)
                            if !l.command.isEmpty { Text(l.command).font(.caption.monospaced()).foregroundStyle(.secondary) }
                        }
                        .textSelection(.enabled)
                    }
                }
            }
            if !log.running && log.lines.isEmpty { Text("Nothing was removed.").foregroundStyle(.secondary) }
            if !log.skipped.isEmpty {
                Section("Skipped") {
                    ForEach(Array(log.skipped.enumerated()), id: \.offset) { _, k in
                        HStack {
                            Text(name(k.agent)) + Text(k.rel.isEmpty ? "" : " / \(k.rel)").font(.callout.monospaced())
                            Spacer()
                            Text(k.reason).foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
        .listStyle(.bordered)
        HStack {
            copyButton(log.lines.map { ($0.agent, $0.command) })
            Spacer()
            Button(log.running ? "Cleaning…" : "Done") { dismiss() }
                .keyboardShortcut(.defaultAction)
                .disabled(log.running)
                .accessibilityIdentifier("cleanDone")
        }
    }

    private func confirm() {
        let paths = chosen.map(\.path)
        copied = false
        log = CleanLog(at: nil, what: "", title: "Cleaning…", running: true, lines: [], skipped: [], freedBytes: 0, removed: 0, failed: 0)
        Task {
            // The log fills in as Loft removes each folder.
            let poll = Task {
                while !Task.isCancelled {
                    try? await Task.sleep(for: .milliseconds(500))
                    if let l = try? await store.cleanLog(), l.running, !Task.isCancelled { log = l }
                }
            }
            defer { poll.cancel() }
            do {
                log = try await store.clean(agentId: nil, paths: paths)
                await onCleaned()
            } catch {
                log = nil
                self.error = APIError.from(error).title
            }
        }
    }
}
