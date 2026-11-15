import LoftKit
import SwiftUI

/// Overview: the web's home summary. The repos at a glance, each agent's last 14 days, commit activity, the most
/// active repos, the open pull requests and issues on GitHub, and a repo's detail.
struct OverviewSection: View {
    @Environment(LoftStore.self) private var store
    @Environment(MacNav.self) private var nav
    @State private var repos: [RepoSummary]?
    @State private var github: [RepoGithub]?
    /// By repo name. Names with underscores may come back renamed by the decoder, so `gh(for:)` also matches the slug.
    @State private var githubByName: [String: RepoGithub] = [:]
    @State private var totals: AgentsTotals?
    @State private var tokenSet = true
    @State private var selected: RepoSummary?
    @State private var error: String?

    private var ghErrors: Int { (github ?? []).filter { $0.error != nil }.count }

    private func gh(for name: String) -> RepoGithub? {
        githubByName[name] ?? github?.first { $0.slug?.split(separator: "/").last.map(String.init) == name }
    }

    private var pulls: [(repo: String, item: RepoGithub.Item)] { items(\.pulls) }
    private var issues: [(repo: String, item: RepoGithub.Item)] { items(\.issues) }

    private func items(_ key: KeyPath<RepoGithub, [RepoGithub.Item]>) -> [(repo: String, item: RepoGithub.Item)] {
        (github ?? []).flatMap { g in g[keyPath: key].map { (g.slug ?? "", $0) } }.sorted { $0.item.updatedAt > $1.item.updatedAt }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                if let error {
                    Label(error, systemImage: "exclamationmark.triangle.fill").foregroundStyle(Theme.failureInk)
                }
                stats
                if !tokenSet && ghErrors > 0 { tokenWarning }
                AgentActivity(totals: totals)
                HStack(alignment: .top, spacing: 16) {
                    ActivityPanel(repos: repos)
                    MostActive(repos: repos) { selected = $0 }.frame(width: 300)
                }
                HStack(alignment: .top, spacing: 16) {
                    GithubList(title: "Open pull requests", symbol: "arrow.triangle.pull", rows: pulls, loading: github == nil, empty: "No open pull requests.")
                    GithubList(title: "Open issues", symbol: "smallcircle.filled.circle", rows: issues, loading: github == nil, empty: "No open issues.")
                }
                RepoTable(repos: repos) { selected = $0 }
            }
            .padding(24)
        }
        .sheet(item: $selected) { r in RepoDetail(repo: r, gh: gh(for: r.name)) }
        .navigationTitle("Overview")
        .toolbar {
            Button { Task { await load() } } label: { Label("Refresh", systemImage: "arrow.clockwise") }
                .keyboardShortcut("r")
                .help("Refresh (⌘R)")
        }
        .task { await load() }
    }

    /// The web's stat row: repositories, commits this month and the month before, open PRs and issues, local changes.
    private var stats: some View {
        let commits = repos.map { $0.reduce(0) { $0 + $1.commits30 } }
        let delta = repos.map { (commits ?? 0) - $0.reduce(0) { $0 + $1.commitsPrior30 } }
        return HStack(spacing: 12) {
            StatTile(label: "Repositories", value: repos.map { "\($0.count)" })
            StatTile(label: "Commits · 30d", value: commits.map { "\($0)" }, sub: delta.map { "\($0 >= 0 ? "+" : "")\($0) vs prior 30d" })
            StatTile(label: "Open pull requests", value: github.map { _ in "\(pulls.count)" })
            StatTile(label: "Open issues", value: github.map { _ in "\(issues.count)" })
            StatTile(label: "Uncommitted work", value: repos.map { "\($0.filter(\.isDirty).count)" }, sub: "repos with local changes")
        }
        .accessibilityIdentifier("overviewStats")
    }

    /// The web's "GitHub data is limited" note: some repos couldn't be read and no token is saved.
    private var tokenWarning: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: "key.fill").foregroundStyle(Theme.attentionInk)
            Text("**GitHub data is limited.** \(ghErrors) repo\(ghErrors == 1 ? "" : "s") couldn't be read, probably because \(ghErrors == 1 ? "it's" : "they're") private. Add a GitHub token in Settings to see their pull requests and issues.")
            Spacer()
            Button("Open Settings") { nav.section = .settings }
        }
        .padding(12)
        .background(Theme.attention.opacity(0.1), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("githubLimited")
    }

    private func load() async {
        async let r = store.repos()
        async let g = store.repoGithub()
        async let t = try? store.agentsTotals()
        async let stats: Void = store.refreshStats()
        do {
            repos = try await r
            error = nil
        } catch { self.error = APIError.from(error).title }
        let byName = (try? await g) ?? [:]
        githubByName = byName
        github = Array(byName.values)
        tokenSet = (try? await store.settingsView())?["github_token_set"]?.boolValue ?? true
        totals = await t
        await stats
    }
}

private struct StatTile: View {
    let label: String
    let value: String?
    var sub: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label.uppercased()).font(.caption2.weight(.medium)).foregroundStyle(.secondary)
            Text(value ?? "—").font(.system(size: 28, weight: .semibold, design: .rounded)).monospacedDigit()
            Text(sub ?? " ").font(.caption).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .accessibilityElement(children: .combine)
    }
}

/// Agents · last 14 days: who's working, what waits on you, runs and spend this week, then a card per agent.
private struct AgentActivity: View {
    @Environment(LoftStore.self) private var store
    @Environment(MacNav.self) private var nav
    let totals: AgentsTotals?

    private var summaries: [AgentSummary] {
        store.sortedAgents.compactMap { store.summaries[$0.id] }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                Text("AGENTS · LAST 14 DAYS").font(.caption.weight(.medium)).foregroundStyle(.secondary)
                Spacer()
                Button("Open agents") { nav.section = .agents }.controlSize(.small)
            }
            headline
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 280), spacing: 12)], spacing: 12) {
                ForEach(summaries) { s in
                    Button { nav.openChat(s.agent.id) } label: { AgentCard(s: s) }.buttonStyle(.plain)
                }
            }
        }
    }

    private var headline: some View {
        let working = store.agents.filter { store.state(of: $0) == .working }.count
        let waiting = summaries.reduce(0) { $0 + $1.pendingPermissions }
        var line = Text("\(working) working").foregroundStyle(working > 0 ? Theme.accent : .secondary)
        if waiting > 0 { line = line + Text(" · \(waiting) waiting on you").foregroundStyle(Theme.attentionInk) }
        if let p = totals?.period {
            line = line + Text(" · \(p.runs) runs · \(dollars(p.costUsd)) \(p.kind == "usage_week" ? "this week" : "last 7 days")").foregroundStyle(.secondary)
            if p.previous.costUsd > 0 {
                let pct = Int(((p.costUsd - p.previous.costUsd) / p.previous.costUsd * 100).rounded())
                line = line + Text("  \(pct >= 0 ? "+" : "")\(pct)%").font(.callout.monospaced()).foregroundStyle(.secondary)
            }
        }
        return line.font(.title3).accessibilityIdentifier("agentsHeadline")
    }
}

private func dollars(_ v: Double) -> String { v < 10 ? String(format: "$%.2f", v) : "$\(Int(v.rounded()))" }

private struct AgentCard: View {
    @Environment(LoftStore.self) private var store
    let s: AgentSummary

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 10) {
                Initials(name: s.agent.name, lead: s.agent.isLead, size: 34)
                VStack(alignment: .leading, spacing: 2) {
                    Text(s.agent.name).font(.headline).lineLimit(1)
                    Text(store.state(of: s.agent).label).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                if s.pendingPermissions > 0 {
                    Label("\(s.pendingPermissions) waiting", systemImage: "exclamationmark.shield").font(.caption).foregroundStyle(Theme.attentionInk)
                }
            }
            HStack {
                stat("Runs", "\(s.runs)")
                stat("Tool calls", "\(s.toolCalls)")
                stat("PRs", "\(s.prs.total)")
                stat("Est. spent", s.costUsd.map(dollars) ?? "—")
            }
            RunBars(user: s.dailyUser, auto: s.dailyAuto)
            Text(s.lastReply.map { $0.text.replacingOccurrences(of: "\n", with: " ") } ?? "No replies yet.")
                .font(.caption).foregroundStyle(.secondary).lineLimit(2)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("agentCard-\(s.agent.name)")
    }

    private func stat(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(value).font(.title3.weight(.semibold)).monospacedDigit()
            Text(label.uppercased()).font(.system(size: 9, weight: .medium)).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// Runs per day, oldest first: on request (accent) stacked on autonomous (grey).
private struct RunBars: View {
    let user: [Int]
    let auto: [Int]

    var body: some View {
        let peak = max(1, zip(user, auto).map { $0 + $1 }.max() ?? 1)
        HStack(alignment: .bottom, spacing: 3) {
            ForEach(Array(zip(user, auto).enumerated()), id: \.offset) { _, d in
                VStack(spacing: 1) {
                    Rectangle().fill(Theme.accent).frame(height: CGFloat(d.0) / CGFloat(peak) * 40)
                    Rectangle().fill(Color.secondary.opacity(0.4)).frame(height: CGFloat(d.1) / CGFloat(peak) * 40)
                }
                .frame(maxWidth: .infinity)
            }
        }
        .frame(height: 42, alignment: .bottom)
        .accessibilityHidden(true)
    }
}

private struct GithubList: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.openURL) private var openURL
    let title: String
    let symbol: String
    let rows: [(repo: String, item: RepoGithub.Item)]
    let loading: Bool
    let empty: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("\(title.uppercased()) · \(rows.count)", systemImage: symbol).font(.caption.weight(.medium)).foregroundStyle(.secondary)
            if loading {
                ProgressView().controlSize(.small)
            } else if rows.isEmpty {
                Text(empty).foregroundStyle(.secondary)
            } else {
                ForEach(rows.prefix(8), id: \.item.id) { r in
                    Button {
                        if let u = PushDestination.link(r.item.url, loftHost: store.api?.baseURL.host()) { openURL(u) }
                    } label: {
                        HStack(alignment: .firstTextBaseline) {
                            Text("\(r.repo.split(separator: "/").last.map(String.init) ?? r.repo)#" + String(r.item.number)).font(.caption.monospaced()).foregroundStyle(.secondary)
                            Text(r.item.title).lineLimit(1)
                            if r.item.draft { Text("draft").font(.caption).foregroundStyle(.secondary) }
                            Spacer()
                            Text(r.item.updatedAt.shortAgo).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}

/// Every repo: branch, commits this month, and what's local (changes, ahead, behind).
private struct RepoTable: View {
    let repos: [RepoSummary]?
    let open: (RepoSummary) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("REPOSITORIES").font(.caption.weight(.medium)).foregroundStyle(.secondary)
            if let repos {
                Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
                    GridRow {
                        Text("Name"); Text("Branch"); Text("Commits · 30d"); Text("Local")
                    }
                    .font(.caption).foregroundStyle(.secondary)
                    ForEach(repos.sorted { $0.commits30 > $1.commits30 }) { r in
                        GridRow {
                            Button(r.name) { open(r) }.buttonStyle(.link).fontWeight(.medium)
                                .accessibilityIdentifier("repoRow-\(r.name)")
                            Text(r.branch).font(.callout.monospaced()).foregroundStyle(.secondary)
                            Text("\(r.commits30)").monospacedDigit()
                            Text(localText(r)).foregroundStyle(r.isDirty ? Theme.attentionInk : .secondary)
                        }
                    }
                }
            } else {
                ProgressView().controlSize(.small)
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}

/// Commits across all repos, the last 26 weeks: the heatmap, weekly bars and their peak.
private struct ActivityPanel: View {
    let repos: [RepoSummary]?

    var body: some View {
        let daily = (repos ?? []).reduce(into: Array(repeating: 0, count: 182)) { acc, r in
            for (i, v) in r.daily.suffix(182).enumerated() { acc[182 - min(182, r.daily.count) + i] += v }
        }
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    eyebrow("Activity")
                    Text("Commits across all repositories · last 26 weeks")
                }
                Spacer()
                Text("\(daily.reduce(0, +).formatted()) total").font(.caption.monospaced()).foregroundStyle(.secondary)
            }
            if repos != nil {
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .top, spacing: 24) { Heatmap(daily: daily); weekly(daily) }
                    VStack(alignment: .leading, spacing: 16) { ScrollView(.horizontal) { Heatmap(daily: daily) }; weekly(daily) }
                }
            } else {
                ProgressView().controlSize(.small)
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("overviewActivity")
    }

    private func weekly(_ daily: [Int]) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("Weekly commits")
                Spacer()
                Text("peak \(weeklyCommits(daily).max() ?? 0)/wk")
            }
            .font(.caption.monospaced()).foregroundStyle(.secondary)
            WeeklyBars(daily: daily, height: 118)
        }
        .frame(minWidth: 240)
    }
}

/// The five repos with the most commits in the last 30 days; one opens its detail.
private struct MostActive: View {
    let repos: [RepoSummary]?
    let open: (RepoSummary) -> Void

    var body: some View {
        let top = (repos ?? []).filter { $0.commits30 > 0 }.sorted { $0.commits30 > $1.commits30 }.prefix(5)
        let peak = max(1, top.first?.commits30 ?? 1)
        VStack(alignment: .leading, spacing: 10) {
            eyebrow("Most active · 30d")
            if repos != nil && top.isEmpty { Text("No commits in the last 30 days.").foregroundStyle(.secondary) }
            ForEach(Array(top.enumerated()), id: \.element.id) { i, r in
                Button { open(r) } label: {
                    HStack(spacing: 10) {
                        Text("\(i + 1)").font(.caption.monospaced()).foregroundStyle(.secondary).frame(width: 14, alignment: .leading)
                        Text(r.name).lineLimit(1)
                        Spacer()
                        ProgressView(value: Double(r.commits30) / Double(peak)).tint(Theme.accent).frame(width: 64)
                        Text("\(r.commits30)").font(.caption.monospaced()).frame(width: 30, alignment: .trailing)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("mostActive-\(r.name)")
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}
