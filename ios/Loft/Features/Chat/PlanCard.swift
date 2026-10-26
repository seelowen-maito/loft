import SwiftUI

/// A delegated plan in the chat of the agent that made it: progress, and each subtask with who's on it,
/// its status, its latest line and its PRs. Refreshes as the subtasks or PRs change.
struct PlanCard: View {
    @Environment(LoftStore.self) private var store
    let taskId: String
    let fallback: String

    /// Changes whenever one of the plan's subtasks (or a PR) does, so the card reloads.
    private var signature: String {
        let subs = store.tasks.filter { $0.parentId == taskId || $0.id == taskId }
            .map { "\($0.id)\($0.status)\($0.updatedAt.timeIntervalSince1970)" }.sorted().joined()
        return subs + "\(store.pullRequests.count)"
    }

    var body: some View {
        Group {
            if let plan = store.plans[taskId] { card(plan) } else {
                Text(fallback).font(.footnote).foregroundStyle(.secondary).frame(maxWidth: .infinity)
            }
        }
        .task(id: signature) { await store.loadPlan(taskId) }
    }

    private func card(_ p: TaskPlan) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            NavigationLink(value: TaskRoute(id: p.task.id)) {
                HStack(alignment: .firstTextBaseline) {
                    Label("Plan", systemImage: "list.bullet.rectangle").font(.caption.weight(.semibold)).foregroundStyle(Theme.accent)
                    Text(p.task.title).font(.subheadline.weight(.semibold)).foregroundStyle(.primary).lineLimit(2)
                    Spacer(minLength: 4)
                    Image(systemName: "chevron.right").font(.caption).foregroundStyle(.tertiary)
                }
            }
            .buttonStyle(.plain)
            VStack(alignment: .leading, spacing: 4) {
                ProgressView(value: Double(p.progress.percent), total: 100).tint(p.progress.failed > 0 ? Theme.failure : Theme.accent)
                Text(progressLine(p.progress)).font(.caption).foregroundStyle(.secondary)
            }
            ForEach(p.subtasks) { s in
                NavigationLink(value: TaskRoute(id: s.id)) { row(s) }.buttonStyle(.plain)
            }
        }
        .padding(12)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("planCard")
    }

    private func row(_ s: TaskPlan.Subtask) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: s.state.symbol).foregroundStyle(s.state.color).frame(width: 18)
            VStack(alignment: .leading, spacing: 2) {
                Text(s.title).font(.footnote.weight(.medium)).foregroundStyle(.primary)
                Text([s.agent?.name, s.state.label].compactMap { $0 }.joined(separator: " · ")).font(.caption2).foregroundStyle(.secondary)
                if let line = s.line { Text(line).font(.caption).foregroundStyle(.secondary).lineLimit(2) }
                ForEach(s.prs ?? [], id: \.self) { pr in
                    if let u = pr.url.flatMap(URL.init(string:)) {
                        Link(destination: u) {
                            Label([pr.number.map { "#\($0)" }, pr.title].compactMap { $0 }.joined(separator: " "), systemImage: "arrow.triangle.pull")
                                .font(.caption2).lineLimit(1)
                        }
                    }
                }
            }
            Spacer(minLength: 0)
        }
        .accessibilityElement(children: .combine)
    }

    private func progressLine(_ p: TaskPlan.Progress) -> String {
        var parts = ["\(p.done) of \(p.total) done"]
        if p.inProgress > 0 { parts.append("\(p.inProgress) in progress") }
        if p.waiting > 0 { parts.append("\(p.waiting) waiting") }
        if p.failed > 0 { parts.append("\(p.failed) failed") }
        return parts.joined(separator: " · ")
    }
}
