import SwiftUI
import UniformTypeIdentifiers
import WontonClient

/// “添加来源”：填地址（家里的服务器判断是插件还是索引）、从文件安装、删除添加过的来源（ADR-0031）。
struct AddSourceView: View {
    let model: PluginsModel
    /// 选好了文件，关掉这一页去确认。
    let chosen: (PluginConfirmation) -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(\.wontonColors) private var colors
    @State private var address = ""
    @FocusState private var isEditingAddress: Bool
    @State private var isAdding = false
    @State private var problem: Problem?
    @State private var isPickingFile = false

    private var added: [AddedSource] { model.viewModel.store?.added ?? [] }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                WontonNotice(icon: .warning, title: String(localized: .pluginSourcesUnreviewedWarning))
                if let problem { ProblemNotice(problem: problem) }
                VStack(spacing: WontonTheme.spacing.s8) {
                    SourceAddressField(address: $address)
                        .focused($isEditingAddress)
                    Button(action: add) {
                        Text(String(localized: .pluginSourcesAdd)).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(WontonButtonStyle())
                    .disabled(isAdding || address.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
                Button {
                    isPickingFile = true
                } label: {
                    Text(String(localized: .pluginSourcesInstallFromFile)).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                if !added.isEmpty {
                    DetailSection(title: String(localized: .pluginSourcesMine)) {
                        WontonGroup {
                            ForEach(Array(added.enumerated()), id: \.element.address) { index, source in
                                if index > 0 { WontonSeparator() }
                                row(source)
                            }
                        }
                    }
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .scrollDismissesKeyboard(.immediately)
        .wontonModalScreen(String(localized: .pluginSourcesTitle))
        .pluginArchiveImporter(isPresented: $isPickingFile, model: model) { result in
            switch result {
            case let .success(archive):
                chosen(.upload(archive: archive))
                dismiss()
            case let .failure(failure):
                problem = Problem(title: failure.message)
            }
        }
    }

    private func row(_ source: AddedSource) -> some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                // 同一个主机上可以有好几个索引，只写主机分不清。
                Text(source.name ?? (source.kind == .index ? source.address : source.provider))
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                if source.name != nil {
                    Text(source.provider)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                }
            }
            Spacer(minLength: 0)
            WontonIconButton(icon: .trash, label: String(localized: .commonDelete)) { remove(source) }
        }
        .padding(.horizontal, WontonTheme.spacing.s12)
        .padding(.vertical, WontonTheme.spacing.s8)
    }

    private func add() {
        isAdding = true
        problem = nil
        Task {
            do {
                _ = try await model.core.addStoreSource(address: address)
                address = ""
                isEditingAddress = false
            } catch {
                problem = error.problem
            }
            isAdding = false
        }
    }

    private func remove(_ source: AddedSource) {
        problem = nil
        Task {
            do {
                try await model.core.removeStoreSource(address: source.address)
            } catch {
                problem = error.problem
            }
        }
    }
}

/// 换版本：填另一个来源，或者选文件。可以比装着的旧，确认页上会醒目提示（ADR-0031）。
struct ChangeVersionView: View {
    let model: PluginsModel
    let id: String
    let chosen: (PluginConfirmation) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var address = ""
    @State private var problem: Problem?
    @State private var isPickingFile = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let problem { ProblemNotice(problem: problem) }
                VStack(spacing: WontonTheme.spacing.s8) {
                    SourceAddressField(address: $address)
                    Button {
                        chosen(.upgradeFrom(id: id, source: address))
                        dismiss()
                    } label: {
                        Text(String(localized: .pluginSourcesContinue)).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(WontonButtonStyle())
                    .disabled(address.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
                Button {
                    isPickingFile = true
                } label: {
                    Text(String(localized: .pluginSourcesInstallFromFile)).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .scrollDismissesKeyboard(.immediately)
        .wontonModalScreen(String(localized: .installChangeVersion))
        .pluginArchiveImporter(isPresented: $isPickingFile, model: model) { result in
            switch result {
            case let .success(archive):
                chosen(.upgradeUpload(id: id, archive: archive))
                dismiss()
            case let .failure(failure):
                problem = Problem(title: failure.message)
            }
        }
    }
}

/// 打开“换版本”页。那一页选好的要等它关上以后才能再盖确认页。
private struct ChangeVersionCover: ViewModifier {
    @Binding var id: String?
    let model: PluginsModel
    @Binding var confirmation: PluginConfirmation?
    @State private var chosen: PluginConfirmation?

    func body(content: Content) -> some View {
        content.fullScreenCover(isPresented: isPresented, onDismiss: {
            confirmation = chosen
            chosen = nil
        }, content: {
            ChangeVersionView(model: model, id: id ?? "") { chosen = $0 }
        })
    }

    private var isPresented: Binding<Bool> {
        Binding(
            get: { id != nil },
            set: { isShowing in
                if !isShowing { id = nil }
            }
        )
    }
}

private struct SourceAddressField: View {
    @Binding var address: String

    var body: some View {
        // verbatim：字符串字面量会按 Markdown 认成链接，占位符显示成蓝色，像是已经填好了。
        TextField(
            String(localized: .pluginSourcesAddress),
            text: $address,
            prompt: Text(verbatim: "https://github.com/owner/repo")
        )
            .keyboardType(.URL)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .wontonField()
    }
}

/// 读不了主人选的文件的原因。
struct PluginArchiveProblem: Error {
    let message: String
}

extension View {
    func changeVersionCover(
        of id: Binding<String?>,
        model: PluginsModel,
        confirmation: Binding<PluginConfirmation?>
    ) -> some View {
        modifier(ChangeVersionCover(id: id, model: model, confirmation: confirmation))
    }

    /// 选插件的压缩包并读出内容。格式由家里的服务器按内容认，这里不按扩展名筛：`.tgz` 这类没有对应的系统类型。
    func pluginArchiveImporter(
        isPresented: Binding<Bool>,
        model: PluginsModel,
        picked: @escaping (Result<Data, PluginArchiveProblem>) -> Void
    ) -> some View {
        fileImporter(isPresented: isPresented, allowedContentTypes: [.item]) { result in
            switch result {
            case let .success(url):
                picked(readPluginArchive(url, limit: model.core.maxPluginArchiveBytes()))
            case let .failure(error):
                picked(.failure(PluginArchiveProblem(message: error.localizedDescription)))
            }
        }
    }
}

/// 太大的文件不读进内存，家里的服务器也不会收。
private func readPluginArchive(_ url: URL, limit: UInt64) -> Result<Data, PluginArchiveProblem> {
    let scoped = url.startAccessingSecurityScopedResource()
    defer { if scoped { url.stopAccessingSecurityScopedResource() } }
    do {
        let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        guard UInt64(size) <= limit else {
            let most = ByteCountFormatter.string(fromByteCount: Int64(limit), countStyle: .binary)
            return .failure(PluginArchiveProblem(message: String(localized: .pluginSourcesFileTooLarge(most))))
        }
        return .success(try Data(contentsOf: url))
    } catch {
        return .failure(PluginArchiveProblem(message: error.localizedDescription))
    }
}
