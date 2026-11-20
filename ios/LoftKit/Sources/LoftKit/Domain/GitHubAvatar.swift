import Foundation

/// A GitHub user's profile picture; the same URL as the web's `githubAvatar.ts`.
public enum GitHubAvatar {
    /// A login as typed: trimmed, without a leading `@`; nil when nothing's left.
    public static func login(_ raw: String) -> String? {
        var name = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if name.hasPrefix("@") { name.removeFirst() }
        return name.isEmpty ? nil : name
    }

    /// `https://github.com/<login>.png`, asked at `pixels` wide; nil with no login.
    public static func url(login raw: String, pixels: Int) -> URL? {
        guard let name = login(raw),
              let path = name.addingPercentEncoding(withAllowedCharacters: loginCharacters) else { return nil }
        return URL(string: "https://github.com/\(path).png?size=\(max(pixels, 1))")
    }

    private static let loginCharacters = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-_."))
}
