import AVKit
import LoftKit
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

/// A video (spec #203) or photo (#309) in the composer: read from Photos or the camera, checked, then uploaded with
/// progress. The message can go once it's up.
@MainActor @Observable
final class AttachmentDraft: Identifiable {
    enum State: Equatable {
        case preparing
        case uploading(Double)
        /// Up, by the server's name.
        case ready(String)
        case failed(String)
    }

    let isVideo: Bool
    private(set) var state: State = .preparing
    private(set) var length: Double?
    /// A photo's size as it goes up, and a small copy for the draft.
    private(set) var bytes = 0
    private(set) var preview: UIImage?
    private var work: Task<Void, Never>?

    init(video: Bool) { isVideo = video }

    var uploaded: String? { if case .ready(let f) = state { f } else { nil } }
    /// Still reading or uploading: the message waits for it.
    var busy: Bool {
        switch state {
        case .preparing, .uploading: true
        case .ready, .failed: false
        }
    }

    /// Reads the file (`load`: nil when it couldn't be read), refuses a video over 200 MB, then uploads it.
    func start(load: @escaping () async throws -> URL?,
               upload: @escaping (URL, @escaping @Sendable (Double) -> Void) async throws -> String) {
        work = Task {
            do {
                guard let file = try await load() else { return state = .failed(isVideo ? "Couldn't read that video." : "Couldn't read that photo.") }
                bytes = (try? file.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
                if isVideo {
                    if let why = VideoAttachment.refusal(bytes: bytes) { return state = .failed(why) }
                    length = try? await AVURLAsset(url: file).load(.duration).seconds
                } else {
                    preview = await Task.detached { PhotoAttachment.thumbnail(file, side: 160).map(UIImage.init) }.value
                }
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

/// A photo Photos or the camera hands over, made a JPEG the server takes, in a temporary file of ours. One at a time:
/// decoding ten full-size photos at once would spike memory.
actor PickedPhoto {
    private static let shared = PickedPhoto()

    static func save(_ image: Data) async -> URL? { await shared.save(image) }

    private func save(_ image: Data) -> URL? {
        guard let jpeg = PhotoAttachment.jpeg(image) else { return nil }
        let to = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).appendingPathExtension("jpg")
        return (try? jpeg.write(to: to)) == nil ? nil : to
    }
}

/// The composer's paperclip: choose photos or a video from Photos (screen recordings too), or take one.
struct AttachMenu: View {
    /// How many more photos the message can take.
    let photosLeft: Int
    let pickedPhotos: ([() async throws -> URL?]) -> Void
    let pickedVideo: (@escaping () async throws -> URL?) -> Void
    @State private var choosingVideo = false
    @State private var choosingPhotos = false
    @State private var camera: Camera.Mode?
    @State private var item: PhotosPickerItem?
    @State private var photoItems: [PhotosPickerItem] = []

    var body: some View {
        Menu {
            Button { choosePhotos() } label: { Label("Choose photos", systemImage: "photo.on.rectangle") }
                .disabled(photosLeft == 0)
                .accessibilityIdentifier("choosePhotos")
            if UIImagePickerController.isSourceTypeAvailable(.camera) {
                Button { camera = .photo } label: { Label("Take a photo", systemImage: "camera") }
                    .disabled(photosLeft == 0)
            }
            Button { chooseVideo() } label: { Label("Choose a video", systemImage: "film") }
                .accessibilityIdentifier("chooseVideo")
            if UIImagePickerController.isSourceTypeAvailable(.camera) {
                Button { camera = .video } label: { Label("Record a video", systemImage: "video") }
            }
        } label: {
            Image(systemName: "paperclip").font(.system(size: 17, weight: .semibold)).frame(width: 30, height: 44)
        }
        .foregroundStyle(.secondary)
        .accessibilityLabel("Attach a photo or video")
        .accessibilityIdentifier("attachVideoButton")
        .photosPicker(isPresented: $choosingVideo, selection: $item, matching: .videos, preferredItemEncoding: .current)
        .photosPicker(isPresented: $choosingPhotos, selection: $photoItems, maxSelectionCount: max(photosLeft, 1),
                      selectionBehavior: .ordered, matching: .images)
        .onChange(of: item) { _, i in
            guard let i else { return }
            item = nil
            pickedVideo { try await i.loadTransferable(type: PickedVideo.self)?.url }
        }
        .onChange(of: photoItems) { _, items in
            guard !items.isEmpty else { return }
            photoItems = []
            pickedPhotos(items.map { i in {
                guard let d = try await i.loadTransferable(type: Data.self) else { return nil }
                return await PickedPhoto.save(d)
            } })
        }
        .fullScreenCover(item: $camera) { mode in
            Camera(mode: mode) { info in
                camera = nil
                if mode == .video, let url = info?[.mediaURL] as? URL {
                    pickedVideo { try PickedVideo.copy(url) }
                } else if mode == .photo, let photo = (info?[.originalImage] as? UIImage)?.jpegData(compressionQuality: 1) {
                    pickedPhotos([{ await PickedPhoto.save(photo) }])
                }
            }
            .ignoresSafeArea()
        }
    }

    private func choosePhotos() {
        #if DEBUG
        // `-fixture-photos <path>,<path>`: as if those photos were chosen in Photos (UI tests; the picker is another process).
        if let paths = FixtureAPI.launchText("fixture-photos") {
            return pickedPhotos(paths.split(separator: ",").map { p in { await PickedPhoto.save(try Data(contentsOf: URL(fileURLWithPath: String(p)))) } })
        }
        #endif
        choosingPhotos = true
    }

    private func chooseVideo() {
        #if DEBUG
        // `-fixture-video <path>`: as if that video was chosen in Photos (UI tests; the picker is another process).
        if let path = FixtureAPI.launchText("fixture-video") {
            return pickedVideo { try PickedVideo.copy(URL(fileURLWithPath: path)) }
        }
        #endif
        choosingVideo = true
    }
}

/// The camera, taking one photo or recording one video.
private struct Camera: UIViewControllerRepresentable {
    enum Mode: Identifiable { case photo, video; var id: Self { self } }
    let mode: Mode
    let done: ([UIImagePickerController.InfoKey: Any]?) -> Void

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let c = UIImagePickerController()
        c.sourceType = .camera
        c.mediaTypes = [mode == .photo ? UTType.image.identifier : UTType.movie.identifier]
        c.cameraCaptureMode = mode == .photo ? .photo : .video
        if mode == .video { c.videoQuality = .typeHigh }
        c.delegate = context.coordinator
        return c
    }

    func updateUIViewController(_ c: UIImagePickerController, context: Context) {}
    func makeCoordinator() -> Coordinator { Coordinator(done: done) }

    final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
        let done: ([UIImagePickerController.InfoKey: Any]?) -> Void
        init(done: @escaping ([UIImagePickerController.InfoKey: Any]?) -> Void) { self.done = done }
        func imagePickerController(_ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
            done(info)
        }
        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) { done(nil) }
    }
}

/// The video above the message field: its length, the upload's progress or why it can't go, and a remove button.
struct VideoChip: View {
    let draft: AttachmentDraft
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

/// A photo above the message field: a thumbnail with the upload's progress, or why it can't go, and a remove button.
struct PhotoChip: View {
    let draft: AttachmentDraft
    let remove: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            Group {
                if let p = draft.preview {
                    Image(uiImage: p).resizable().scaledToFill()
                } else {
                    Theme.card
                }
            }
            .frame(width: 64, height: 64)
            .overlay {
                switch draft.state {
                case .preparing:
                    ProgressView()
                case .uploading(let p):
                    ProgressView(value: p).tint(.white).padding(8).frame(maxHeight: .infinity, alignment: .bottom)
                        .accessibilityLabel("Uploading").accessibilityValue("\(Int(p * 100)) percent")
                case .ready:
                    EmptyView()
                case .failed:
                    Image(systemName: "exclamationmark.triangle.fill").font(.title3).foregroundStyle(Theme.failure)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
            // Under the photo, not on it, and capped so the glyph fits its 44-pt target at every text size (#316).
            Button(action: remove) {
                Image(systemName: "xmark.circle.fill").font(.title3).foregroundStyle(.secondary)
                    .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
                    .frame(width: 44, height: 44)
            }
            .accessibilityLabel("Remove the photo")
            .accessibilityIdentifier("removePhoto")
        }
        .frame(width: 64)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(label)
        .accessibilityIdentifier("photoChip")
    }

    private var label: String {
        switch draft.state {
        case .preparing: "Photo, reading"
        case .uploading: "Photo, uploading"
        case .ready: "Photo, ready to send"
        case .failed(let why): "Photo, \(why)"
        }
    }
}

/// A photo sent in the chat; tapping it opens it full screen.
struct ChatPhoto: View {
    let url: URL
    @State private var full = false

    var body: some View {
        Button { full = true } label: {
            AsyncImage(url: url) { image in
                image.resizable().scaledToFill()
            } placeholder: {
                Theme.card.overlay(ProgressView())
            }
            .frame(width: 120, height: 120)
            .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Photo")
        .accessibilityHint("Opens it full screen")
        .accessibilityIdentifier("chatPhoto")
        .fullScreenCover(isPresented: $full) { FullScreenPhoto(url: url) }
    }
}

/// A sent photo on black, fitted to the screen, with Done.
private struct FullScreenPhoto: View {
    let url: URL
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            AsyncImage(url: url) { image in
                image.resizable().scaledToFit()
            } placeholder: {
                ProgressView().tint(.white)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(.black)
            .accessibilityLabel("Photo")
            .accessibilityIdentifier("fullScreenPhoto")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
            .toolbarBackground(.black, for: .navigationBar)
            .toolbarColorScheme(.dark, for: .navigationBar)
        }
    }
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
