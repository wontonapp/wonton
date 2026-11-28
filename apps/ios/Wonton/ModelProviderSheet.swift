import SwiftUI
import WontonClient

/// 点开的一家服务商：按插件给的接法摆登录和填 key，接好后选模型（ADR-0129 第 3 节）。
struct ModelProviderSheet: View {
    let page: ModelProviderPage
    let current: ModelChoice?
    /// 上一个动作没成的原因；带 `field` 的标在对应的输入框上。
    let problem: AgentModelsProblem?
    let rejection: Problem?
    let reload: () -> Void
    let send: (AgentModelsAction) -> Void
    @State private var selected: String?
    @State private var confirmsDisconnect = false
    @Environment(\.wontonColors) private var colors

    private var models: [ModelOption] {
        if case let .ready(models) = page.models { return models }
        return []
    }

    /// 没点过时预选：正在用这家的模型就选它，否则选第一个（插件推荐的，核心已经排在最前）。
    private var selection: String? {
        if let selected, models.contains(where: { $0.id == selected }) { return selected }
        if let current, current.provider == page.row.id, models.contains(where: { $0.id == current.model }) {
            return current.model
        }
        return models.first?.id
    }

    private var inUse: String? { current?.provider == page.row.id ? current?.model : nil }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                notices
                // 接好以后也留着登录和 key：换一份不用先断开（和 Android 一样）。
                if page.row.connected {
                    ModelProviderConnected(page: page) { confirmsDisconnect = true }
                }
                if page.deviceLogin || page.browserLogin {
                    ModelLoginSection(page: page, send: send)
                }
                if let form = page.key {
                    ModelKeyFormView(page: page, form: form, problem: problem, send: send)
                }
                ModelListSection(
                    state: page.models, selection: selection, inUse: inUse, retry: reload,
                    select: { selected = $0 }
                )
            }
            .padding(.horizontal, WontonTheme.spacing.pageInset)
            .padding(.vertical, WontonTheme.spacing.s16)
        }
        .scrollDismissesKeyboard(.interactively)
        .modelBottomBar { useButton }
        .wontonModalScreen(page.row.name)
        .wontonDialog(
            String(localized: .AgentModels0.agentModelsProviderDisconnectTitle(page.row.name)),
            message: String(localized: .AgentModels0.agentModelsProviderDisconnectMessage),
            isPresented: $confirmsDisconnect,
            actions: [
                WontonDialogAction(title: String(localized: .Common.commonCancel)),
                WontonDialogAction(
                    title: String(localized: .AgentModels1.agentModelsProviderDisconnect), kind: .danger,
                    perform: { send(.disconnectModelProvider) }
                )
            ]
        )
        .wontonDialog(
            page.confirm?.model ?? "",
            message: page.confirm?.message,
            isPresented: Binding(get: { page.confirm != nil }, set: { _ in }),
            actions: [
                WontonDialogAction(
                    title: String(localized: .Common.commonCancel), perform: { send(.dismissModelConfirmation) }
                ),
                WontonDialogAction(
                    title: String(localized: .AgentModels2.agentModelsConfirmUse), kind: .primary,
                    perform: { send(.confirmModelSelection) }
                )
            ],
            // 弹窗开着是因为核心在等确认：按钮和点空白都只发动作，核心清掉 `confirm` 弹窗才关。
            onDismiss: { send(.dismissModelConfirmation) }
        )
    }

    @ViewBuilder private var notices: some View {
        // 标得到输入框上的（`field`）交给那个输入框，这里不再说一遍。
        if let problem, problem.field == nil || page.key == nil || page.row.connected {
            ProblemNotice(problem: problem.problem)
        } else if problem == nil, let rejection {
            ProblemNotice(problem: rejection)
        }
        if page.row.connected, page.keyCheck?.check == .unverified {
            WontonNotice(
                icon: .info, title: String(localized: .AgentModels2.agentModelsKeyUnverified), isError: false
            ) {
                if let message = page.keyCheck?.message, !message.isEmpty { ProblemDetail(text: message) }
            }
        }
    }

    @ViewBuilder private var useButton: some View {
        if let selection {
            Button { send(.selectModel(model: selection)) } label: {
                HStack(spacing: WontonTheme.spacing.s8) {
                    if page.busy { ProgressView().tint(Color(colors.onPrimary)) }
                    Text(.AgentModels2.agentModelsModelsUse)
                }
                .frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle())
            .disabled(page.busy || selection == inUse)
            .padding(.horizontal, WontonTheme.spacing.pageInset)
            .padding(.vertical, WontonTheme.spacing.s8)
        }
    }
}

/// 已接好：账号或 key 的尾号，和断开。
private struct ModelProviderConnected: View {
    let page: ModelProviderPage
    let disconnect: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            WontonIcon.checkCircleFill.view().foregroundStyle(Color(colors.success))
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(.AgentModels2.agentModelsProvidersConnected)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                if let account = page.row.account, !account.isEmpty {
                    Text(verbatim: account)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .combine)
            Button(String(localized: .AgentModels1.agentModelsProviderDisconnect), action: disconnect)
                .buttonStyle(WontonSmallButtonStyle(kind: .danger))
                .disabled(page.busy)
        }
        .padding(.leading, WontonTheme.spacing.s16)
        .padding(.trailing, WontonTheme.spacing.s8)
        .padding(.vertical, WontonTheme.spacing.s4)
        .background(RoundedRectangle(cornerRadius: WontonTheme.radii.card).fill(Color(colors.surfaceSubtle)))
    }
}

/// 这家的模型：没接好不显示；点一行选中，底下的按钮才保存。
private struct ModelListSection: View {
    let state: ModelListState
    let selection: String?
    let inUse: String?
    let retry: () -> Void
    let select: (String) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        switch state {
        case .notConnected:
            EmptyView()
        case .loading:
            ProgressView().frame(maxWidth: .infinity).padding(.vertical, WontonTheme.spacing.s16)
        case let .failed(problem):
            ProblemRow(problem: problem.problem, retry: retry)
        case let .ready(models):
            if models.isEmpty {
                Text(.AgentModels2.agentModelsModelsEmpty)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            } else {
                LazyVStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                    ForEach(models, id: \.id) { model in
                        StepOptionRow(
                            title: model.name ?? model.id, description: model.name == nil ? nil : model.id,
                            isSelected: model.id == selection, neutralSelection: true,
                            select: { select(model.id) },
                            trailing: { tag(model) }
                        )
                    }
                }
            }
        }
    }

    @ViewBuilder private func tag(_ model: ModelOption) -> some View {
        if model.id == inUse {
            Text(.AgentModels2.agentModelsModelsInUse)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.success))
        } else if model.recommended {
            Text(.AgentModels0.agentModelsModelsRecommended)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        }
    }
}

private extension View {
    /// 钉在底下的主按钮：列表从它后面滚过去（同 `wontonTopBar` 的做法）。
    @ViewBuilder func modelBottomBar<Bar: View>(@ViewBuilder _ bar: () -> Bar) -> some View {
        let content = bar()
        if #available(iOS 26, *) {
            safeAreaBar(edge: .bottom, spacing: 0) { content }
        } else {
            safeAreaInset(edge: .bottom, spacing: 0) {
                content.background(.ultraThinMaterial, ignoresSafeAreaEdges: .bottom)
            }
        }
    }
}
