import SwiftUI
import WontonClient

/// 第三方客户端连这个插件用的地址和账号，可以选中复制。
private struct LanClientAccount: View {
    let client: LanClient
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            ForEach(client.addresses, id: \.self) { row("地址", $0) }
            row("账号", client.username)
            row("密码", client.password)
        }
    }

    private func row(_ name: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: WontonTheme.spacing.s8) {
            Text(name)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            Text(value)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
                .textSelection(.enabled)
        }
    }
}

/// 已装插件的开关：核心重建容器期间不能再点，失败时写原因，并重取详情（关失败时插件标为异常）。
struct InstalledLanWebui: View {
    let model: PluginsModel
    let id: String
    let offer: LanWebuiOffer
    let updated: (PluginDetail) -> Void
    @State private var target: Bool?
    @State private var problem: Problem?

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            LanWebuiToggle(
                offer: offer,
                isOn: Binding(get: { target ?? offer.enabled }, set: { change(to: $0) })
            )
            .disabled(target != nil)
            if target != nil { ProgressView() }
            if let problem { ProblemNotice(problem: problem) }
            if let clientProblem = offer.clientProblem { WontonNotice(icon: .warning, title: clientProblem) }
            if let client = offer.client { LanClientAccount(client: client) }
        }
    }

    private func change(to enabled: Bool) {
        target = enabled
        problem = nil
        Task {
            do {
                updated(try await model.core.setLanWebui(id: id, enabled: enabled))
            } catch {
                problem = error.problem
                if let detail = try? await model.core.installedPlugin(id: id) { updated(detail) }
            }
            target = nil
        }
    }
}
