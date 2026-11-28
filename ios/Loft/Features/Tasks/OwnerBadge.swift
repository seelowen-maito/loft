import LoftKit
import SwiftUI

/// An issue's Owner: their GitHub picture beside "@login". While it loads, or when it can't, a person icon instead.
struct OwnerBadge: View {
    let login: String
    @ScaledMetric private var size: CGFloat
    @Environment(\.displayScale) private var scale

    init(login: String, size: CGFloat = 16, relativeTo style: Font.TextStyle = .caption) {
        self.login = login
        _size = ScaledMetric(wrappedValue: size, relativeTo: style)
    }

    var body: some View {
        if let name = GitHubAvatar.login(login) {
            HStack(spacing: 4) {
                AsyncImage(url: GitHubAvatar.url(login: name, pixels: Int((size * scale).rounded(.up)))) { phase in
                    if let image = phase.image {
                        image.resizable().scaledToFill()
                    } else {
                        Image(systemName: "person.crop.circle.fill").resizable().scaledToFit().foregroundStyle(.secondary)
                    }
                }
                .frame(width: size, height: size)
                .clipShape(Circle())
                .accessibilityHidden(true)
                Text("@\(name)").lineLimit(1)
            }
        }
    }
}
