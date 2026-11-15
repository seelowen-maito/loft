import Foundation

extension LoftStore {
    /// A message that came by voice (dictated, or from Talk): sends it, then waits for the agent's whole reply to
    /// settle and hands it to `read`, as the iPhone's chat does. Nothing is read when the send failed (the chat
    /// shows Retry) or when `reads` is off. `keepWaiting` ends the wait, as on the iPhone when the chat is left.
    public func sendByVoice(_ text: String, to agentId: String, images: [String] = [], reads: Bool,
                            keepWaiting: () -> Bool = { true },
                            read: @MainActor (String, Int64?) async -> Void) async {
        let before = conversation(agentId).last?.id ?? 0
        await send(text, to: agentId, images: images)
        guard reads, !pending(for: agentId).contains(where: \.isFailed),
              let answer = await waitForReply(agentId: agentId, after: before, while: keepWaiting) else { return }
        let id = conversation(agentId).last { $0.role == "assistant" && $0.kind == "text" }?.id
        await read(answer, id)
    }
}
