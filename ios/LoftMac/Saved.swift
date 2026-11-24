import LoftKit
import SwiftUI

/// The Saved section (spec #675), as the web's Saved page: every kept message, newest first, with search and a tag
/// filter; an item opens as a readable page with Edit and Delete.
struct SavedSection: View {
    @Environment(LoftStore.self) private var store
    @Environment(MacNav.self) private var nav
    @State private var query = ""
    @State private var tag = ""
    @State private var list: SavedList?
    @State private var error: String?

    var body: some View {
        @Bindable var nav = nav
        NavigationStack(path: $nav.savedPath) {
            Group {
                if store.info != nil && store.info?.supports(.saved) != true {
                    ContentUnavailableView("Saved needs a newer Loft", systemImage: "bookmark",
                                           description: Text("This server doesn't have Saved yet. Pull the latest Loft and restart it."))
                } else {
                    content
                }
            }
            .navigationTitle("Saved")
            .toolbarBackground(WebTheme.canvas, for: .windowToolbar)
            .navigationDestination(for: Int64.self) { SavedPage(id: $0) }
        }
        // Typing searches once it pauses; the list reloads when you come back from an item.
        .task(id: "\(query)\n\(tag)\n\(nav.savedPath.count)") {
            guard nav.savedPath.isEmpty else { return }
            try? await Task.sleep(for: .milliseconds(200))
            guard !Task.isCancelled else { return }
            do {
                list = try await store.savedList(query: query.trimmingCharacters(in: .whitespaces), tag: tag)
                error = nil
            } catch {
                self.error = APIError.from(error).title
            }
        }
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                HStack(spacing: 10) {
                    TextField("Search titles and text", text: $query)
                        .textFieldStyle(.roundedBorder)
                        .frame(maxWidth: 280)
                        .accessibilityIdentifier("savedSearch")
                    ForEach([""] + Saved.tagOptions(list?.tags ?? []), id: \.self) { t in
                        Button(t.isEmpty ? "All" : t) { tag = t }
                            .buttonStyle(.plain)
                            .font(.system(size: 12))
                            .padding(.horizontal, 10).padding(.vertical, 3)
                            .foregroundStyle(tag == t ? WebTheme.fg : WebTheme.fg3)
                            .background(tag == t ? WebTheme.fg.opacity(0.08) : .clear, in: Capsule())
                            .accessibilityAddTraits(tag == t ? .isSelected : [])
                    }
                }
                if let error { Text(error).foregroundStyle(Theme.failureInk) }
                if let list, list.items.isEmpty {
                    Text(query.isEmpty && tag.isEmpty ? "Nothing saved yet. Save a message from a chat: right-click it, then Save." : "Nothing saved matches.")
                        .foregroundStyle(WebTheme.fg3)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 40)
                }
                ForEach(list?.items ?? []) { i in
                    Button { nav.savedPath.append(i.id) } label: { SavedRow(item: i) }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("savedRow-\(i.title)")
                }
            }
            .frame(maxWidth: 900, alignment: .leading)
            .padding(20)
            .frame(maxWidth: .infinity)
        }
    }
}

private struct SavedRow: View {
    let item: SavedItem

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                Text(item.title).font(.system(size: 14)).foregroundStyle(WebTheme.fg).lineLimit(1)
                Spacer(minLength: 8)
                if !item.tag.isEmpty { SavedTag(text: item.tag) }
            }
            Text(Saved.excerpt(item.content)).font(.system(size: 12.5)).foregroundStyle(WebTheme.fg3).lineLimit(2)
            Text("\(item.agentName) · \(item.createdAt.formatted(date: .abbreviated, time: .omitted))")
                .font(.system(size: 10.5, design: .monospaced))
                .foregroundStyle(WebTheme.fg3)
        }
        .padding(.horizontal, 16).padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(WebTheme.fg.opacity(0.02), in: RoundedRectangle(cornerRadius: WebTheme.rowRadius))
        .overlay(RoundedRectangle(cornerRadius: WebTheme.rowRadius).strokeBorder(WebTheme.line))
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}

/// One saved item: its title, tag, source and content, with Edit and Delete.
private struct SavedPage: View {
    @Environment(LoftStore.self) private var store
    @Environment(MacNav.self) private var nav
    @Environment(\.dismiss) private var dismiss
    let id: Int64
    @State private var item: SavedItem?
    @State private var error: String?
    @State private var editing = false
    @State private var confirmDelete = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if let error { Text(error).foregroundStyle(Theme.failureInk) }
                if let item, editing {
                    SavedEditForm(item: item) { edited in
                        self.item?.title = edited.title
                        self.item?.tag = edited.tag
                        self.item?.content = edited.content
                        self.item?.updatedAt = edited.updatedAt
                        editing = false
                    } cancel: { editing = false }
                } else if let item {
                    HStack(alignment: .top) {
                        Text(item.title).font(.system(size: 20, weight: .medium)).foregroundStyle(WebTheme.fg).textSelection(.enabled)
                            .accessibilityAddTraits(.isHeader)
                        Spacer()
                        Button { editing = true } label: { Label("Edit", systemImage: "pencil") }.accessibilityIdentifier("editSaved")
                        Button { confirmDelete = true } label: { Label("Delete", systemImage: "trash") }
                            .accessibilityIdentifier("deleteSaved")
                            .confirmationDialog("Delete this saved item?", isPresented: $confirmDelete) {
                                Button("Delete", role: .destructive) { delete() }
                                Button("Keep it", role: .cancel) {}
                            } message: {
                                Text("It's removed from Saved. The message in the chat stays. This can't be undone.")
                            }
                    }
                    HStack(spacing: 8) {
                        if !item.tag.isEmpty { SavedTag(text: item.tag) }
                        Text("From \(item.agentName) · \(item.createdAt.formatted(date: .abbreviated, time: .omitted))")
                        if let edited = item.updatedAt { Text("· Edited \(edited.formatted(date: .abbreviated, time: .shortened))") }
                        if item.sourceExists == true, store.agent(item.agentId) != nil {
                            Button { nav.openChat(item.agentId) } label: { Label("Open in chat", systemImage: "bubble.left") }
                                .buttonStyle(.link)
                                .accessibilityIdentifier("savedOpenChat")
                        } else if item.sourceExists == false {
                            Text("· The source message is gone")
                        }
                    }
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(WebTheme.fg3)
                    SavedBody(content: item.content)
                } else if error == nil {
                    ProgressView().frame(maxWidth: .infinity)
                }
            }
            .frame(maxWidth: 900, alignment: .leading)
            .padding(20)
            .frame(maxWidth: .infinity)
        }
        .navigationTitle(item?.title ?? "Saved")
        .task {
            do {
                item = try await store.savedItem(id: id)
            } catch {
                let e = APIError.from(error)
                if case .server(404, _) = e { self.error = "This saved item doesn't exist." } else { self.error = e.title }
            }
        }
    }

    private func delete() {
        Task {
            do {
                try await store.deleteSaved(id: id)
                dismiss()
            } catch {
                self.error = APIError.from(error).title
            }
        }
    }
}

/// An item's content as it reads on its page; the edit form's preview shows the same.
private struct SavedBody: View {
    let content: String

    var body: some View {
        MarkdownView(content)
            .foregroundStyle(WebTheme.fg)
            .padding(.horizontal, 20).padding(.vertical, 16)
            .background(WebTheme.fg.opacity(0.02), in: RoundedRectangle(cornerRadius: WebTheme.panelRadius))
            .overlay(RoundedRectangle(cornerRadius: WebTheme.panelRadius).strokeBorder(WebTheme.line))
    }
}

/// Edit: the title, the tag and the content as Markdown, with a preview.
struct SavedEditForm: View {
    @Environment(LoftStore.self) private var store
    let item: SavedItem
    let saved: (SavedItem) -> Void
    let cancel: () -> Void
    @State private var title: String
    @State private var tag: String
    @State private var content: String
    @State private var preview = false
    @State private var saving = false
    @State private var error: String?

    init(item: SavedItem, saved: @escaping (SavedItem) -> Void, cancel: @escaping () -> Void) {
        self.item = item
        self.saved = saved
        self.cancel = cancel
        _title = State(initialValue: item.title)
        _tag = State(initialValue: item.tag)
        _content = State(initialValue: item.content)
    }

    private var valid: Bool { !title.trimmingCharacters(in: .whitespaces).isEmpty && !content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            LabeledField("Title") { TextField("Title", text: $title).textFieldStyle(.roundedBorder).accessibilityIdentifier("savedTitleField") }
            LabeledField("Tag") { SavedTagPicker(tag: $tag) }
            HStack {
                Text("Content, in Markdown").font(.system(size: 12)).foregroundStyle(WebTheme.fg3)
                Spacer()
                Picker("Content view", selection: $preview) {
                    Text("Write").tag(false)
                    Text("Preview").tag(true)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()
            }
            if preview {
                SavedBody(content: content)
            } else {
                TextEditor(text: $content)
                    .font(.system(size: 12.5, design: .monospaced))
                    .frame(minHeight: 320)
                    .padding(6)
                    .background(WebTheme.field, in: RoundedRectangle(cornerRadius: 10))
                    .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(WebTheme.lineStrong))
                    .accessibilityIdentifier("savedContentField")
            }
            if let error { Text(error).foregroundStyle(Theme.failureInk) }
            HStack {
                Spacer()
                Button("Cancel", action: cancel).keyboardShortcut(.cancelAction)
                Button(saving ? "Saving…" : "Save") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!valid || saving)
                    .accessibilityIdentifier("saveEdit")
            }
        }
    }

    private func save() {
        saving = true
        Task {
            defer { saving = false }
            do {
                saved(try await store.updateSaved(id: item.id, title: title.trimmingCharacters(in: .whitespaces),
                                                  tag: tag.trimmingCharacters(in: .whitespaces), content: content))
            } catch {
                self.error = APIError.from(error).title
            }
        }
    }
}

/// Save in a chat message's menu: a small form for the title and tag, then the message is kept in Saved.
struct SaveMessageSheet: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let message: ChatMessage
    @State private var title: String
    @State private var tag = Saved.suggestedTags[0]
    @State private var saving = false
    @State private var error: String?

    init(message: ChatMessage) {
        self.message = message
        _title = State(initialValue: Saved.defaultTitle(message.displayContent))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Save message").font(.system(size: 15, weight: .medium))
            LabeledField("Title") { TextField("Title", text: $title).textFieldStyle(.roundedBorder).accessibilityIdentifier("saveTitleField") }
            LabeledField("Tag") { SavedTagPicker(tag: $tag) }
            if let error { Text(error).foregroundStyle(Theme.failureInk) }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(saving ? "Saving…" : "Save") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(title.trimmingCharacters(in: .whitespaces).isEmpty || saving)
                    .accessibilityIdentifier("saveMessageConfirm")
            }
        }
        .padding(20)
        .frame(width: 420)
    }

    private func save() {
        saving = true
        Task {
            defer { saving = false }
            do {
                _ = try await store.saveMessage(message, title: title.trimmingCharacters(in: .whitespaces), tag: tag.trimmingCharacters(in: .whitespaces))
                dismiss()
            } catch {
                self.error = APIError.from(error).title
            }
        }
    }
}

private struct LabeledField<Content: View>: View {
    let label: String
    @ViewBuilder let content: Content

    init(_ label: String, @ViewBuilder content: () -> Content) {
        self.label = label
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(label).font(.system(size: 12)).foregroundStyle(WebTheme.fg3)
            content
        }
    }
}

/// The suggested tags as chips, and a field for any other.
private struct SavedTagPicker: View {
    @Binding var tag: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                ForEach(Saved.suggestedTags, id: \.self) { t in
                    Button(t) { tag = t }
                        .buttonStyle(.plain)
                        .font(.system(size: 12))
                        .padding(.horizontal, 10).padding(.vertical, 3)
                        .foregroundStyle(tag == t ? WebTheme.fg : WebTheme.fg2)
                        .background(tag == t ? Theme.accent.opacity(0.12) : .clear, in: Capsule())
                        .overlay(Capsule().strokeBorder(tag == t ? Theme.accent.opacity(0.6) : WebTheme.lineStrong))
                        .accessibilityAddTraits(tag == t ? .isSelected : [])
                }
            }
            TextField("Or type a tag", text: $tag).textFieldStyle(.roundedBorder).accessibilityIdentifier("tagField")
        }
    }
}

private struct SavedTag: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.system(size: 11))
            .padding(.horizontal, 8).padding(.vertical, 2)
            .foregroundStyle(WebTheme.fg2)
            .overlay(Capsule().strokeBorder(WebTheme.lineStrong))
    }
}
