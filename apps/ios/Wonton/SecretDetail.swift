import SwiftUI
import WontonClient

/// 要填的密钥：只有主人看得到这一节；点开可改，可选项可以清空。改完核心会重建用到它的服务。
struct SecretSection: View {
    let model: PluginsModel
    let detail: PluginDetail
    let updated: (PluginDetail) -> Void
    @State private var isOwner = false

    var body: some View {
        Group {
            if isOwner, !detail.secretInputs.isEmpty {
                DetailSection(title: String(localized: .pluginSecretTitle)) {
                    ForEach(Array(detail.secretInputs.enumerated()), id: \.offset) { index, item in
                        SecretRow(model: model, pluginId: detail.id, item: item, updated: updated)
                        if index < detail.secretInputs.count - 1 { WontonSeparator() }
                    }
                }
            }
        }
        .task(id: detail.id) {
            isOwner = (try? await model.core.me())?.role == .owner
        }
    }
}

/// 一项密钥的填写情况；值本身不显示。
private struct SecretRow: View {
    let model: PluginsModel
    let pluginId: String
    let item: SecretInputState
    let updated: (PluginDetail) -> Void
    @State private var editing = false
    @State private var draft = ""
    @State private var busy = false
    @State private var problem: Problem?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            HStack(spacing: WontonTheme.spacing.s8) {
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                    Text(item.title)
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textPrimary))
                    Text(stateText)
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if !item.required {
                    Text(.pluginPermissionOptional)
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                if !editing {
                    Button(.pluginSecretEdit) { editing = true }
                        .buttonStyle(WontonSmallButtonStyle())
                        .disabled(busy)
                }
            }
            if editing { editor }
            if let problem { ProblemNotice(problem: problem) }
        }
    }

    private var editor: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            SecretInputField(input: input, value: $draft, enabled: !busy)
            HStack(spacing: WontonTheme.spacing.s8) {
                Button(.commonSave) { save(draft) }
                    .buttonStyle(WontonSmallButtonStyle())
                    .disabled(busy || draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                if !item.required {
                    Button(.pluginSecretClear) { save(nil) }
                        .buttonStyle(WontonSmallButtonStyle())
                        .disabled(busy)
                }
                Button(.commonCancel) { cancel() }
                    .buttonStyle(WontonSmallButtonStyle())
                    .disabled(busy)
            }
        }
    }

    private var stateText: String {
        guard item.filled else { return String(localized: .pluginSecretUnfilled) }
        guard let changedAt = item.changedAtMs else { return String(localized: .pluginSecretFilled) }
        let date = Date(timeIntervalSince1970: Double(changedAt) / 1000)
        return String(localized: .pluginSecretFilled) + " · "
            + date.formatted(date: .abbreviated, time: .shortened)
    }

    private var input: SecretInput {
        SecretInput(
            name: item.name, title: item.title, description: nil, url: nil, required: item.required
        )
    }

    private func save(_ value: String?) {
        busy = true
        problem = nil
        Task {
            do {
                updated(try await model.core.setPluginSecret(id: pluginId, name: item.name, value: value))
                editing = false
                draft = ""
            } catch {
                problem = error.problem
            }
            busy = false
        }
    }

    private func cancel() {
        editing = false
        draft = ""
        problem = nil
    }
}
