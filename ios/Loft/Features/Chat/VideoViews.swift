import AVKit
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

/// A video in the composer: read from Photos or the camera, checked, then uploaded with progress. The message
/// can go once it's up (spec #203).
@MainActor @Observable
final class VideoDraft {
    enum State: Equatable {
        case preparing
        case uploading(Double)
        /// Up, by the server's name.
        case ready(String)
        case failed(String)
    }

    private(set) var state: State = .preparing
    private(set) var length: Double?
    private var work: Task<Void, Never>?

    var uploaded: String? { if case .ready(let f) = state { f } else { nil } }
    /// Still reading or uploading: the message waits for it.
    var busy: Bool {
        switch state {
        case .preparing, .uploading: true
        case .ready, .failed: false
        }
    }

    /// Reads the video (`load`: nil when nothing was picked), refuses one over 200 MB, then uploads it.
    func start(load: @escaping () async throws -> URL?,
               upload: @escaping (URL, @escaping @Sendable (Double) -> Void) async throws -> String) {
        work = Task {
            do {
                guard let file = try await load() else { return state = .failed("Couldn't read that video.") }
                let bytes = (try? file.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
                if let why = VideoAttachment.refusal(bytes: bytes) { return state = .failed(why) }
                length = try? await AVURLAsset(url: file).load(.duration).seconds
                state = .uploading(0)
                let name = try await upload(file) { p in
                    Task { @MainActor in
                        // A late report mustn't take a finished upload back to uploading.
                        guard case .uploading = self.state else { return }
                        self.state = .uploading(p)
                    }
                }
                try? FileManager.default.removeItem(at: file)
                state = .ready(name)
            } catch is CancellationError {
            } catch {
                state = .failed(APIError.from(error).title)
            }
        }
    }

    func cancel() { work?.cancel() }
}

/// A video Photos or the camera hands over, copied to a temporary file of ours.
struct PickedVideo: Transferable {
    let url: URL

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(contentType: .movie) { SentTransferredFile($0.url) } importing: { received in
            PickedVideo(url: try copy(received.file))
        }
    }

    static func copy(_ file: URL) throws -> URL {
        let to = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            .appendingPathExtension(file.pathExtension.isEmpty ? "mov" : file.pathExtension)
        try FileManager.default.copyItem(at: file, to: to)
        return to
    }
}

/// The composer's paperclip: choose a video from Photos (screen recordings too) or record one.
struct VideoAttachMenu: View {
    let picked: (@escaping () async throws -> URL?) -> Void
    @State private var choosing = false
    @State private var recording = false
    @State private var item: PhotosPickerItem?

    var body: some View {
        Menu {
            Button { choose() } label: { Label("Choose a video", systemImage: "photo.on.rectangle") }
                .accessibilityIdentifier("chooseVideo")
            if UIImagePickerController.isSourceTypeAvailable(.camera) {
                Button { recording = true } label: { Label("Record a video", systemImage: "video") }
            }
        } label: {
            Image(systemName: "paperclip").font(.system(size: 17, weight: .semibold)).frame(width: 30, height: 44)
        }
        .foregroundStyle(.secondary)
        .accessibilityLabel("Attach a video")
        .accessibilityIdentifier("attachVideoButton")
        .photosPicker(isPresented: $choosing, selection: $item, matching: .videos, preferredItemEncoding: .current)
        .onChange(of: item) { _, i in
            guard let i else { return }
            item = nil
            picked { try await i.loadTransferable(type: PickedVideo.self)?.url }
        }
        .fullScreenCover(isPresented: $recording) {
            CameraRecorder { url in
                recording = false
                if let url { picked { try PickedVideo.copy(url) } }
            }
            .ignoresSafeArea()
        }
    }

    private func choose() {
        #if DEBUG
        // `-fixture-video <path>`: as if that video was chosen in Photos (UI tests; the picker is another process).
        if let path = FixtureAPI.launchText("fixture-video") {
            return picked { try PickedVideo.copy(URL(fileURLWithPath: path)) }
        }
        #endif
        choosing = true
    }
}

/// The camera, recording one video.
private struct CameraRecorder: UIViewControllerRepresentable {
    let done: (URL?) -> Void

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let c = UIImagePickerController()
        c.sourceType = .camera
        c.mediaTypes = [UTType.movie.identifier]
        c.cameraCaptureMode = .video
        c.videoQuality = .typeHigh
        c.delegate = context.coordinator
        return c
    }

    func updateUIViewController(_ c: UIImagePickerController, context: Context) {}
    func makeCoordinator() -> Coordinator { Coordinator(done: done) }

    final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
        let done: (URL?) -> Void
        init(done: @escaping (URL?) -> Void) { self.done = done }
        func imagePickerController(_ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
            done(info[.mediaURL] as? URL)
        }
        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) { done(nil) }
    }
}

/// The video above the message field: its length, the upload's progress or why it can't go, and a remove button.
struct VideoChip: View {
    let draft: VideoDraft
    let remove: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "video.fill").foregroundStyle(Theme.accent).accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(.subheadline.weight(.medium))
                switch draft.state {
                case .preparing:
                    Text("Reading the video…").font(.caption).foregroundStyle(.secondary)
                case .uploading(let p):
                    ProgressView(value: p).accessibilityLabel("Uploading").accessibilityValue("\(Int(p * 100)) percent")
                case .ready:
                    Text("Ready to send").font(.caption).foregroundStyle(.secondary)
                case .failed(let why):
                    Text(why).font(.caption).foregroundStyle(Theme.failure)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Button(action: remove) {
                Image(systemName: "xmark.circle.fill").font(.title3).foregroundStyle(.secondary).frame(width: 44, height: 44)
            }
            .accessibilityLabel("Remove the video")
            .accessibilityIdentifier("removeVideo")
        }
        .padding(.leading, 12)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("videoChip")
    }

    private var title: String { draft.length.map { "Video · \(VideoAttachment.length($0))" } ?? "Video" }
}

/// A video sent in the chat: an inline player with its length under it.
struct ChatVideo: View {
    let url: URL
    @State private var player: AVPlayer?
    @State private var length: Double?
    /// Width over height, once known; 16:9 meanwhile.
    @State private var aspect: CGFloat = 16 / 9

    var body: some View {
        VStack(alignment: .trailing, spacing: 4) {
            VideoPlayer(player: player)
                .aspectRatio(aspect, contentMode: .fit)
                .frame(maxWidth: 260, maxHeight: 320)
                .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                .accessibilityLabel(length.map { "Video, \(VideoAttachment.length($0)) long" } ?? "Video")
                .accessibilityIdentifier("chatVideo")
            if let length {
                Text("Video · \(VideoAttachment.length(length))").font(.caption2).foregroundStyle(.secondary)
                    .accessibilityHidden(true)
            }
        }
        .task(id: url) {
            let asset = AVURLAsset(url: url)
            player = AVPlayer(playerItem: AVPlayerItem(asset: asset))
            length = try? await asset.load(.duration).seconds
            if let track = try? await asset.loadTracks(withMediaType: .video).first,
               let (size, t) = try? await track.load(.naturalSize, .preferredTransform) {
                let s = size.applying(t)
                if s.width != 0, s.height != 0 { aspect = abs(s.width / s.height) }
            }
        }
    }
}
