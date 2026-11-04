import SwiftUI
import WontonClient

/// 安装、升级的确认页：原样显示家里的服务器给的预览，确认后把这份预览交回核心，授予的就是这里列出的权限。
struct InstallConfirmView: View {
    let model: PluginsModel
    let confirmation: PluginConfirmation
    @Environment(\.dismiss) private var dismiss
    @State private var loaded: Result<InstallPreview, Error>?
    @State private var rejection: String?
    @State private var lanWebui = false

    var body: some View {
        VStack(spacing: 0) {
            content.frame(maxWidth: .infinity, maxHeight: .infinity)
            if let preview {
                Button {
                    confirm(preview)
                } label: {
                    Text(isUpgrade ? "升级" : "安装").frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle())
                .disabled(!canConfirm)
                .padding(WontonTheme.spacing.pageInset)
            }
        }
        .wontonModalScreen(isUpgrade ? "升级" : "安装")
        .wontonDialog("操作被拒绝", message: rejection, isPresented: isShowingRejection)
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
            PreviewContent(model: model, preview: preview, lanWebui: $lanWebui)
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
            let preview = switch confirmation {
            case let .install(source): try await model.core.previewInstall(source: source)
            case let .upgrade(id): try await model.core.previewUpgrade(id: id)
            }
            lanWebui = preview.lanWebui?.enabled ?? false
            loaded = .success(preview)
        } catch {
            loaded = .failure(error)
        }
    }

    /// 在这里直接发给核心：被拒绝时原因留在确认页上，不随确认页关掉。
    private func confirm(_ shown: InstallPreview) {
        var preview = shown
        preview.lanWebui?.enabled = lanWebui
        let action: Action = isUpgrade ? .upgradePlugin(preview: preview) : .installPlugin(preview: preview)
        do {
            try model.core.dispatch(action: action)
            dismiss()
        } catch {
            rejection = String(describing: error)
        }
    }
}

private struct PreviewContent: View {
    let model: PluginsModel
    let preview: InstallPreview
    @Binding var lanWebui: Bool
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                header
                if let blocked = preview.blocked {
                    WontonNotice(icon: .blocked, title: blocked)
                }
                if !preview.notices.isEmpty {
                    NoticeLines(lines: preview.notices)
                }
                if !preview.permissions.isEmpty {
                    DetailSection(title: "权限") {
                        ForEach(preview.permissions, id: \.permission) { PermissionRow(line: $0) }
                    }
                }
                if let offer = preview.lanWebui {
                    LanWebuiToggle(offer: offer, isOn: $lanWebui)
                }
                if let notes = preview.releaseNotes {
                    DetailSection(title: "更新说明") {
                        Text(notes)
                            .wontonText(WontonTheme.typography.body)
                            .foregroundStyle(Color(colors.textPrimary))
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(WontonTheme.spacing.pageInset)
        }
    }

    private var header: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            PluginIconView(icon: preview.icon, category: nil, size: WontonTheme.size.previewIcon, model: model)
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(preview.title)
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                Text(version)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                HStack(spacing: WontonTheme.spacing.s8) {
                    Tag(text: preview.trust.label, tint: preview.trust.tint(colors))
                    if let author = preview.author {
                        Text(author.name)
                            .wontonText(WontonTheme.typography.caption)
                            .foregroundStyle(Color(colors.textSecondary))
                    }
                }
            }
        }
    }

    private var version: String {
        guard let from = preview.upgradeFrom else { return preview.version }
        return "\(from) → \(preview.version)"
    }
}
