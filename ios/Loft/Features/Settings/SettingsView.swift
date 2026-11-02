import Speech
import SwiftUI
import UserNotifications

struct SettingsView: View {
    @Environment(LoftStore.self) private var store
    @AppStorage(Prefs.appearance) private var appearance = "dark"
    @AppStorage(Prefs.notifyApprovals) private var notifyApprovals = true
    @AppStorage(Prefs.notifyTaskOutcomes) private var notifyTaskOutcomes = true
    @AppStorage(Prefs.notifyFailures) private var notifyFailures = true
    @AppStorage(Prefs.pushQuietHours) private var quietHours = false
    @AppStorage(Prefs.pushQuietStart) private var quietStart = "22:00"
    @AppStorage(Prefs.pushQuietEnd) private var quietEnd = "07:00"
    private var push: PushRegistrar { .shared }

    private var pushStatus: String {
        if store.info?.supports(.push) != true { return "On this iPhone only" }
        if push.registered { return "Push on" }
        return push.token == nil ? "Waiting for Apple" : "Not registered yet"
    }

    private func pushKindBinding(_ k: PushKind) -> Binding<Bool> {
        Binding(get: { k.isOn() },
                set: { UserDefaults.standard.set($0, forKey: k.prefKey); Task { await push.register() } })
    }

    /// "HH:mm" ⇄ a date today, for the pickers; a change re-registers with the new hours.
    private func quietTime(_ value: Binding<String>) -> Binding<Date> {
        let f = DateFormatter()
        f.dateFormat = "HH:mm"
        return Binding(get: {
            let d = f.date(from: value.wrappedValue) ?? Date()
            let c = Calendar.current.dateComponents([.hour, .minute], from: d)
            return Calendar.current.date(bySettingHour: c.hour ?? 22, minute: c.minute ?? 0, second: 0, of: Date()) ?? Date()
        }, set: {
            value.wrappedValue = f.string(from: $0)
            Task { await push.register() }
        })
    }
    @AppStorage(Prefs.speechLocale) private var speechLocale = ""
    @AppStorage(Prefs.speechOnDeviceOnly) private var onDeviceOnly = false
    // Same UserDefaults keys VoiceSession.handsFree/readReplies read, so the engine sees every change.
    @AppStorage(Prefs.voiceReadReplies) private var readReplies = true
    @AppStorage(Prefs.voiceHandsFree) private var handsFree = true
    @AppStorage(Prefs.grillReadAloud) private var grillReadAloud = true
    @AppStorage(Prefs.voiceInterrupt) private var interruptByVoice = true
    @AppStorage(Prefs.voiceShortRead) private var shortRead = true
    @AppStorage(Prefs.voiceBrief) private var offerBrief = true
    @AppStorage(Prefs.aiCommands) private var aiCommands = true
    @AppStorage(Prefs.aiShortSpoken) private var aiShortSpoken = true
    @AppStorage(Prefs.aiNeedsYou) private var aiNeedsYou = true
    @AppStorage(Prefs.aiQuickAck) private var aiQuickAck = true
    @AppStorage(Prefs.aiRoute) private var aiRoute = true
    @AppStorage(Prefs.aiChatReplies) private var aiChatReplies = true
    @AppStorage(Prefs.aiExplain) private var aiExplain = true
    @AppStorage(Prefs.speechVoice) private var speechVoice = ""
    @State private var notificationStatus: UNAuthorizationStatus = .notDetermined
    @State private var confirmForget = false
    @State private var copied = false

    /// An Apple Intelligence switch with its one-line explanation; off and disabled while the model can't run.
    private func aiToggle(_ title: String, _ value: Binding<Bool>, enabled: Bool = true, id: String, note: String) -> some View {
        let usable = OnDeviceModel.status.isAvailable && enabled
        return Toggle(isOn: usable ? value : .constant(false)) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                Text(note).font(.caption).foregroundStyle(.secondary)
            }
        }
        .disabled(!usable)
        .accessibilityIdentifier(id)
    }

    private var interruptNote: String {
        (interruptByVoice ? " While it reads, say “stop”, “skip” or “repeat” to cut it short." : "")
            + (shortRead ? " Long replies start with their opening; say “more” for the rest." : "")
    }

    var body: some View {
        @Bindable var store = store
        Form {
            Section("Connection") {
                LabeledContent("Status") {
                    Label(store.connection.label, systemImage: "circle.fill")
                        .labelStyle(StatusDotLabel(color: store.connection.color))
                }
                LabeledContent("Server") {
                    Text(store.serverAddress ?? "—").lineLimit(1).truncationMode(.middle)
                }
                if let mac = store.info?.host, mac != store.api?.baseURL.host() {
                    LabeledContent("Mac") { Text(mac).lineLimit(1).truncationMode(.middle) }
                }
                if let v = store.info?.version { LabeledContent("Loft version", value: v) }
                if let sync = store.lastSync { LabeledContent("Last sync", value: sync.formatted(date: .abbreviated, time: .standard)) }
                LabeledContent("Authentication", value: "None · tailnet only")
                Button("Reconnect") { store.reconnect() }
                NavigationLink("Change server") { ConnectView(isOnboarding: false) }
            }

            Section {
                Picker("Agent", selection: $store.orchestratorId) {
                    Text("None").tag(String?.none)
                    ForEach(store.sortedAgents) { Text($0.name).tag(String?.some($0.id)) }
                }
            } header: {
                Text("Orchestrator")
            } footer: {
                Text("The agent the Chat tab opens first and that “Plan & delegate” hands tasks to. Its own permissions apply.")
            }

            Section {
                if notificationStatus == .denied {
                    Label("Notifications are off for Loft.", systemImage: "bell.slash").foregroundStyle(.secondary)
                    if let url = URL(string: UIApplication.openSettingsURLString) {
                        Link("Open Settings", destination: url).accessibilityIdentifier("openNotificationSettings")
                    }
                } else if notificationStatus == .notDetermined {
                    Button("Allow notifications") {
                        Task {
                            _ = await store.notifier.requestAuthorization()
                            notificationStatus = await store.notifier.authorizationStatus()
                            await push.start()
                        }
                    }
                } else {
                    LabeledContent("Allowed", value: pushStatus).accessibilityIdentifier("pushStatus")
                    if push.registered {
                        Button("Send test push") { Task { await push.sendTest() } }
                            .accessibilityIdentifier("sendTestPush")
                        if let t = push.lastTestAt {
                            LabeledContent("Last test push", value: t.formatted(date: .omitted, time: .shortened))
                        }
                    }
                    if let e = push.lastError { Text(e).font(.footnote).foregroundStyle(Theme.failure) }
                }
                Toggle("Approval requests", isOn: $notifyApprovals)
                Toggle("Task done, failed or blocked", isOn: $notifyTaskOutcomes)
                Toggle("Agent run failures", isOn: $notifyFailures)
            } header: {
                Text("Notifications")
            } footer: {
                Text(store.info?.supports(.push) == true
                     ? "These three come from this iPhone while Loft is open or was just in use. With Loft closed, the Push kinds below decide what reaches you; the same item never alerts twice."
                     : "Shown while Loft is open or was just in use. This Loft can't send push notifications yet.")
            }

            if store.info?.supports(.push) == true, notificationStatus != .denied {
                Section {
                    ForEach(PushKind.allCases) { k in
                        Toggle(k.label, isOn: pushKindBinding(k)).accessibilityIdentifier("pushKind-\(k.rawValue)")
                    }
                    Toggle("Quiet hours", isOn: $quietHours.onChange { Task { await push.register() } })
                        .accessibilityIdentifier("pushQuietHours")
                    if quietHours {
                        DatePicker("From", selection: quietTime($quietStart), displayedComponents: .hourAndMinute)
                        DatePicker("Until", selection: quietTime($quietEnd), displayedComponents: .hourAndMinute)
                    }
                } header: {
                    Text("Push, when Loft is closed")
                } footer: {
                    Text("Sent by your Mac. They say which agent needs you and why, never what's in a message. Quiet hours hold pushes until they end, except permission requests that block an agent.")
                }
            }

            Section {
                Picker("Language", selection: $speechLocale) {
                    Text("System (\(Self.systemLanguageName))").tag("")
                    ForEach(Self.speechLocales, id: \.identifier) { l in
                        Text(Locale.current.localizedString(forIdentifier: l.identifier) ?? l.identifier).tag(l.identifier)
                    }
                }
                Toggle("On-device only", isOn: $onDeviceOnly)
            } header: {
                Text("Dictation")
            } footer: {
                Text("Loft uses on-device recognition when your language supports it. With “On-device only”, audio never leaves this iPhone and dictation is unavailable for languages without on-device support.")
            }

            Section {
                Toggle("Read replies aloud", isOn: $readReplies)
                    .accessibilityIdentifier("readRepliesToggle")
                Toggle("Hands-free", isOn: $handsFree)
                    .accessibilityIdentifier("handsFreeToggle")
                Toggle("Stop or skip by voice", isOn: $interruptByVoice)
                    .accessibilityIdentifier("interruptByVoiceToggle")
                Toggle("Read long replies short", isOn: $shortRead)
                    .accessibilityIdentifier("shortReadToggle")
                if store.supportsBrief {
                    Toggle(isOn: $offerBrief) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Offer a brief for long dictations")
                            Text("Written by Claude on your Mac. Your words stay one tap away.").font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    .accessibilityIdentifier("voiceBriefToggle")
                }
                NavigationLink {
                    VoicePickerView()
                } label: {
                    LabeledContent("Voice", value: VoiceChoice.displayName(for: speechVoice))
                }
                .accessibilityIdentifier("voiceRow")
                NavigationLink("Voice log") { VoiceLogView() }
                    .accessibilityIdentifier("voiceLogRow")
            } header: {
                Text("Voice conversation")
            } footer: {
                Text("Tap the mic in a chat, or tap Talk on the Overview. Take your time: pauses never send. Say “send” to send, “review” to stop and edit, or “cancel” to throw it away. After 5 seconds of quiet Loft asks “Would you like to submit this?”: say yes (it's tidied up and sent, with Undo), no, or just keep talking. With hands-free on, Loft reads the reply and listens again; say “stop” to end." + interruptNote)
            }

            if store.supportsGrill {
                Section {
                    Toggle("Read questions aloud", isOn: $grillReadAloud)
                        .accessibilityIdentifier("grillReadAloudToggle")
                } header: {
                    Text("Interviews")
                } footer: {
                    Text("When an agent grills you, it asks in rounds. Each question is read with its recommendation in your chosen voice and, with hands-free on, the mic opens for your answer. Say “yes” to accept the recommendation, or “skip”, “back”, “repeat” or “I don't know” any time; the round is sent after its last question. Spoken answers follow “When you stop talking” above.")
                }
            }

            Section {
                LabeledContent("On-device model", value: OnDeviceModel.status.text)
                    .accessibilityIdentifier("onDeviceModelStatus")
                // Off and disabled while the model can't run, so it doesn't look like it's working; the setting is kept.
                aiToggle("Understand voice commands", $aiCommands, id: "aiCommandsToggle",
                         note: "Tells an answer from a command like “skip” when the usual words don't match. It only ever asks.")
                aiToggle("Short spoken versions", $aiShortSpoken, enabled: shortRead, id: "aiShortSpokenToggle",
                         note: "Reads a long reply as a few plain sentences; say “more” to hear it all. Needs “Read long replies short”.")
                aiToggle("Summaries in Needs you", $aiNeedsYou, id: "aiNeedsYouToggle",
                         note: "One line per item and suggested replies, which only fill in the reply box.")
                aiToggle("Quick first reply", $aiQuickAck, id: "aiQuickAckToggle",
                         note: "Says at once what the Orchestrator will look into, until the real reply comes.")
                aiToggle("Suggest who it's for", $aiRoute, id: "aiRouteToggle",
                         note: "Offers “Send to …?” when a message looks meant for one agent. Nothing goes without your tap.")
                aiToggle("Suggested replies in chats", $aiChatReplies, id: "aiChatRepliesToggle",
                         note: "When an agent asks you something, offers two or three replies. A tap fills in the reply; you send it.")
                aiToggle("Explain permission requests", $aiExplain, id: "aiExplainToggle",
                         note: "“What does this do?” on an approval says it in plain words. The risk labels come from the app, not the model, and you still decide.")
            } header: {
                Text("Apple Intelligence")
            } footer: {
                Text("Runs on this iPhone, works offline and sends nothing anywhere. When it's not available, Loft works as before.")
            }

            Section("Appearance") {
                Picker("Theme", selection: $appearance) {
                    Text("Dark").tag("dark")
                    Text("Light").tag("light")
                    Text("System").tag("system")
                }
                .pickerStyle(.segmented)
            }

            Section {
                Text(diagnostics)
                    .font(.caption.monospaced())
                    .textSelection(.enabled)
                Button(copied ? "Copied" : "Copy diagnostics") {
                    UIPasteboard.general.string = diagnostics
                    copied = true
                }
            } header: {
                Text("Diagnostics")
            } footer: {
                Text("Safe to share: no messages or credentials.")
            }

            Section("About") {
                LabeledContent("App version", value: Self.appVersion)
                Button("Forget this server", role: .destructive) { confirmForget = true }
            }
        }
        .navigationTitle("Settings")
        .task { notificationStatus = await store.notifier.authorizationStatus() }
        .confirmationDialog("Forget this server?", isPresented: $confirmForget, titleVisibility: .visible) {
            Button("Forget", role: .destructive) { store.forgetServer() }
        } message: {
            Text("The saved address and cached data on this iPhone are removed. Nothing changes on the Mac.")
        }
    }

    /// The device language as a name ("English (United States)"), without region overrides like "@rg=iszzzz"
    /// that `Locale.current.identifier` carries when the region setting differs from the language.
    static var systemLanguageName: String {
        let id = Locale.current.language.languageCode.map { code in
            [code.identifier, Locale.current.language.region?.identifier].compactMap { $0 }.joined(separator: "_")
        } ?? Locale.current.identifier
        return Locale.current.localizedString(forIdentifier: id) ?? id
    }

    static var speechLocales: [Locale] {
        SFSpeechRecognizer.supportedLocales().sorted {
            (Locale.current.localizedString(forIdentifier: $0.identifier) ?? $0.identifier)
                < (Locale.current.localizedString(forIdentifier: $1.identifier) ?? $1.identifier)
        }
    }

    static var appVersion: String {
        let v = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "?"
        let b = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "?"
        return "\(v) (\(b))"
    }

    private var diagnostics: String {
        var lines = [
            "app \(Self.appVersion) · iOS \(UIDevice.current.systemVersion)",
            "server \(store.api?.baseURL.absoluteString ?? "none") · mac \(store.info?.host ?? "?")",
            "server version \(store.info?.version ?? "?") · features \(store.info?.features?.joined(separator: ",") ?? "none")",
            "connection \(store.connection.label)",
            "last sync \(store.lastSync.map { JSONCoding.formatDate($0) } ?? "never")",
            "agents \(store.agents.count) · tasks \(store.tasks.count) · pending approvals \(store.pendingApprovals().count) · unsent \(store.pending.count)",
        ]
        if case .reconnecting(let attempt, let e) = store.connection {
            lines.append("last error (attempt \(attempt)): \(e.diagnostic)")
        }
        return lines.joined(separator: "\n")
    }
}

private struct StatusDotLabel: LabelStyle {
    let color: Color
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 6) {
            Circle().fill(color).frame(width: 8, height: 8)
            configuration.title
        }
    }
}
