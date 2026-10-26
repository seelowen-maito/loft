import SwiftUI

/// A task brief or a delegated-subtask report, shown as a compact row instead of a full bubble.
/// Briefs show "Task: <title>" and expand to the description; reports show the outcome in one line.
struct TaskMessageRow: View {
    @Environment(LoftStore.self) private var store
    let message: ChatMessage
    @State private var expanded = false

    private var task: HBTask? { store.task(message.referencedTaskId) }
    private var status: String? { message.meta?["task_status"]?.stringValue }

    /// The brief's text without its first line (the title); empty when there's nothing more.
    private var details: String {
        let lines = message.displayContent.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: false)
        return lines.count > 1 ? String(lines[1]).trimmingCharacters(in: .whitespacesAndNewlines) : ""
    }

    private var title: String {
        if let t = task?.title { return t }
        let text = message.displayContent
        // Reports read “Title”, which you delegated to me, is now …
        if let open = text.firstIndex(of: "“"), let close = text[open...].firstIndex(of: "”") {
            return String(text[text.index(after: open)..<close])
        }
        return text.split(separator: "\n").first.map(String.init) ?? "Task"
    }

    /// What came after "is now <status>: " in a report.
    private var outcome: String? {
        guard let r = message.displayContent.range(of: ": ", options: .backwards) else { return nil }
        let s = message.displayContent[r.upperBound...].trimmingCharacters(in: .whitespacesAndNewlines)
        return s.isEmpty ? nil : s
    }

    var body: some View {
        if message.isTaskBrief { brief } else { report }
    }

    // MARK: Brief

    private var brief: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button {
                withAnimation(.snappy) { expanded.toggle() }
            } label: {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Image(systemName: "list.clipboard").foregroundStyle(Theme.accent)
                    (Text("Task: ").foregroundStyle(.secondary) + Text(title).fontWeight(.semibold))
                        .multilineTextAlignment(.leading)
                    Spacer(minLength: 4)
                    if !details.isEmpty {
                        Image(systemName: "chevron.down")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.tertiary)
                            .rotationEffect(.degrees(expanded ? 180 : 0))
                    }
                }
                .font(.subheadline)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(details.isEmpty)
            .accessibilityLabel("Task: \(title)")
            .accessibilityHint(details.isEmpty ? "" : expanded ? "Hides the description" : "Shows the description")

            if expanded {
                Text(details).font(.footnote).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .transition(.opacity)
            }
            footer(from: message.fromAgentName.map { "From \($0)" } ?? "Handed over by you")
        }
        .padding(12)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Theme.accent.opacity(0.35)))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("taskBrief")
    }

    // MARK: Report

    private var report: some View {
        let (symbol, color, verb) = Self.style(status)
        return VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: symbol).foregroundStyle(color)
                Group {
                    Text(title).fontWeight(.semibold) + Text(" is \(verb)") + Text(outcome.map { ": \($0)" } ?? "")
                }
                .font(.subheadline)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            footer(from: message.fromAgentName.map { "From \($0)" } ?? "")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(color.opacity(0.1), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("taskReport")
    }

    static func style(_ status: String?) -> (symbol: String, color: Color, verb: String) {
        switch status {
        case "done": ("checkmark.circle.fill", Theme.working, "done")
        case "failed": ("xmark.circle.fill", Theme.failure, "failed")
        case "blocked": ("pause.circle.fill", Theme.attention, "blocked")
        case "cancelled": ("minus.circle.fill", .secondary, "cancelled")
        case let s?: ("circle.fill", .secondary, "now " + s.replacingOccurrences(of: "_", with: " "))
        case nil: ("circle.fill", .secondary, "updated")
        }
    }

    // MARK: Shared

    private func footer(from: String) -> some View {
        HStack(spacing: 6) {
            if !from.isEmpty { Text(from) }
            Text(message.createdAt.formatted(date: .omitted, time: .shortened))
            Spacer(minLength: 0)
            if let t = task {
                NavigationLink(value: TaskRoute(id: t.id)) {
                    Text("Open task").fontWeight(.semibold).foregroundStyle(Theme.accent)
                }
                .accessibilityIdentifier("openTask")
            }
        }
        .font(.caption2)
        .foregroundStyle(.secondary)
    }
}
