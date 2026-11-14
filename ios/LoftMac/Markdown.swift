import AppKit
import LoftKit
import SwiftUI

/// A message's Markdown drawn as the web's `.prose-chat` (frontend/src/index.css): 14pt text with relaxed lines,
/// headings, lists, mono code blocks with a copy button, quotes, tables and links.
struct MarkdownView: View {
    let blocks: [MarkdownBlock]

    init(_ text: String) { blocks = MarkdownBlock.parse(text) }
    init(blocks: [MarkdownBlock]) { self.blocks = blocks }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(blocks.indices, id: \.self) { i in
                MarkdownBlockView(block: blocks[i])
            }
        }
        .font(.system(size: 14))
        .lineSpacing(4)
        .tint(Theme.accent)
        .textSelection(.enabled)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Inline Markdown: bold, italics, links underlined in the accent colour, `code` on a faint background.
    static func inline(_ s: String) -> AttributedString {
        var a = (try? AttributedString(markdown: s, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(s)
        for run in a.runs {
            if run.inlinePresentationIntent?.contains(.code) == true {
                a[run.range].font = .system(size: 12.5, design: .monospaced)
                a[run.range].backgroundColor = WebTheme.fg.opacity(0.08)
            }
            if run.link != nil { a[run.range].underlineStyle = .single }
        }
        return a
    }
}

private struct MarkdownBlockView: View {
    let block: MarkdownBlock

    var body: some View {
        switch block {
        case .heading(let level, let text):
            Text(MarkdownView.inline(text))
                .font(.system(size: level == 1 ? 17 : level == 2 ? 15.5 : 14, weight: .semibold))
                .padding(.top, 4)
                .accessibilityAddTraits(.isHeader)
        case .paragraph(let text):
            Text(MarkdownView.inline(text))
        case .list(_, let items):
            VStack(alignment: .leading, spacing: 4) {
                ForEach(items.indices, id: \.self) { i in
                    let item = items[i]
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Group {
                            if let checked = item.checked {
                                Image(systemName: checked ? "checkmark.square" : "square")
                            } else {
                                Text(item.marker).monospacedDigit()
                            }
                        }
                        .foregroundStyle(WebTheme.fg2)
                        .frame(minWidth: 16, alignment: .trailing)
                        Text(MarkdownView.inline(item.text))
                    }
                    .padding(.leading, CGFloat(item.depth) * 20)
                }
            }
        case .code(let language, let text):
            CodeBlock(language: language, text: text)
        case .quote(let blocks):
            HStack(spacing: 10) {
                Rectangle().fill(WebTheme.lineStrong).frame(width: 2)
                MarkdownView(blocks: blocks).foregroundStyle(WebTheme.fg2)
            }
            .fixedSize(horizontal: false, vertical: true)
        case .table(let header, let rows):
            Grid(alignment: .leading, horizontalSpacing: 0, verticalSpacing: 0) {
                GridRow { ForEach(header.indices, id: \.self) { cell(header[$0]).fontWeight(.semibold) } }
                ForEach(rows.indices, id: \.self) { r in
                    GridRow { ForEach(header.indices, id: \.self) { c in cell(rows[r].indices.contains(c) ? rows[r][c] : "") } }
                }
            }
            .font(.system(size: 13))
        case .rule:
            Rectangle().fill(WebTheme.line).frame(height: 1).padding(.vertical, 4)
        }
    }

    private func cell(_ text: String) -> some View {
        Text(MarkdownView.inline(text))
            .padding(.horizontal, 8).padding(.vertical, 5)
            .frame(maxWidth: .infinity, alignment: .leading)
            .overlay(alignment: .bottom) { Rectangle().fill(WebTheme.line).frame(height: 1) }
    }
}

/// A fenced code block: mono, scrolling sideways when a line is long, with a copy button.
private struct CodeBlock: View {
    let language: String
    let text: String
    @State private var copied = false

    var body: some View {
        ScrollView(.horizontal) {
            Text(text)
                .font(.system(size: 12.5, design: .monospaced))
                .lineSpacing(2)
                .fixedSize()
                .padding(12)
                .padding(.trailing, 28)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(WebTheme.canvas, in: RoundedRectangle(cornerRadius: WebTheme.rowRadius, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: WebTheme.rowRadius, style: .continuous).strokeBorder(WebTheme.line))
        .overlay(alignment: .topTrailing) {
            Button {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(text, forType: .string)
                copied = true
                Task { try? await Task.sleep(for: .seconds(1.5)); copied = false }
            } label: {
                Image(systemName: copied ? "checkmark" : "doc.on.doc").font(.system(size: 11))
                    .foregroundStyle(copied ? Theme.accent : WebTheme.fg3)
                    .frame(width: 24, height: 24)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(4)
            .help(copied ? "Copied" : "Copy")
            .accessibilityLabel(language.isEmpty ? "Copy code" : "Copy \(language) code")
            .accessibilityIdentifier("copyCode")
        }
    }
}
