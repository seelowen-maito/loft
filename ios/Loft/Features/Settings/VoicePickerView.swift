import AVFoundation
import SwiftUI

/// Chooses the voice that reads agent replies aloud. "Automatic" keeps picking the best installed
/// voice for each reply's language; a chosen voice is used only for replies in its own language.
struct VoicePickerView: View {
    @Environment(Speaker.self) private var speaker
    @AppStorage(Prefs.speechVoice) private var chosenId = ""
    @State private var previewing: String?

    private let installed = AVSpeechSynthesisVoice.speechVoices().map(VoiceChoice.init)
    /// Novelty voices clutter the list; one stays only while it's the current choice.
    private var voices: [VoiceChoice] { installed.filter { !$0.isNovelty || $0.identifier == chosenId } }
    private var sections: [(language: String, voices: [VoiceChoice])] {
        VoiceChoice.grouped(voices, firstLanguage: Locale.current.language.languageCode?.identifier ?? "en")
    }

    var body: some View {
        List {
            Section {
                Button { chosenId = "" } label: {
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Automatic").foregroundStyle(.primary)
                            Text("The best installed voice for each reply's language.")
                                .font(.footnote).foregroundStyle(.secondary)
                        }
                        Spacer(minLength: 8)
                        if chosenId.isEmpty { checkmark }
                    }
                    .frame(minHeight: 44)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(chosenId.isEmpty ? .isSelected : [])
                .accessibilityIdentifier("voiceAutomatic")
            } footer: {
                Text("A chosen voice reads replies in its own language. Replies in other languages use the automatic voice for that language. Download more voices in iOS Settings > Accessibility > Spoken Content > Voices.")
            }

            ForEach(sections, id: \.language) { section in
                Section(Locale.current.localizedString(forLanguageCode: section.language) ?? section.language) {
                    ForEach(section.voices, id: \.identifier) { v in row(v) }
                }
            }
        }
        .navigationTitle("Voice")
        .navigationBarTitleDisplayMode(.inline)
        .onDisappear { if previewing != nil { speaker.stop() } }
    }

    private var checkmark: some View {
        Image(systemName: "checkmark").font(.body.weight(.semibold)).foregroundStyle(Theme.accent)
    }

    private func row(_ v: VoiceChoice) -> some View {
        let selected = chosenId == v.identifier
        let playing = previewing == v.identifier && speaker.isSpeaking
        return HStack(spacing: 8) {
            Button { chosenId = v.identifier } label: {
                HStack(spacing: 8) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(v.name).foregroundStyle(.primary)
                        AdaptiveStack(spacing: 6) {
                            Text(regionLabel(v)).font(.footnote).foregroundStyle(.secondary)
                            QualityBadge(quality: v.quality)
                        }
                    }
                    Spacer(minLength: 8)
                    if selected { checkmark }
                }
                .frame(minHeight: 44)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("\(v.name), \(regionLabel(v)), \(v.quality.label) quality")
            .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
            .accessibilityIdentifier("voice-\(v.identifier)")

            Button {
                if playing {
                    speaker.stop()
                } else if let voice = AVSpeechSynthesisVoice(identifier: v.identifier) {
                    previewing = v.identifier
                    Task {
                        await speaker.preview(voice)
                        if previewing == v.identifier { previewing = nil }
                    }
                }
            } label: {
                Image(systemName: playing ? "stop.circle.fill" : "play.circle")
                    .font(.title3)
                    .foregroundStyle(Theme.accent)
                    .frame(minWidth: 44, minHeight: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.borderless)
            .accessibilityLabel(playing ? "Stop preview" : "Preview \(v.name)")
            // A fresh element when playback starts or stops, so VoiceOver picks up the new label.
            .id(playing)
        }
    }

    private func regionLabel(_ v: VoiceChoice) -> String {
        Locale.current.localizedString(forIdentifier: v.language) ?? v.language
    }
}

private struct QualityBadge: View {
    let quality: VoiceChoice.Quality

    var body: some View {
        Text(quality.label)
            .font(.caption2.weight(.semibold))
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .foregroundStyle(quality == .premium ? Theme.accent : .secondary)
            .background((quality == .premium ? Theme.accent : Color.secondary).opacity(0.15), in: Capsule())
            .fixedSize()
            .accessibilityHidden(true)
    }
}

extension VoiceChoice {
    /// What Settings shows for the saved choice; a voice that was uninstalled reads as Automatic.
    static func displayName(for id: String) -> String {
        guard !id.isEmpty, let v = AVSpeechSynthesisVoice(identifier: id) else { return "Automatic" }
        return v.name
    }
}
