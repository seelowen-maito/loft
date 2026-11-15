import LoftKit
import SwiftUI

/// Settings: the web's sections, saved together with only what changed, plus this Mac's Loft address. Backups' Back
/// up now, Clean, Herdr UI, Push notifications and Mac access act at once, as on the web.
struct SettingsSection: View {
    @Environment(LoftStore.self) private var store
    /// `GET /api/settings`'s `settings` as saved, and as edited here.
    @State private var saved: [String: JSONValue]?
    @State private var draft: [String: JSONValue] = [:]
    /// Write-only: nil keeps the saved token, "" removes it.
    @State private var token: String?
    @State private var tokenSet = false
    @State private var pendingRestart: [String] = []
    @State private var runningAgents = 0
    @State private var saving = false
    @State private var restarting = false
    @State private var error: String?
    @State private var folderError: String?

    private var dirty: Bool { saved.map { $0 != draft } == true || token != nil }

    var body: some View {
        Form {
            Section {
                AddressForm(isOnboarding: false).padding(.vertical, 4)
            }
            Section { AppleIntelligenceSection().toggleStyle(.checkbox).padding(.vertical, 4) }
            if saved != nil {
                if !pendingRestart.isEmpty || restarting { restartBanner }
                general
                BackupsSettings(folder: binding("backup_dir", ""), folderError: folderError)
                DiskSettings(idleHours: Binding(get: { Int(draft["idle_timeout_hours"]?.numberValue ?? 4) },
                                                set: { draft["idle_timeout_hours"] = .number(Double(max(1, $0))) }))
                GithubBudgetSettings()
                SpendSettings()
                Section("Updates") {
                    Toggle("Update automatically", isOn: bool("auto_update"))
                    note("Checks for new commits every minute. Installs once no agent is running, then restarts.")
                }
                Section("Agents") {
                    Toggle("Messaging rules", isOn: bool("messaging_rules")).accessibilityIdentifier("messagingRulesToggle")
                    note("Tells every agent to message other agents only with a finding, report, verdict or request, never thanks or waiting notes.")
                }
                Section("GitHub issues") {
                    Picker("Comments going to GitHub", selection: binding("comment_approval_default", "ask")) {
                        Text("Ask for every comment").tag("ask")
                        Text("Auto-approve my own comments").tag("own")
                        Text("Auto-approve everything").tag("all")
                    }
                    Toggle("Publish new Loft issues to GitHub automatically", isOn: bool("auto_publish_all"))
                    note("Each new Loft issue in a project with a GitHub repo becomes a GitHub issue right away. Off: it stays in Loft until you press Publish to GitHub.")
                }
                Section("Navigation") {
                    Toggle("Show Simulator", isOn: bool("show_simulator"))
                    note("Adds a tab to the web app to view and control a booted iOS Simulator on this Mac.")
                }
                Section("Keyboard shortcuts") {
                    ForEach(Self.shortcuts, id: \.0) { s in LabeledContent(s.1, value: s.0) }
                }
                HerdrSettings()
                PushSettings()
                MacAccessSettings()
            } else if let error {
                Text(error).foregroundStyle(Theme.failureInk)
            } else {
                ProgressView().frame(maxWidth: .infinity)
            }
        }
        .formStyle(.grouped)
        .frame(maxWidth: 760)
        .frame(maxWidth: .infinity)
        .safeAreaInset(edge: .bottom) { if dirty || (error != nil && saved != nil) { saveBar } }
        .navigationTitle("Settings")
        .task { await load(resetDraft: true) }
    }

    static let shortcuts = [("⌘1 … ⌘5", "Go to Overview, Agents, Issues, Needs you, Settings"), ("⌘N", "New message"),
                            ("⌥⌘I", "Show or hide an agent's settings"), ("⌘R", "Refresh"), ("⌘S", "Save an agent's settings")]

    private func note(_ s: String) -> some View { Text(s).font(.caption).foregroundStyle(.secondary) }

    private func binding(_ key: String, _ fallback: String) -> Binding<String> {
        Binding(get: { draft[key]?.stringValue ?? fallback }, set: { draft[key] = .string($0) })
    }

    private func bool(_ key: String) -> Binding<Bool> {
        Binding(get: { draft[key]?.boolValue ?? false }, set: { draft[key] = .bool($0) })
    }

    private var general: some View {
        Section("General") {
            LabeledContent {
                TextField("Projects folder", text: binding("projects_dir", "")).labelsHidden().font(.body.monospaced())
                    .accessibilityIdentifier("projectsDirField")
            } label: {
                Text("Projects folder")
                Text("Where your git repos live. Needs a restart.")
            }
            LabeledContent {
                TextField("Claude command", text: binding("claude_bin", "")).labelsHidden().font(.body.monospaced())
            } label: {
                Text("Claude command")
                Text("What agents run.")
            }
            LabeledContent {
                TextField("SSH host", text: binding("ssh_host", ""), prompt: Text("none")).labelsHidden().font(.body.monospaced())
            } label: {
                Text("SSH host")
                Text("How other machines reach this one, e.g. loft. Leave empty if you only use Loft on this machine.")
            }
            LabeledContent {
                HStack {
                    SecureField("GitHub token", text: Binding(get: { token ?? "" }, set: { token = $0 }),
                                prompt: Text(tokenSet && token == nil ? "•••••••• saved" : "ghp_…"))
                        .labelsHidden()
                    if tokenSet { Button("Remove") { token = "" } }
                }
            } label: {
                Text("GitHub token")
                Text(tokenSet ? "A token is saved. Type a new one to replace it." : "Needed for private repos and merging PRs. Without one, Loft uses gh auth token.")
            }
        }
    }

    private var restartBanner: some View {
        Section {
            HStack {
                Image(systemName: "arrow.clockwise").foregroundStyle(Theme.attentionInk)
                if restarting {
                    Text("Restarting Loft…")
                } else {
                    Text("Restart to apply \(pendingRestart.map { $0 == "projects_dir" ? "Projects folder" : $0 }.joined(separator: ", ")).")
                    if runningAgents > 0 { Text("Waiting for \(runningAgents) running agent\(runningAgents == 1 ? "" : "s") to finish.").foregroundStyle(.secondary) }
                }
                Spacer()
                if !restarting {
                    Button("Restart now") { Task { await restart() } }
                        .disabled(runningAgents > 0 || !store.canWrite)
                        .accessibilityIdentifier("restartNow")
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("restartBanner")
        }
    }

    private var saveBar: some View {
        HStack(spacing: 12) {
            if let error {
                Text(error).foregroundStyle(Theme.failureInk).lineLimit(2)
            } else {
                Text("Unsaved changes").foregroundStyle(.secondary)
            }
            Spacer()
            if dirty {
                Button("Discard") { draft = saved ?? [:]; token = nil; error = nil; folderError = nil }
                Button(saving ? "Saving…" : "Save") { Task { await save() } }
                    .buttonStyle(.accent)
                    .keyboardShortcut("s")
                    .disabled(saving || !store.canWrite)
                    .accessibilityIdentifier("settingsSaveAll")
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.bar)
    }

    private func load(resetDraft: Bool) async {
        do {
            let v = try await store.settingsView()
            guard case .object(let s)? = v["settings"] else { throw APIError.unsupported("Server settings") }
            saved = s
            if resetDraft { draft = s }
            tokenSet = v["github_token_set"]?.boolValue ?? false
            if case .array(let a)? = v["pending_restart"] { pendingRestart = a.compactMap(\.stringValue) } else { pendingRestart = [] }
            runningAgents = Int(v["running_agents"]?.numberValue ?? 0)
            if resetDraft { error = nil }
        } catch {
            self.error = APIError.from(error).title
        }
    }

    /// Only what changed goes; the backups folder after the rest, so a folder Loft refuses fails only that field.
    private func save() async {
        guard let saved else { return }
        saving = true
        defer { saving = false }
        error = nil
        folderError = nil
        var changes = draft.filter { saved[$0.key] != $0.value }
        let folder = changes.removeValue(forKey: "backup_dir")
        if let token { changes["github_token"] = .string(token) }
        do {
            if !changes.isEmpty { try await store.saveSettings(changes) }
            if let folder {
                do { try await store.saveBackupDir(folder.stringValue ?? "") } catch { folderError = APIError.from(error).title }
            }
            token = nil
            let keep = draft
            await load(resetDraft: false)
            // A refused folder stays in the field to fix.
            draft = self.saved ?? keep
            if folderError != nil, let folder { draft["backup_dir"] = folder; error = folderError }
        } catch {
            self.error = APIError.from(error).title
        }
    }

    private func restart() async {
        do { try await store.restart() } catch { self.error = APIError.from(error).title; return }
        restarting = true
        defer { restarting = false }
        // Wait for the new process to answer.
        for _ in 0..<60 {
            try? await Task.sleep(for: .seconds(1))
            await load(resetDraft: false)
            if pendingRestart.isEmpty, error == nil { break }
        }
    }
}

/// The last backup and its error, Back up now, and where backups go (saved with the other settings).
private struct BackupsSettings: View {
    @Environment(LoftStore.self) private var store
    @Binding var folder: String
    let folderError: String?
    @State private var status: BackupStatus?
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        Section("Backups") {
            LabeledContent("Last backup") {
                if let last = status?.last {
                    Text("\(last.at.formatted(.relative(presentation: .named))) · \(last.at.formatted(date: .abbreviated, time: .shortened)) · \(bytesText(last.bytes))")
                } else {
                    Text(status == nil ? "" : "No backup yet.")
                }
            }
            .accessibilityIdentifier("backupLast")
            if let e = error ?? status?.error.map({ "The last backup failed \($0.at.formatted(.relative(presentation: .named))): \($0.message)" }) {
                Label(e, systemImage: "exclamationmark.triangle.fill").foregroundStyle(Theme.failureInk).accessibilityIdentifier("backupError")
            }
            Button(busy ? "Backing up…" : "Back up now") { Task { await backUp() } }
                .disabled(busy || !store.canWrite)
                .accessibilityIdentifier("backUpNow")
            Picker("Back up to", selection: Binding(get: { status?.folder(folder) ?? .local }, set: { f in
                switch f {
                case .local: folder = ""
                case .icloud: folder = status?.icloud ?? folder
                case .other: break
                }
            })) {
                Text("This Mac").tag(BackupStatus.Folder.local)
                if status?.icloud != nil { Text("iCloud Drive").tag(BackupStatus.Folder.icloud) }
                if case .other(let d)? = status?.folder(folder) { Text("Other folder").tag(BackupStatus.Folder.other(d)) }
            }
            .pickerStyle(.segmented)
            if status?.icloud != nil, folder == status?.icloud {
                Text("Backups in iCloud Drive leave this Mac: all your chats and your GitHub token are then stored by Apple. Save to use it.")
                    .font(.caption).foregroundStyle(Theme.attentionInk)
            }
            LabeledContent {
                TextField("Backups folder", text: $folder, prompt: Text(status?.dir ?? "~/Library/Application Support/Loft/backups"))
                    .labelsHidden().font(.body.monospaced())
            } label: {
                Text("Backups folder")
                Text("Loft backs up its database and settings once a day, owner-only, and keeps 7 daily and 4 weekly backups. Point it at an external disk to survive this Mac's disk.")
            }
            if let folderError { Text(folderError).font(.caption).foregroundStyle(Theme.failureInk) }
        }
        .task { status = try? await store.backups().status }
    }

    private func backUp() async {
        busy = true
        defer { busy = false }
        do {
            status = try await store.backUpNow()
            error = nil
        } catch {
            self.error = APIError.from(error).title
            // The failure is recorded: show the last good backup next to it.
            status = try? await store.backups().status
        }
    }
}

/// Disk: free space, the agents' Simulator devices, the idle timeout (saved with the rest), Check space and the last
/// run's log.
private struct DiskSettings: View {
    @Environment(LoftStore.self) private var store
    @Binding var idleHours: Int
    @State private var sizes: WorkspaceSizes?
    @State private var devices: [AgentDevice]?
    @State private var last: CleanLog?
    @State private var sheet: CleanSheet.Start?

    var body: some View {
        Section("Disk") {
            HStack {
                Text(sizes.map { s in s.freeBytes.map { "\(bytesText($0)) free\(s.isLow ? " · disk almost full" : "")" } ?? "Free space unknown." } ?? "Measuring…")
                    .foregroundStyle(sizes?.isLow == true ? Theme.attentionInk : .primary)
                    .accessibilityIdentifier("diskFree")
                Spacer()
                Button("Check space") { sheet = .check }
                    .disabled(store.info?.supports(.cleanPicker) != true)
                    .accessibilityIdentifier("checkSpace")
            }
            if let last, let at = last.at {
                HStack {
                    Text("\(last.title ?? last.what) \(at.formatted(.relative(presentation: .named))) · freed \(bytesText(last.freedBytes)) from \(last.removed) item\(last.removed == 1 ? "" : "s")")
                    if last.failed > 0 { Text("· \(last.failed) failed").foregroundStyle(Theme.failureInk) }
                    Spacer()
                    Button("View log") { sheet = .log(last) }.buttonStyle(.link).foregroundStyle(Theme.accent)
                }
                .accessibilityIdentifier("lastClean")
            }
            ForEach(devices ?? []) { d in
                LabeledContent {
                    HStack(spacing: 12) {
                        Text(d.stateText).foregroundStyle(.secondary)
                        Text(bytesText(d.bytes)).monospacedDigit()
                    }
                } label: {
                    Label(d.title, systemImage: "iphone")
                }
            }
            Stepper(value: $idleHours, in: 1...max(168, idleHours)) {
                LabeledContent {
                    Text(idleHours == 1 ? "1 hour" : "\(idleHours) hours")
                } label: {
                    Text("Idle timeout")
                    Text("After this long with no run, Loft stops the processes the agent left running and shuts its Simulator device down.")
                }
            }
            .accessibilityIdentifier("idleTimeout")
            Text("Check space lists what's in every agent's workspace; Delete selected removes what you tick. A clone, a main clone too, can be ticked once git says everything in it is pushed (the agent clones it again); notes and uploads stay unless you tick them. Loft also removes build output untouched for 3 days, daily.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .task { await load() }
        .sheet(item: $sheet) { start in
            CleanSheet(start: start, names: Dictionary(sizes?.agents.map { ($0.id, $0.name) } ?? [], uniquingKeysWith: { a, _ in a })) { await load() }
        }
    }

    private func load() async {
        sizes = try? await store.workspaceSizes()
        if store.info?.supports(.cleanPicker) == true { last = try? await store.cleanLog() }
        if store.info?.supports(.agentDevices) == true { devices = try? await store.agentDevices() }
    }
}

/// What's left of GitHub's hourly limit (shared with the agents' own `gh`), and where Loft's requests went.
private struct GithubBudgetSettings: View {
    @Environment(LoftStore.self) private var store
    @State private var budget: JSONValue?

    private static let sources = ["prs": "Pull request cards", "mirror": "Issue mirror", "repos": "Repos page", "inbox": "GitHub inbox",
                                  "user": "Your clicks", "agent": "Agents' Loft tools", "other": "Other"]

    var body: some View {
        Section("GitHub API") {
            if case .object(let resources)? = budget?["resources"], !resources.isEmpty {
                ForEach(resources.keys.sorted(), id: \.self) { name in
                    let r = resources[name]
                    let limit = r?["limit"]?.numberValue ?? 1, left = r?["remaining"]?.numberValue ?? 0
                    let reset = Date(timeIntervalSince1970: r?["reset"]?.numberValue ?? 0)
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Text("\(name == "core" ? "REST" : name == "graphql" ? "GraphQL" : name): \(Int(left)) of \(Int(limit)) left")
                            Spacer()
                            Text("resets \(reset.formatted(date: .omitted, time: .shortened))").font(.caption).foregroundStyle(.secondary)
                        }
                        ProgressView(value: max(0, min(1, left / limit)))
                            .tint(left / limit < 0.1 ? Theme.failure : left / limit < 0.3 ? Theme.attention : Theme.working)
                    }
                }
                Text("The limit is per GitHub account, so the agents' own gh commands use it too.").font(.caption).foregroundStyle(.secondary)
                if case .object(let by)? = budget?["by_source"], !by.isEmpty {
                    let rows = by.sorted { ($0.value["requests"]?.numberValue ?? 0) > ($1.value["requests"]?.numberValue ?? 0) }
                    ForEach(rows, id: \.key) { k, v in
                        LabeledContent(Self.sources[k] ?? k,
                                       value: "\(Int(v["requests"]?.numberValue ?? 0)) counted · \(Int(v["not_modified"]?.numberValue ?? 0)) free")
                    }
                }
            } else {
                Text("No answer from GitHub yet since Loft started.").foregroundStyle(.secondary)
            }
        }
        .task {
            while !Task.isCancelled {
                budget = try? await store.githubBudget()
                try? await Task.sleep(for: .seconds(15))
            }
        }
    }
}

/// All-time spend: every run Loft has counted.
private struct SpendSettings: View {
    @Environment(LoftStore.self) private var store
    @State private var all: AgentsTotals.Spend?

    var body: some View {
        Section("Spend") {
            Text(all.map { "All time: \($0.runs.formatted()) runs · est. \($0.costUsd.formatted(.currency(code: "USD")))" } ?? " ")
                .help("Every run since Loft started counting, deleted agents and cleared chats included")
                .accessibilityIdentifier("allTimeSpend")
            Text("Estimated at API list prices.").font(.caption).foregroundStyle(.secondary)
        }
        .task { all = try? await store.agentsTotals().allTime }
    }
}

/// Mac access: whether macOS lets Loft's agents into the folders it guards with a prompt.
private struct MacAccessSettings: View {
    @Environment(LoftStore.self) private var store
    @State private var check: JSONValue?
    @State private var busy = false
    @State private var error: String?

    private static let states = ["allowed": "allowed", "blocked": "blocked", "waiting": "macOS is asking on the Mac", "missing": "not on this Mac"]

    var body: some View {
        Section {
            HStack {
                Text("macOS asks before any app reads Desktop, Documents, Downloads or iCloud Drive. Checking can show those prompts on the Mac.")
                    .font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button(busy ? "Checking…" : check == nil ? "Check" : "Check again") { Task { await run() } }
                    .disabled(busy || !store.canWrite)
            }
            if case .array(let items)? = check?["items"] {
                ForEach(Array(items.enumerated()), id: \.offset) { _, i in
                    let state = i["state"]?.stringValue ?? ""
                    LabeledContent {
                        Text(Self.states[state] ?? state).foregroundStyle(state == "allowed" ? Theme.working : state == "blocked" ? Theme.failureInk : Theme.attentionInk)
                    } label: {
                        Text(i["label"]?.stringValue ?? "")
                        Text(i["folder"]?.stringValue ?? "").font(.caption.monospaced())
                    }
                }
            }
            if let binary = check?["binary"]?.stringValue,
               case .array(let items)? = check?["items"],
               items.contains(where: { ["blocked", "waiting"].contains($0["state"]?.stringValue ?? "") }) {
                Text("To let agents in for good: System Settings → Privacy & Security → Full Disk Access, press +, then ⇧⌘G and paste \(binary). Turn it on, then Check again.")
                    .font(.caption)
            }
            if let error { Text(error).foregroundStyle(Theme.failureInk) }
        } header: {
            Text("Mac access")
        }
    }

    private func run() async {
        busy = true
        defer { busy = false }
        do { check = try await store.macAccess(); error = nil } catch { self.error = APIError.from(error).title }
    }
}
