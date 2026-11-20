import Foundation

/// One section of the agents list: Leads, a group, or "No group". The iPhone and the Mac list agents the same way.
public struct AgentSection: Identifiable {
    public let id: String
    public let title: String
    public let agents: [Agent]

    /// Leads first, then one section per group (alphabetical), then "No group"; each keeps the order it's given.
    public static func sections(_ agents: [Agent]) -> [AgentSection] {
        let leads = agents.filter(\.isLead)
        let byGroup = Dictionary(grouping: agents.filter { !$0.isLead }, by: \.team)
        let groups = byGroup.keys.filter { !$0.isEmpty }.sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }
        var out: [AgentSection] = []
        if !leads.isEmpty { out.append(AgentSection(id: "leads", title: "Leads", agents: leads)) }
        out += groups.map { AgentSection(id: "group:" + $0, title: $0, agents: byGroup[$0]!) }
        if let none = byGroup[""] { out.append(AgentSection(id: "none", title: "No group", agents: none)) }
        return out
    }
}
