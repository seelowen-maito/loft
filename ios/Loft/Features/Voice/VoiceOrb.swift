import SwiftUI

/// The listening orb, driven by the mic level (0...1). With Reduce Motion it becomes a
/// static ring that only scales with the level.
struct VoiceOrb: View {
    var level: Float
    var active: Bool
    var tint: Color = Theme.accent
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var l: CGFloat { active ? CGFloat(min(max(level, 0), 1)) : 0 }

    var body: some View {
        Group {
            // The moving orb redraws every frame, so under UI tests (`-no-animations`) the app would never go idle
            // and every tap would wait out XCTest's idle timeout: the still version instead.
            if reduceMotion || TestMode.noAnimations {
                ZStack {
                    Circle().stroke(tint.opacity(0.35), lineWidth: 3).scaleEffect(1 + l * 0.3)
                    Circle().fill(tint.gradient).scaleEffect(0.62)
                }
            } else {
                TimelineView(.animation(paused: !active)) { t in
                    let phase = t.date.timeIntervalSinceReferenceDate
                    ZStack {
                        ForEach(0..<3) { i in
                            Circle()
                                .fill(tint.opacity(0.22 - Double(i) * 0.05))
                                .scaleEffect(0.72 + l * (0.25 + CGFloat(i) * 0.12))
                                .offset(x: cos(phase * (0.9 + Double(i) * 0.4)) * 6 * l,
                                        y: sin(phase * (1.1 + Double(i) * 0.3)) * 6 * l)
                                .blur(radius: 6 + CGFloat(i) * 4)
                        }
                        Circle()
                            .fill(AngularGradient(colors: [tint, tint.opacity(0.7), .white.opacity(0.85), tint],
                                                  center: .center, angle: .radians(phase * 0.8)))
                            .scaleEffect(0.62 + l * 0.08)
                            .shadow(color: tint.opacity(0.6), radius: 18 * (0.4 + l))
                    }
                }
            }
        }
        .animation(.easeOut(duration: 0.12), value: level)
        .aspectRatio(1, contentMode: .fit)
        .accessibilityHidden(true)
    }
}
