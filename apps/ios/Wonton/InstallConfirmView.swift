import SwiftUI
import WontonClient

/// 安装、升级的确认页：原样显示家里的服务器给的预览，确认后把这份预览交回核心，授予的就是这里列出的权限。
struct InstallConfirmView: View {
    let model: PluginsModel
    let confirmation: PluginConfirmation
    @Environment(\.dismiss) private var dismiss
    @State private var loaded: Result<InstallPreview, Error>?
    @State private var rejection: String?

    var body: some View {
        NavigationStack {
            content
                .navigationTitle(isUpgrade ? "升级" : "安装")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("取消") { dismiss() }
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button(isUpgrade ? "升级" : "安装") {
                            if let preview { confirm(preview) }
                        }
                        .disabled(!canConfirm)
                    }
                }
                .alert("操作被拒绝", isPresented: isShowingRejection, presenting: rejection) { _ in
                    Button("好") {}
                } message: { reason in
                    Text(reason)
                }
        }
        .task { await load() }
    }

    @ViewBuilder private var content: some View {
        switch loaded {
        case nil:
            ProgressView()
        case let .failure(error):
            ProblemView(message: String(describing: error), nextStep: nil) {
                Task { await load() }
            }
        case let .success(preview):
            PreviewList(model: model, preview: preview)
        }
    }

    private var isUpgrade: Bool {
        if case .upgrade = confirmation { return true }
        return false
    }

    private var preview: InstallPreview? {
        if case let .success(preview) = loaded { return preview }
        return nil
    }

    /// 装不了（`blocked`）或者有核心不支持的权限时不能确认。
    private var canConfirm: Bool {
        guard let preview else { return false }
        return preview.blocked == nil && preview.permissions.allSatisfy(\.supported)
    }

    private var isShowingRejection: Binding<Bool> {
        Binding(
            get: { rejection != nil },
            set: { isShowing in
                if !isShowing { rejection = nil }
            }
        )
    }

    private func load() async {
        loaded = nil
        do {
            switch confirmation {
            case let .install(source):
                loaded = .success(try await model.core.previewInstall(source: source))
            case let .upgrade(id):
                loaded = .success(try await model.core.previewUpgrade(id: id))
            }
        } catch {
            loaded = .failure(error)
        }
    }

    /// 在这里直接发给核心：被拒绝时原因留在确认页上，不随确认页关掉。
    private func confirm(_ preview: InstallPreview) {
        let action: Action = isUpgrade ? .upgradePlugin(preview: preview) : .installPlugin(preview: preview)
        do {
            try model.core.dispatch(action: action)
            dismiss()
        } catch {
            rejection = String(describing: error)
        }
    }
}

private struct PreviewList: View {
    let model: PluginsModel
    let preview: InstallPreview

    var body: some View {
        List {
            Section {
                HStack(spacing: 14) {
                    PluginIconView(icon: preview.icon, kind: preview.kind, size: 64, model: model)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(preview.title).font(.headline)
                        Text(version).font(.subheadline).foregroundStyle(.secondary)
                        HStack(spacing: 6) {
                            Tag(text: preview.trust.label, tint: preview.trust.tint)
                            if let author = preview.author {
                                Text(author.name).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                if let blocked = preview.blocked {
                    Label(blocked, systemImage: "nosign").foregroundStyle(.red)
                }
            }
            if !preview.notices.isEmpty {
                Section { NoticeLines(lines: preview.notices) }
            }
            if !preview.permissions.isEmpty {
                Section("权限") {
                    ForEach(preview.permissions, id: \.permission) { PermissionRow(line: $0) }
                }
            }
            if let notes = preview.releaseNotes {
                Section("更新说明") { Text(notes) }
            }
        }
    }

    private var version: String {
        guard let from = preview.upgradeFrom else { return preview.version }
        return "\(from) → \(preview.version)"
    }
}
