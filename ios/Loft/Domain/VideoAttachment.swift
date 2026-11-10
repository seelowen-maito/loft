import Foundation

/// A video sent in the chat (spec #203): MP4 or MOV from Photos or the camera, up to 200 MB, uploaded to the
/// agent's `uploads/` and named in the message's `images`. The server makes frames of it for the agent.
enum VideoAttachment {
    /// The server's limit (`uploads::MAX_VIDEO_BYTES`).
    static let maxBytes = 200_000_000

    /// Why a video of `bytes` can't be sent, or nil when it can.
    static func refusal(bytes: Int) -> String? {
        bytes > maxBytes ? "This video is over 200 MB. Send a shorter one." : nil
    }

    /// Uploads the server plays as a video, by their name's extension.
    static func isVideo(_ file: String) -> Bool {
        ["mp4", "mov", "webm"].contains((file as NSString).pathExtension.lowercased())
    }

    /// `0:07`, `1:23`, `1:02:05`, as a video's length is shown.
    static func length(_ seconds: Double) -> String {
        let s = Int(seconds.rounded())
        return s >= 3600 ? String(format: "%d:%02d:%02d", s / 3600, s / 60 % 60, s % 60) : String(format: "%d:%02d", s / 60, s % 60)
    }
}

extension ChatMessage {
    /// The videos sent with the message (`meta.images`, which also lists images).
    var videos: [String] {
        guard case .array(let a)? = meta?["images"] else { return [] }
        return a.compactMap(\.stringValue).filter(VideoAttachment.isVideo)
    }
}
