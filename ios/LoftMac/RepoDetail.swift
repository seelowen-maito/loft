import LoftKit
import SwiftUI

/// A repo opened from Overview, as the web's repo drawer: its numbers, daily activity, open PRs and issues, languages,
/// contributors and recent commits.
struct RepoDetail: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    let repo: RepoSummary
    let gh: RepoGithub?
    @State private var commits: [RepoCommit]?

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(repo.path).font(.caption.monospaced()).foregroundStyle(.secondary)
                    Text(repo.name).font(.largeTitle.weight(.semibold))
                    if let d = gh?.description { Text(d).foregroundStyle(.secondary) }
                }
                Spacer()
                Button("Close") { dismiss() }.keyboardShortcut(.cancelAction)
            }
            .padding(20)
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    Text("\(repo.branch.isEmpty ? "detached" : repo.branch) · \(localText(repo))").font(.callout.monospaced())
                        .foregroundStyle(repo.isDirty ? Theme.attentionInk : .secondary)
                    HStack(spacing: 12) {
                        tile("Commits", repo.totalCommits.formatted())
                        tile("Branches", repo.branches.map(String.init) ?? "—")
                        tile("Stars", gh?.error == nil ? gh?.stars.map(String.init) ?? "—" : "—")
                        tile("Forks", gh?.error == nil ? gh?.forks.map(String.init) ?? "—" : "—")
                    }
                    VStack(alignment: .leading, spacing: 8) {
                        eyebrow("Daily activity")
                        ScrollView(.horizontal) { Heatmap(daily: repo.daily, cell: 10) }
                    }
                    .accessibilityIdentifier("repoDailyActivity")
                    if let e = gh?.error {
                        Label(e, systemImage: "exclamationmark.triangle.fill").foregroundStyle(Theme.attentionInk)
                    } else if let gh {
                        HStack(alignment: .top, spacing: 20) {
                            miniList("Pull requests", gh.pulls)
                            miniList("Issues", gh.issues)
                        }
                    }
                    HStack(alignment: .top, spacing: 20) {
                        VStack(alignment: .leading, spacing: 8) {
                            eyebrow("Languages")
                            if let l = repo.languages, !l.isEmpty { ShareBars(items: l) } else { Text("—").foregroundStyle(.secondary) }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        VStack(alignment: .leading, spacing: 6) {
                            eyebrow("Contributors")
                            ForEach(repo.contributors ?? [], id: \.name) { c in
                                HStack {
                                    Text(c.name).lineLimit(1)
                                    Spacer()
                                    Text("\(c.count)").font(.caption.monospaced()).foregroundStyle(.secondary)
                                }
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    VStack(alignment: .leading, spacing: 8) {
                        eyebrow("Recent commits")
                        if let commits {
                            ForEach(commits) { c in
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(c.subject).lineLimit(1)
                                    HStack(spacing: 8) {
                                        Text(String(c.hash.prefix(7)))
                                        Text(c.author)
                                        Text(c.date.shortAgo)
                                        if let i = c.insertions { Text("+\(i)").foregroundStyle(Theme.working) }
                                        if let d = c.deletions { Text("−\(d)").foregroundStyle(Theme.failureInk) }
                                    }
                                    .font(.caption.monospaced()).foregroundStyle(.secondary)
                                }
                            }
                            if commits.isEmpty { Text("No commits.").foregroundStyle(.secondary) }
                        } else {
                            ProgressView().controlSize(.small)
                        }
                    }
                    .accessibilityIdentifier("repoRecentCommits")
                }
                .padding(20)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .frame(width: 640, height: 680)
        .task { commits = (try? await store.repoCommits(name: repo.name)) ?? [] }
    }

    private func tile(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            eyebrow(label)
            Text(value).font(.system(size: 24, weight: .semibold, design: .rounded)).monospacedDigit()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }

    private func miniList(_ title: String, _ items: [RepoGithub.Item]) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            eyebrow("\(title) · \(items.count)")
            if items.isEmpty { Text("None open.").foregroundStyle(.secondary) }
            ForEach(items.prefix(8)) { i in
                Button {
                    if let u = PushDestination.link(i.url, loftHost: store.api?.baseURL.host()) { openURL(u) }
                } label: {
                    (Text("#\(i.number) ").font(.callout.monospaced()).foregroundStyle(.secondary) + Text(i.title)).lineLimit(1)
                }
                .buttonStyle(.plain)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

func eyebrow(_ s: String) -> some View {
    Text(s.uppercased()).font(.caption.weight(.medium)).foregroundStyle(.secondary)
}

/// What's local in a repo: changes, ahead, behind, or clean.
func localText(_ r: RepoSummary) -> String {
    var parts: [String] = []
    let changes = r.staged + r.modified + r.untracked
    if changes > 0 { parts.append("\(changes) changed") }
    if r.ahead > 0 { parts.append("\(r.ahead) ahead") }
    if r.behind > 0 { parts.append("\(r.behind) behind") }
    return parts.isEmpty ? "clean" : parts.joined(separator: " · ")
}

/// The web's contribution grid: `daily` oldest first, ending today, one column per week ending on today's weekday.
struct Heatmap: View {
    let daily: [Int]
    var cell: CGFloat = 12
    var gap: CGFloat = 3

    var body: some View {
        let today = Calendar.current.component(.weekday, from: .now) - 1
        let padded = daily + Array(repeating: -1, count: 6 - today)
        let lead = (7 - padded.count % 7) % 7
        let all = Array(repeating: -1, count: lead) + padded
        let cols = stride(from: 0, to: all.count, by: 7).map { Array(all[$0..<min($0 + 7, all.count)]) }
        let peak = max(1, daily.max() ?? 1)
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .top, spacing: gap) {
                ForEach(cols.indices, id: \.self) { ci in
                    VStack(spacing: gap) {
                        ForEach(cols[ci].indices, id: \.self) { ri in
                            let v = cols[ci][ri]
                            RoundedRectangle(cornerRadius: 2.5)
                                .fill(v < 0 ? Color.clear : Self.color(v, peak))
                                .frame(width: cell, height: cell)
                                .help(v < 0 ? "" : "\(v) commit\(v == 1 ? "" : "s") · \(day(daily.count - 1 - (ci * 7 + ri - lead)))")
                        }
                    }
                }
            }
            HStack(spacing: 4) {
                Text("less")
                ForEach(0..<5) { l in RoundedRectangle(cornerRadius: 2).fill(Self.levels[l]).frame(width: 10, height: 10) }
                Text("more")
            }
            .font(.caption2.monospaced()).foregroundStyle(.secondary)
        }
        .accessibilityElement()
        .accessibilityLabel("Daily commit activity, \(daily.reduce(0, +)) commits")
    }

    private static let levels: [Color] = [Color.secondary.opacity(0.15), Theme.accent.opacity(0.3), Theme.accent.opacity(0.5), Theme.accent.opacity(0.75), Theme.accent]

    private static func color(_ v: Int, _ peak: Int) -> Color {
        guard v > 0 else { return levels[0] }
        let r = Double(v) / Double(peak)
        return levels[r > 0.66 ? 4 : r > 0.33 ? 3 : r > 0.12 ? 2 : 1]
    }

    private func day(_ daysAgo: Int) -> String {
        (Calendar.current.date(byAdding: .day, value: -daysAgo, to: .now) ?? .now).formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day())
    }
}

/// Commits per week, the last 26 weeks, oldest first.
func weeklyCommits(_ daily: [Int], weeks: Int = 26) -> [Int] {
    (0..<weeks).reversed().map { w in
        let end = daily.count - w * 7
        return daily[max(0, end - 7)..<max(0, end)].reduce(0, +)
    }
}

struct WeeklyBars: View {
    let daily: [Int]
    var height: CGFloat = 44

    var body: some View {
        let series = weeklyCommits(daily)
        let peak = max(1, series.max() ?? 1)
        HStack(alignment: .bottom, spacing: 2) {
            ForEach(series.indices, id: \.self) { i in
                let v = series[i]
                RoundedRectangle(cornerRadius: 1)
                    .fill(Theme.accent.opacity(0.9))
                    .frame(height: v == 0 ? 0 : max(2, CGFloat(v) / CGFloat(peak) * (height - 4)))
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)
                    .contentShape(Rectangle())
                    .help("\(v) commits · week of \((Calendar.current.date(byAdding: .day, value: -((series.count - 1 - i) * 7 + 6), to: .now) ?? .now).formatted(.dateTime.month(.abbreviated).day()))")
            }
        }
        .frame(height: height)
        .overlay(alignment: .bottom) { Rectangle().fill(Color.secondary.opacity(0.4)).frame(height: 1) }
        .accessibilityHidden(true)
    }
}

/// Horizontal share bars: name, bar, percent.
struct ShareBars: View {
    let items: [Share]

    var body: some View {
        let total = max(1, items.reduce(0) { $0 + $1.count })
        Grid(alignment: .leading, horizontalSpacing: 8, verticalSpacing: 6) {
            ForEach(items, id: \.name) { s in
                GridRow {
                    Text(s.name).lineLimit(1).frame(width: 90, alignment: .leading)
                    ProgressView(value: Double(s.count) / Double(total)).tint(Theme.accent)
                    Text("\(Int((Double(s.count) / Double(total) * 100).rounded()))%").font(.caption.monospaced()).foregroundStyle(.secondary)
                }
                .help("\(s.count) files")
            }
        }
    }
}
