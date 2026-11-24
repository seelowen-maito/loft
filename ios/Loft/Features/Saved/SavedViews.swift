import LoftKit
import SwiftUI

/// Saved (spec #675): chat messages kept for good, with a title and a tag, as on the web's Saved page.
struct SavedRoute: Hashable {}
struct SavedItemRoute: Hashable { let id: Int64 }

/// Every saved item, newest first, with search and a tag filter.
struct SavedListView: View {
    @Environment(LoftStore.self) private var store
    @State private var query = ""
    @State private var tag = ""
    @State private var list: SavedList?
    @State private var error: APIError?

    var body: some View {
        List {
            Section {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach([""] + Saved.tagOptions(list?.tags ?? []), id: \.self) { t in
                            SavedTagButton(label: t.isEmpty ? "All" : t, selected: tag == t) { tag = t }
                        }
                    }
                }
                .accessibilityIdentifier("savedTagFilter")
                .listRowInsets(EdgeInsets(top: 4, leading: 16, bottom: 4, trailing: 16))
                .listRowBackground(Color.clear)
            }
            if let error {
                Label(error.title, systemImage: "exclamationmark.triangle").foregroundStyle(Theme.failure)
            }
            if let list, list.items.isEmpty {
                Text(query.isEmpty && tag.isEmpty ? "Nothing saved yet. Save a message from a chat: touch and hold it, then Save." : "Nothing saved matches.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity)
                    .listRowBackground(Color.clear)
            }
            ForEach(list?.items ?? []) { i in
                NavigationLink(value: SavedItemRoute(id: i.id)) { SavedRow(item: i) }
                    .accessibilityIdentifier("savedRow-\(i.title)")
            }
        }
        .navigationTitle("Saved")
        .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: "Search titles and text")
        // Typing searches once it pauses; it reloads when you come back from an item.
        .task(id: "\(query)\n\(tag)") {
            try? await Task.sleep(for: .milliseconds(200))
            guard !Task.isCancelled else { return }
            await load()
        }
        .refreshable { await load() }
    }

    private func load() async {
        do {
            let l = try await store.savedList(query: query.trimmingCharacters(in: .whitespaces), tag: tag)
            list = l
            error = nil
            // Its last item deleted, the tag's chip is gone: show All (this reloads).
            tag = Saved.keptFilter(tag, tags: l.tags)
        } catch {
            self.error = APIError.from(error)
        }
    }
}

private struct SavedRow: View {
    let item: SavedItem

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                Text(item.title).font(.body.weight(.medium)).lineLimit(2)
                Spacer(minLength: 8)
                if !item.tag.isEmpty { SavedTagLabel(text: item.tag) }
            }
            Text(Saved.excerpt(item.content)).font(.subheadline).foregroundStyle(.secondary).lineLimit(2)
            Text("\(item.agentName) · \(item.createdAt.formatted(date: .abbreviated, time: .omitted))")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
    }
}

/// One saved item as a readable page, with Edit and Delete.
struct SavedItemView: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let id: Int64
    @State private var item: SavedItem?
    @State private var missing = false
    @State private var editing = false
    @State private var confirmDelete = false
    @State private var error: APIError?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if missing {
                    Text("This saved item doesn't exist.").foregroundStyle(.secondary)
                } else if let item {
                    Text(item.title).font(.title2.weight(.semibold)).accessibilityAddTraits(.isHeader)
                    meta(item)
                    SavedMarkdown(blocks: MarkdownBlock.parse(item.content))
                        .padding(14)
                        .background(Theme.card, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                        .accessibilityIdentifier("savedContent")
                } else {
                    ProgressView().frame(maxWidth: .infinity)
                }
            }
            .padding()
        }
        .background(Color(uiColor: .systemGroupedBackground))
        .navigationTitle("Saved")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if item != nil {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Edit") { editing = true }.accessibilityIdentifier("editSaved")
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(role: .destructive) { confirmDelete = true } label: { Label("Delete", systemImage: "trash") }
                        .accessibilityIdentifier("deleteSaved")
                        .confirmationDialog("Delete this saved item?", isPresented: $confirmDelete, titleVisibility: .visible) {
                            Button("Delete", role: .destructive) { delete() }
                            Button("Keep it", role: .cancel) {}
                        } message: {
                            Text("It's removed from Saved. The message in the chat stays. This can't be undone.")
                        }
                }
            }
        }
        .sheet(isPresented: $editing) {
            if let item {
                NavigationStack {
                    SavedEditView(item: item) { edited in
                        self.item?.title = edited.title
                        self.item?.tag = edited.tag
                        self.item?.content = edited.content
                        self.item?.updatedAt = edited.updatedAt
                    }
                }
            }
        }
        .task { await load() }
        .errorAlert($error)
    }

    @ViewBuilder private func meta(_ item: SavedItem) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                if !item.tag.isEmpty { SavedTagLabel(text: item.tag) }
                Text("From \(item.agentName) · \(item.createdAt.formatted(date: .abbreviated, time: .omitted))")
            }
            if let edited = item.updatedAt {
                Text("Edited \(edited.formatted(date: .abbreviated, time: .shortened))")
            }
            if item.sourceExists == true, store.agent(item.agentId) != nil {
                NavigationLink(value: ChatRoute(agentId: item.agentId)) {
                    Label("Open in chat", systemImage: "bubble.left")
                }
                .accessibilityIdentifier("savedOpenChat")
            } else if item.sourceExists == false {
                Text("The source message is gone")
            }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }

    private func load() async {
        do {
            item = try await store.savedItem(id: id)
        } catch {
            let e = APIError.from(error)
            if case .server(404, _) = e { missing = true } else { self.error = e }
        }
    }

    private func delete() {
        Task {
            do {
                try await store.deleteSaved(id: id)
                dismiss()
            } catch {
                self.error = APIError.from(error)
            }
        }
    }
}

/// Edit: the title, the tag and the content as Markdown, with a preview.
struct SavedEditView: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let item: SavedItem
    var saved: (SavedItem) -> Void
    @State private var title: String
    @State private var tag: String
    @State private var content: String
    @State private var preview = false
    @State private var saving = false
    @State private var error: APIError?

    init(item: SavedItem, saved: @escaping (SavedItem) -> Void) {
        self.item = item
        self.saved = saved
        _title = State(initialValue: item.title)
        _tag = State(initialValue: item.tag)
        _content = State(initialValue: item.content)
    }

    private var valid: Bool { !title.trimmingCharacters(in: .whitespaces).isEmpty && !content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    var body: some View {
        Form {
            Section("Title") {
                TextField("Title", text: $title, axis: .vertical).accessibilityIdentifier("savedTitleField")
            }
            Section("Tag") { SavedTagPicker(tag: $tag) }
            Section {
                Picker("Content view", selection: $preview) {
                    Text("Write").tag(false)
                    Text("Preview").tag(true)
                }
                .pickerStyle(.segmented)
                if preview {
                    SavedMarkdown(blocks: MarkdownBlock.parse(content))
                } else {
                    TextField("Content", text: $content, axis: .vertical)
                        .font(.system(.callout, design: .monospaced))
                        .lineLimit(8...)
                        .accessibilityIdentifier("savedContentField")
                }
            } header: {
                Text("Content, in Markdown")
            }
        }
        .navigationTitle("Edit")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            ToolbarItem(placement: .confirmationAction) {
                Button(saving ? "Saving…" : "Save") { save() }.disabled(!valid || saving).accessibilityIdentifier("saveEdit")
            }
        }
        .errorAlert($error)
    }

    private func save() {
        saving = true
        Task {
            defer { saving = false }
            do {
                let i = try await store.updateSaved(id: item.id, title: title.trimmingCharacters(in: .whitespaces),
                                                    tag: tag.trimmingCharacters(in: .whitespaces), content: content)
                saved(i)
                dismiss()
            } catch {
                self.error = APIError.from(error)
            }
        }
    }
}

/// Save in a chat message's menu: a small form for the title and tag, then the message is kept in Saved.
struct SaveMessageSheet: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let message: ChatMessage
    /// Called with the new item once it's saved, for the chat's confirmation.
    var saved: (SavedItem) -> Void = { _ in }
    @State private var title: String
    @State private var tag = Saved.suggestedTags[0]
    @State private var saving = false
    @State private var error: APIError?

    init(message: ChatMessage, saved: @escaping (SavedItem) -> Void = { _ in }) {
        self.message = message
        self.saved = saved
        _title = State(initialValue: Saved.defaultTitle(message.displayContent))
    }

    var body: some View {
        NavigationStack {
            Form {
                Section("Title") {
                    TextField("Title", text: $title, axis: .vertical).accessibilityIdentifier("saveTitleField")
                }
                Section("Tag") { SavedTagPicker(tag: $tag) }
            }
            .navigationTitle("Save message")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button(saving ? "Saving…" : "Save") { save() }
                        .disabled(title.trimmingCharacters(in: .whitespaces).isEmpty || saving)
                        .accessibilityIdentifier("saveMessageConfirm")
                }
            }
            .errorAlert($error)
        }
        .presentationDetents([.medium, .large])
    }

    private func save() {
        saving = true
        Task {
            defer { saving = false }
            do {
                saved(try await store.saveMessage(message, title: title.trimmingCharacters(in: .whitespaces), tag: tag.trimmingCharacters(in: .whitespaces)))
                dismiss()
            } catch {
                self.error = APIError.from(error)
            }
        }
    }
}

/// The suggested tags as buttons, and a field for any other.
private struct SavedTagPicker: View {
    @Binding var tag: String

    var body: some View {
        HStack(spacing: 6) {
            ForEach(Saved.suggestedTags, id: \.self) { t in
                SavedTagButton(label: t, selected: tag == t) { tag = t }
            }
        }
        .accessibilityIdentifier("suggestedTags")
        TextField("Or type a tag", text: $tag).accessibilityIdentifier("tagField")
    }
}

private struct SavedTagButton: View {
    let label: String
    let selected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(label)
                .font(.subheadline)
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .foregroundStyle(selected ? Theme.onAccent : .primary)
                .background(selected ? AnyShapeStyle(Theme.accent) : AnyShapeStyle(Theme.card), in: Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

private struct SavedTagLabel: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.caption.weight(.medium))
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
            .background(Theme.accent.opacity(0.15), in: Capsule())
            .foregroundStyle(Theme.accent)
    }
}

/// A saved item's Markdown as blocks, as the web's item page reads: headings, lists, code, quotes, tables and rules.
/// The chat draws inline Markdown only; a saved tutorial reads better with its structure.
private struct SavedMarkdown: View {
    let blocks: [MarkdownBlock]

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(blocks.indices, id: \.self) { block(blocks[$0]) }
        }
        .textSelection(.enabled)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder private func block(_ b: MarkdownBlock) -> some View {
        switch b {
        case .heading(let level, let text):
            Text(Markdown.render(text)).font(level == 1 ? .title3.weight(.semibold) : .headline).accessibilityAddTraits(.isHeader)
        case .paragraph(let text):
            Text(Markdown.render(text))
        case .list(_, let items):
            VStack(alignment: .leading, spacing: 4) {
                ForEach(items.indices, id: \.self) { i in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Group {
                            if let checked = items[i].checked { Image(systemName: checked ? "checkmark.square" : "square") } else { Text(items[i].marker) }
                        }
                        .foregroundStyle(.secondary)
                        VStack(alignment: .leading, spacing: 6) {
                            if !items[i].text.isEmpty { Text(Markdown.render(items[i].text)) }
                            if !items[i].blocks.isEmpty { SavedMarkdown(blocks: items[i].blocks) }
                        }
                    }
                    .padding(.leading, CGFloat(items[i].depth) * 16)
                }
            }
        case .code(_, let text):
            ScrollView(.horizontal) {
                Text(verbatim: text).font(.system(.footnote, design: .monospaced)).padding(10)
            }
            .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 8))
        case .quote(let inner):
            HStack(spacing: 10) {
                RoundedRectangle(cornerRadius: 1).fill(.secondary).frame(width: 2)
                SavedMarkdown(blocks: inner).foregroundStyle(.secondary)
            }
            .fixedSize(horizontal: false, vertical: true)
        case .table(let header, _, let rows):
            // Each row as a line of its cells: a phone is too narrow for columns.
            VStack(alignment: .leading, spacing: 4) {
                Text(Markdown.render(header.joined(separator: " · "))).fontWeight(.semibold)
                ForEach(rows.indices, id: \.self) { Text(Markdown.render(rows[$0].joined(separator: " · "))) }
            }
        case .rule:
            Divider()
        }
    }
}
