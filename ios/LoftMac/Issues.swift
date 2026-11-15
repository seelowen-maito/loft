import LoftKit
import SwiftUI

/// The Issues section, the web's single board: the Issues column first, then the specs in stage columns, with the
/// All / Issues / Specs filter on top. A card opens its page; a spec's page has its Delivery and its tickets.
struct IssuesSection: View {
    @Environment(LoftStore.self) private var store
    @Environment(MacNav.self) private var nav
    @AppStorage("boardType") private var type = BoardType.all
    @AppStorage("boardShowClosed") private var showClosed = false
    @AppStorage("boardShowDropped") private var showDropped = false

    private var stages: [SpecStage] { SpecStage.allCases.filter { $0 != .dropped || showDropped } }

    var body: some View {
        @Bindable var nav = nav
        NavigationStack(path: $nav.issuePath) {
            Group {
                if store.info != nil && !store.supportsTasks {
                    ContentUnavailableView("Issues need a newer Loft", systemImage: "checklist",
                                           description: Text("This server doesn't have the issues API yet. Pull the latest Loft and restart it."))
                } else {
                    board
                }
            }
            .navigationTitle("Issues")
            .navigationDestination(for: String.self) { IssuePage(taskId: $0) }
        }
        .onAppear { store.deliveryWatchers += 1; store.loadDeliveries() }
        .onDisappear { store.deliveryWatchers -= 1 }
    }

    private var board: some View {
        let deliveries = Array(store.deliverySummaries.values)
        return VStack(spacing: 0) {
            HStack(spacing: 14) {
                Picker("Show", selection: $type) {
                    ForEach(BoardType.allCases) { Text($0.label).tag($0) }
                }
                .pickerStyle(.segmented)
                .fixedSize()
                .accessibilityIdentifier("boardType")
                if type != .specs { Toggle("Show closed", isOn: $showClosed).accessibilityIdentifier("boardShowClosed") }
                if type != .issues { Toggle("Show dropped", isOn: $showDropped).accessibilityIdentifier("boardShowDropped") }
                Spacer()
            }
            .toggleStyle(.checkbox)
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            Divider()
            ScrollView(.horizontal) {
                HStack(alignment: .top, spacing: 12) {
                    if type != .specs {
                        let issues = SpecViews.boardIssues(store.tasks, deliveries, showClosed: showClosed)
                        BoardColumn(title: "Issues", count: issues.count, id: "issues") {
                            ForEach(issues) { IssueCard(task: $0) }
                        }
                    }
                    if type != .issues {
                        ForEach(stages) { stage in
                            let specs = SpecViews.specs(store.tasks, in: stage, store.deliverySummaries)
                            BoardColumn(title: stage.label, count: specs.count, id: stage.rawValue, tone: stage.tone) {
                                ForEach(specs) { SpecCard(spec: $0) }
                            }
                        }
                    }
                }
                .padding(16)
            }
            .frame(maxHeight: .infinity)
        }
        .overlay {
            if store.lastSync == nil { ProgressView() }
        }
    }
}

/// One column of the board: its title and count, and its cards scrolling under them.
private struct BoardColumn<Content: View>: View {
    let title: String
    let count: Int
    let id: String
    var tone: Color = .secondary
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(title.uppercased()).foregroundStyle(tone)
                Spacer()
                Text("\(count)").foregroundStyle(.secondary)
            }
            .font(.caption.weight(.semibold).monospaced())
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("boardColumn-\(id)")
            ScrollView {
                LazyVStack(spacing: 8) {
                    content
                    if count == 0 {
                        Text("Nothing here").font(.caption).foregroundStyle(.secondary).padding(.vertical, 16)
                    }
                }
                .padding([.horizontal, .bottom], 8)
            }
        }
        .frame(width: 290)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(Color.secondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }
}

private struct Card<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        content
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(10)
            .background(.background, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Color.secondary.opacity(0.2)))
            .contentShape(Rectangle())
    }
}

/// An issue's card: open or closed, its title, where it lives, its project, when it last changed and who has it.
private struct IssueCard: View {
    @Environment(LoftStore.self) private var store
    let task: HBTask

    var body: some View {
        NavigationLink(value: task.id) {
            Card {
                VStack(alignment: .leading, spacing: 5) {
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Image(systemName: task.state.isClosed ? "checkmark.circle" : "circle.circle")
                            .foregroundStyle(task.state.isClosed ? Color.purple : Theme.working)
                            .accessibilityLabel(task.state.isClosed ? "Closed" : "Open")
                        Text(task.title).foregroundStyle(task.state.isClosed ? .secondary : .primary).lineLimit(3)
                    }
                    HStack(spacing: 8) {
                        Text(task.github.map { "#\($0.number)" } ?? "Loft")
                        if !task.project.isEmpty { Text(task.project) }
                        Text(task.updatedAt, format: .relative(presentation: .named))
                        Spacer(minLength: 0)
                        if let a = store.agent(task.agentId) { Text(a.name).lineLimit(1) }
                    }
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                }
            }
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("issueCard-\(task.title)")
    }
}

/// A spec's card: its title and Delivery badge, its tickets' progress, who is on which ticket, where it lives and its
/// spec PR; below, a dot per ticket that opens the ticket list.
private struct SpecCard: View {
    @Environment(LoftStore.self) private var store
    let spec: HBTask
    @State private var showsTickets = false

    var body: some View {
        let d = SpecViews.delivery(of: spec, in: store.deliverySummaries)
        let tickets = SpecViews.tickets(of: spec, d, tasks: store.tasks)
        Card {
            VStack(alignment: .leading, spacing: 6) {
                NavigationLink(value: spec.id) {
                    VStack(alignment: .leading, spacing: 6) {
                        HStack(alignment: .firstTextBaseline) {
                            Text(spec.title).lineLimit(3)
                            Spacer(minLength: 4)
                            if let d { DeliveryBadge(state: d.state) }
                        }
                        if let p = SpecViews.progress(of: spec, d, tasks: store.tasks) { ProgressRow(done: p.done, total: p.total) }
                        if let d {
                            ForEach(SpecViews.onIt(d), id: \.self) { Text($0).font(.caption).foregroundStyle(.secondary).lineLimit(2) }
                            if let line = SpecViews.waitingOnYouLine(d.waitingOnYou) { Text(line).font(.caption).foregroundStyle(Theme.attentionInk) }
                        }
                        HStack(spacing: 8) {
                            Text(SpecViews.ref(spec))
                            if let pr = d?.pr { Label("#\(pr)", systemImage: "arrow.triangle.pull") }
                        }
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("specCard-\(spec.title)")
                if !tickets.isEmpty {
                    Button { withAnimation(.snappy) { showsTickets.toggle() } } label: {
                        HStack(spacing: 6) {
                            Image(systemName: "chevron.right").rotationEffect(.degrees(showsTickets ? 90 : 0)).font(.caption2)
                            HStack(spacing: 3) { ForEach(tickets) { Circle().fill(ticketTone($0.state)).frame(width: 7, height: 7) } }
                            Spacer()
                            Text("\(tickets.count) \(tickets.count == 1 ? "ticket" : "tickets")").font(.caption.monospaced())
                        }
                        .foregroundStyle(.secondary)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(showsTickets ? "Hide tickets" : "Show \(tickets.count) tickets")
                    .accessibilityIdentifier("specTickets-\(spec.title)")
                    if showsTickets {
                        ForEach(tickets) { TicketLink(ticket: $0) }
                    }
                }
            }
        }
    }
}

/// A spec's page: what it is, its Delivery (Start delivery while it waits, then who has which ticket, Pause or
/// Resume), its progress and spec PR, and its tickets, each opening its own issue. Other issues get the same page
/// without the spec parts.
struct IssuePage: View {
    @Environment(LoftStore.self) private var store
    let taskId: String

    var body: some View {
        if let t = store.task(taskId) {
            Form {
                Section {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(t.title).font(.title2.weight(.semibold)).textSelection(.enabled).accessibilityIdentifier("issueTitle")
                        HStack(spacing: 8) {
                            Text(SpecViews.isSpec(t) ? SpecViews.ref(t) : t.github.map { "\($0.repo)#\($0.number)" } ?? "Only in Loft")
                            Text(t.state.label)
                            if let a = store.agent(t.agentId) { Text(a.name) }
                            if let labels = t.labels, !labels.isEmpty { Text(labels.joined(separator: ", ")) }
                        }
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                    }
                    if let g = t.github, let url = PushDestination.link(g.url, loftHost: store.api?.baseURL.host()) {
                        Link("Open on GitHub", destination: url).foregroundStyle(Theme.accent).accessibilityIdentifier("issueGitHubLink")
                    }
                    if !t.description.isEmpty {
                        Text(t.description).textSelection(.enabled).font(.callout)
                    }
                    if t.state.isActive && !t.note.isEmpty { LabeledContent("Latest progress", value: t.note) }
                    if !t.error.isEmpty { Text(t.error).foregroundStyle(Theme.failureInk) }
                }
                if t.hasDeliveryCard { DeliveryCard(task: t) }
                if SpecViews.isSpec(t) { SpecProgress(spec: t, underCard: t.hasDeliveryCard) }
                let children = store.subtasks(of: t.id)
                if !SpecViews.isSpec(t), !children.isEmpty {
                    Section("Sub-issues") {
                        ForEach(children) { c in
                            NavigationLink(value: c.id) { LabeledContent(c.title, value: c.state.label) }
                        }
                    }
                }
            }
            .formStyle(.grouped)
            .navigationTitle(SpecViews.isSpec(t) ? "Spec" : "Issue")
        } else {
            ContentUnavailableView("Issue not found", systemImage: "checklist", description: Text("It may have been deleted."))
        }
    }
}

/// The Delivery card of an open spec or standalone ticket on GitHub, like the web's and the iPhone's: Start delivery
/// until it started, then where it stands, who has which ticket, and Pause or Resume when they apply.
private struct DeliveryCard: View {
    @Environment(LoftStore.self) private var store
    let task: HBTask
    @State private var busy: IssueDeliveryAction?
    @State private var said: (ok: Bool, text: String)?

    var body: some View {
        let d = store.deliveries[task.id]
        let number = task.github?.number ?? 0
        Section("Delivery") {
            VStack(alignment: .leading, spacing: 4) {
                Text(intro(d, number)).foregroundStyle(.secondary)
                ForEach(d?.runningLines ?? [], id: \.self) { Text($0).accessibilityIdentifier("deliveryRunning") }
                if let line = SpecViews.waitingOnYouLine(d?.waitingOnYou) { Text(line).foregroundStyle(Theme.attentionInk) }
                if let line = SpecViews.waitingLine(d?.waitingForLead) { Text(line).foregroundStyle(Theme.attentionInk).accessibilityIdentifier("deliveryWaiting") }
                if let line = d?.line {
                    Text(line.text).fontWeight(.medium).foregroundStyle(color(line.tone)).accessibilityIdentifier("deliveryState")
                }
            }
            ForEach(d?.skippedTickets ?? [], id: \.number) { x in
                Text("Skipped #\(x.number) \(x.title): it \(x.skipped), so it's being built elsewhere. It counts as done when it closes.")
                    .font(.footnote).foregroundStyle(.secondary)
            }
            HStack {
                if d?.started != true { button(.start, "Start delivery", symbol: "play.fill") }
                if d?.showsPause == true { button(.pause, "Pause", symbol: "pause.fill") }
                if d?.showsResume == true { button(.resume, "Resume", symbol: "play.fill") }
                if busy != nil { ProgressView().controlSize(.small) }
            }
            if let said {
                Text(said.text).font(.footnote).foregroundStyle(said.ok ? Color.secondary : Theme.failureInk).accessibilityIdentifier("deliverySaid")
            }
        }
        .task(id: task.id) { try? await store.loadDelivery(task.id) }
        .onAppear { store.showDeliveryCard(task.id) }
        .onDisappear { store.hideDeliveryCard(task.id) }
    }

    private func intro(_ d: IssueDelivery?, _ n: Int) -> String {
        if let d, d.started { return d.progressLine(issue: n) }
        return task.isDeliverableSpec
            ? "Loft builds the open sub-issues of #\(n) one at a time on its spec branch and draft spec PR, starting with the first unblocked one on the team's Dev."
            : "Loft opens a ticket/\(n) branch and a draft PR that closes #\(n), then has the team's Dev build it and QA, Security and Code Review check it."
    }

    private func color(_ tone: IssueDelivery.Tone) -> Color {
        switch tone {
        case .running, .final: Theme.accent
        case .stalled: Theme.failureInk
        case .paused: .secondary
        case .needsYou: Theme.attentionInk
        case .done: Theme.working
        }
    }

    private func button(_ action: IssueDeliveryAction, _ title: String, symbol: String) -> some View {
        Button { run(action) } label: { Label(title, systemImage: symbol) }
            .disabled(busy != nil || !store.canWrite)
            .accessibilityIdentifier("delivery-\(action.rawValue)")
    }

    private func run(_ action: IssueDeliveryAction) {
        busy = action
        said = nil
        Task {
            do {
                said = try await store.deliveryAction(task.id, action).map { (true, $0) }
                store.loadDeliveries()
            } catch {
                said = (false, APIError.from(error).localizedDescription)
            }
            busy = nil
        }
    }
}

/// Under the Delivery card, a spec's progress and spec PR (and its state when there's no card), Merge for a plan kept
/// in Loft that's ready, and its tickets.
private struct SpecProgress: View {
    @Environment(LoftStore.self) private var store
    let spec: HBTask
    let underCard: Bool
    @State private var merging = false
    @State private var result: (ok: Bool, text: String)?

    var body: some View {
        let d = SpecViews.delivery(of: spec, in: store.deliverySummaries)
        let tickets = SpecViews.tickets(of: spec, d, tasks: store.tasks)
        if let d {
            Section(underCard ? "Progress" : "Delivery") {
                if !underCard || d.pr != nil {
                    HStack {
                        if !underCard { DeliveryBadge(state: d.state).accessibilityIdentifier("deliveryState") }
                        Spacer()
                        if let pr = d.pr, let g = spec.github, let url = URL(string: "https://github.com/\(g.repo)/pull/\(pr)") {
                            Link(destination: url) { Label("Spec PR #\(pr)", systemImage: "arrow.triangle.pull") }.foregroundStyle(Theme.accent).accessibilityIdentifier("specPRLink")
                        }
                    }
                }
                if !underCard {
                    ForEach(SpecViews.onIt(d), id: \.self) { Text($0).foregroundStyle(.secondary) }
                    if let line = SpecViews.waitingOnYouLine(d.waitingOnYou) { Text(line).foregroundStyle(Theme.attentionInk) }
                    if let line = SpecViews.waitingLine(d.waitingForLead) { Text(line).foregroundStyle(Theme.attentionInk) }
                }
                if let p = SpecViews.progress(of: spec, d, tasks: store.tasks) {
                    ProgressRow(done: p.done, total: p.total).accessibilityIdentifier("deliveryProgress")
                }
                if SpecViews.canMerge(spec, d) {
                    Button { merge(d) } label: { Label(merging ? "Merging…" : "Merge", systemImage: "arrow.triangle.merge") }
                        .disabled(merging || !store.canWrite)
                        .accessibilityIdentifier("mergeDelivery")
                }
                if let result { Text(result.text).font(.footnote).foregroundStyle(result.ok ? Theme.working : Theme.failureInk) }
            }
        }
        if !tickets.isEmpty {
            Section("Tickets") {
                ForEach(tickets) { TicketLink(ticket: $0) }
            }
        }
    }

    private func merge(_ d: DeliverySummary) {
        merging = true
        result = nil
        Task {
            do { result = (true, try await store.mergeDelivery(d)) } catch { result = (false, APIError.from(error).localizedDescription) }
            merging = false
        }
    }
}

/// A ticket with its state and review verdicts; it opens its own issue when Loft has one.
private struct TicketLink: View {
    let ticket: SpecViews.TicketLine

    var body: some View {
        let row = VStack(alignment: .leading, spacing: 3) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                if let n = ticket.number { Text("#\(n)").font(.caption.monospaced()).foregroundStyle(.secondary) }
                Text(ticket.title).lineLimit(2)
                Spacer(minLength: 4)
                Text(SpecViews.ticketLabel(ticket.state)).font(.caption.weight(.medium)).foregroundStyle(ticketTone(ticket.state))
            }
            if !ticket.verdicts.isEmpty {
                HStack(spacing: 8) {
                    ForEach(ticket.verdicts.sorted { $0.key < $1.key }, id: \.key) { role, v in
                        Label("\(role) \(v == "pass" ? "passed" : "waiting")", systemImage: v == "pass" ? "checkmark.circle.fill" : "clock")
                            .font(.caption2).foregroundStyle(v == "pass" ? Theme.working : .secondary)
                    }
                }
            }
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("ticketRow-\(ticket.title)")
        if let id = ticket.taskId {
            NavigationLink(value: id) { row }.buttonStyle(.plain)
        } else {
            row
        }
    }
}

private struct DeliveryBadge: View {
    let state: String

    var body: some View {
        Text(SpecViews.deliveryLabel(state))
            .font(.caption2.weight(.medium))
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(Theme.accent.opacity(0.15), in: Capsule())
            .foregroundStyle(Theme.accent)
    }
}

private struct ProgressRow: View {
    let done: Int
    let total: Int

    var body: some View {
        HStack(spacing: 8) {
            ProgressView(value: Double(done), total: Double(max(total, 1)))
            Text("\(done)/\(total)").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(done) of \(total) tickets done")
    }
}

/// A ticket state's colour, as on the web: done green, needs you orange, being worked on the accent.
private func ticketTone(_ state: String) -> Color {
    switch state {
    case "closed": Theme.working
    case "needs_human": Theme.attentionInk
    case "started", "review", "closing": Theme.accent
    default: .secondary
    }
}

private extension SpecStage {
    var tone: Color {
        switch self {
        case .planned, .dropped: .secondary
        case .delivery: Theme.accent
        case .ready: Theme.attentionInk
        case .shipped: Theme.working
        }
    }
}
