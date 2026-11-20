import Foundation
import ImageIO
import UniformTypeIdentifiers

/// A video sent in the chat (spec #203): MP4 or MOV from Photos or the camera, up to 200 MB, uploaded to the
/// agent's `uploads/` and named in the message's `images`. The server makes frames of it for the agent.
public enum VideoAttachment {
    /// The server's limit (`uploads::MAX_VIDEO_BYTES`).
    public static let maxBytes = 200_000_000

    /// Why a video of `bytes` can't be sent, or nil when it can.
    public static func refusal(bytes: Int) -> String? {
        bytes > maxBytes ? "This video is over 200 MB. Send a shorter one." : nil
    }

    /// Uploads the server plays as a video, by their name's extension.
    public static func isVideo(_ file: String) -> Bool {
        ["mp4", "mov", "webm"].contains((file as NSString).pathExtension.lowercased())
    }

    /// `0:07`, `1:23`, `1:02:05`, as a video's length is shown.
    public static func length(_ seconds: Double) -> String {
        let s = Int(seconds.rounded())
        return s >= 3600 ? String(format: "%d:%02d:%02d", s / 3600, s / 60 % 60, s % 60) : String(format: "%d:%02d", s / 60, s % 60)
    }
}

extension ChatMessage {
    /// The videos sent with the message (`meta.images`, which also lists images).
    public var videos: [String] {
        guard case .array(let a)? = meta?["images"] else { return [] }
        return a.compactMap(\.stringValue).filter(VideoAttachment.isVideo)
    }
}

/// A photo sent in the chat (#309): from Photos or the camera, made a JPEG small enough for the server, uploaded to
/// the agent's `uploads/` and named in the message's `images`, as the web sends images.
public enum PhotoAttachment {
    /// The server's limit per image (`uploads::MAX_BYTES`).
    public static let maxBytes = 3_750_000
    /// The server's limit for a message's images together (`uploads::MAX_MESSAGE_BYTES`).
    public static let maxMessageBytes = 20_000_000
    /// The server's limit of images per message, a video counting as one.
    public static let maxPerMessage = 10
    /// What the server counts a video as toward `maxMessageBytes`: its frames (`uploads::VIDEO_ALLOWANCE`).
    public static let videoAllowance = 6_000_000

    /// Why photos of these sizes, and a video if there is one, can't go in one message, or nil when they can.
    public static func refusal(bytes: [Int], video: Bool) -> String? {
        guard bytes.reduce(0, +) + (video ? videoAllowance : 0) > maxMessageBytes else { return nil }
        return video ? "These photos and the video add up to over 20 MB. Send them in a few messages."
                     : "These photos add up to over 20 MB. Send them in a few messages."
    }

    /// `image` (HEIC, PNG, JPEG…) as an upright JPEG under `maxBytes`: at most 4096 px on its long side, smaller
    /// until it fits. Nil when it isn't an image.
    public static func jpeg(_ image: Data, maxBytes: Int = maxBytes) -> Data? {
        guard let source = CGImageSourceCreateWithData(image as CFData, nil) else { return nil }
        var side = 4096
        while side >= 256 {
            guard let scaled = thumbnail(source, side: side) else { return nil }
            let out = NSMutableData()
            guard let dest = CGImageDestinationCreateWithData(out, UTType.jpeg.identifier as CFString, 1, nil) else { return nil }
            CGImageDestinationAddImage(dest, scaled, [kCGImageDestinationLossyCompressionQuality: 0.8] as CFDictionary)
            guard CGImageDestinationFinalize(dest) else { return nil }
            if out.length <= maxBytes { return out as Data }
            side = side * 3 / 4
        }
        return nil
    }

    /// The image at `url`, upright, at most `side` px on its long side.
    public static func thumbnail(_ url: URL, side: Int) -> CGImage? {
        CGImageSourceCreateWithURL(url as CFURL, nil).flatMap { thumbnail($0, side: side) }
    }

    private static func thumbnail(_ source: CGImageSource, side: Int) -> CGImage? {
        CGImageSourceCreateThumbnailAtIndex(source, 0, [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: side,
        ] as CFDictionary)
    }
}

extension ChatMessage {
    /// The images sent with the message that aren't videos.
    public var photos: [String] {
        guard case .array(let a)? = meta?["images"] else { return [] }
        return a.compactMap(\.stringValue).filter { !VideoAttachment.isVideo($0) }
    }
}
