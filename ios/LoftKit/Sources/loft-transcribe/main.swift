// `loft-transcribe <audio.pcm>`: the text of a recording, transcribed on this Mac with Apple's SpeechAnalyzer
// (LoftKit's recognizer), never over the network. The backend runs it for `POST /api/voice/transcribe`.
// The file is raw 16 kHz mono 16-bit little-endian PCM. Prints the text and exits 0; when it can't run here it
// prints a reason the web can show on stderr and exits 2; any other failure exits 1.
import AVFoundation
import Foundation
import LoftKit
import Speech

func fail(_ message: String, code: Int32) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(code)
}

let args = CommandLine.arguments
guard args.count == 2, let data = FileManager.default.contents(atPath: args[1]) else {
    fail("usage: loft-transcribe <audio.pcm> (16 kHz mono 16-bit PCM)", code: 1)
}

@available(macOS 26.0, *)
func transcribe(_ data: Data) async -> String {
    let format = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: 16_000, channels: 1, interleaved: true)!
    let frames = AVAudioFrameCount(data.count / 2)
    let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: max(frames, 1))!
    buffer.frameLength = frames
    data.withUnsafeBytes { raw in
        if frames > 0 { memcpy(buffer.int16ChannelData![0], raw.baseAddress!, Int(frames) * 2) }
    }

    let recognizer = AnalyzerSegmentRecognizer(locale: Locale.current)
    var segments: [String] = []
    var partial = ""
    var failure: String?
    recognizer.onCommit = { segments.append($0) }
    recognizer.onPartial = { partial = $0 }
    recognizer.onError = { failure = $0 }
    do {
        try await recognizer.prepare()
        try recognizer.start()
    } catch {
        fail(error.localizedDescription, code: 2)
    }
    if frames > 0 { recognizer.append(buffer) }
    await recognizer.flush()
    if let failure { fail(failure, code: 1) }
    return VoiceText.join(segments + [partial])
}

if data.isEmpty {
    // No audio: nothing to hear, and the analyzer's flush never returns on none.
    print("")
} else if #available(macOS 26.0, *), Speech.SpeechTranscriber.isAvailable {
    print(await transcribe(data))
} else {
    fail("Transcribing needs macOS 26 or later with Apple's speech model.", code: 2)
}
