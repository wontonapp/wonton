import SwiftUI
import WontonClient

/// 要填的密钥：只有主人看得到这一节；点开可改，可选项可以清空。改完核心会重建用到它的服务。
struct SecretSection: View {
    let detail: PluginDetail
    let model: PluginsModel
    let updated: (PluginDetail) -> Void
    @State private var isOwner = false

    var body: some View {
        if isOwner, !detail.secretInputs.isEmpty {
            DetailSection(title: String(localized: .secretSectionTitle)) {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(detail.secretInputs.enumerated()), id: \.element.name) { index, input in
                        if index > 0 { WontonSeparator() }
                        SecretRow(pluginId: detail.id, input: input, model: model, updated: updated)
                    }
                }
            }
            .task {
                isOwner = (try? await model.core.me())?.role == .owner
            }
        }
    }
}

private struct SecretRow: View {
    let pluginId: String
    let input: SecretInputState
    let model: PluginsModel
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
                    Text(verbatim: input.title)
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textPrimary))
                    Text(stateText)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                Spacer(minLength: 0)
                if !editing {
                    Button(String(localized: .secretEdit)) { editing = true }
                        .buttonStyle(WontonSmallButtonStyle())
                        .disabled(busy)
                }
            }
            if editing {
                let field = SecretInput(
                    name: input.name, title: input.title, description: nil, url: nil, required: input.required,
                )
                SecretInputField(field, draft) { draft = $0 }
                HStack(spacing: WontonTheme.spacing.s8) {
                    Button(String(localized: .commonSave)) {
                        perform(draft)
                    }
                    .buttonStyle(WontonSmallButtonStyle())
                    .disabled(busy || draft.isEmpty)
                    if !input.required {
                        Button(String(localized: .secretClear)) { perform(nil) }
                            .buttonStyle(WontonSmallButtonStyle())
                            .disabled(busy)
                    }
                    Button(String(localized: .commonCancel)) {
                        editing = false
                        draft = ""
                        problem = nil
                    }
                    .buttonStyle(WontonSmallButtonStyle())
                    .disabled(busy)
                }
                if let problem {
                    ProblemNotice(problem: problem)
                }
            }
        }
        .padding(.vertical, WontonTheme.spacing.s4)
    }

    private var stateText: String {
        let filled = input.filled ? String(localized: .secretFilled) : String(localized: .secretUnfilled)
        guard let changedAtMs = input.changedAtMs else { return filled }
        let changed = Date(timeIntervalSince1970: Double(changedAtMs) / 1000)
            .formatted(date: .abbreviated, time: .shortened)
        return "\(filled) · \(changed)"
    }

    private func perform(_ value: String?) {
        busy = true
        problem = nil
        Task {
            do {
                updated(try await model.core.setPluginSecret(id: pluginId, name: input.name, value: value))
                editing = false
                draft = ""
            } catch {
                problem = error.problem
            }
            busy = false
        }
    }
}
