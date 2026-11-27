import SwiftUI
import WontonClient

/// 一个 Bot 的“模型”页（ADR-0129）：顶上是在用的模型和测试，下面是服务商；点一家，从底下升起它的那一页。
/// 自己不带滚动容器，放进引导的卡片或别的页面里。
struct AgentModelsPage: View {
    let view: AgentModelsViewModel
    /// 核心当场拒掉的动作。
    var rejection: Problem?
    let send: (AgentModelsAction) -> Void
    /// 收起那一页的转场里核心已经把 `provider` 清空了，照最后一次的内容画完。
    @State private var shownProvider: ModelProviderPage?

    var body: some View {
        Group {
            switch view.state {
            case .loading:
                LoadingView().frame(maxWidth: .infinity)
            case let .failed(problem):
                ProblemView(problem: problem.problem, retry: reload).frame(maxWidth: .infinity)
            case .ready:
                ready
            }
        }
        .onChange(of: view.provider, initial: true) { _, page in
            if let page { shownProvider = page }
        }
        .onChange(of: view.current) { _, current in
            // 选好了：收起这一家，回到顶上看测试结果。
            if let current, current.provider == view.provider?.row.id { send(.closeModelProvider) }
        }
        .sheet(isPresented: Binding(
            get: { view.provider != nil },
            set: { if !$0 { send(.closeModelProvider) } }
        )) {
            if let page = view.provider ?? shownProvider {
                ModelProviderSheet(
                    page: page, current: view.current, problem: view.actionProblem,
                    rejection: rejection, reload: reload, send: send
                )
                .id(page.row.id)
            }
        }
    }

    private var ready: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            if view.provider == nil {
                if let problem = view.actionProblem {
                    ProblemNotice(problem: problem.problem)
                } else if let rejection {
                    ProblemNotice(problem: rejection)
                }
            }
            CurrentModelCard(
                current: view.current, providerName: view.current.map { view.providerName($0.provider) },
                test: view.test, runTest: { send(.testModel) }
            )
            ModelProviderList(view: view, send: send)
        }
    }

    private func reload() {
        if let bot = view.bot { send(.openAgentModels(bot: bot)) }
    }
}

/// 在用的模型和最近一次测试。
private struct CurrentModelCard: View {
    let current: ModelChoice?
    let providerName: String?
    let test: ModelTestState?
    let runTest: () -> Void
    @Environment(\.wontonColors) private var colors

    private var isTesting: Bool {
        if case .running = test { return true }
        return false
    }

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            if let current {
                HStack(alignment: .center, spacing: WontonTheme.spacing.s12) {
                    VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                        Text(verbatim: providerName ?? current.provider)
                            .wontonText(WontonTheme.typography.supporting)
                            .foregroundStyle(Color(colors.textSecondary))
                        Text(verbatim: current.model)
                            .wontonText(WontonTheme.typography.itemTitle)
                            .foregroundStyle(Color(colors.textPrimary))
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityElement(children: .combine)
                    Button(action: runTest) {
                        HStack(spacing: WontonTheme.spacing.s8) {
                            if isTesting { ProgressView().controlSize(.small) }
                            Text(.AgentModels1.agentModelsTestRun)
                        }
                    }
                    .buttonStyle(WontonSmallButtonStyle())
                    .disabled(isTesting)
                }
                if let test { ModelTestLine(test: test) }
                Text(.AgentModels1.agentModelsTestNote)
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.textSecondary))
            } else {
                Text(.AgentModels0.agentModelsCurrentNone)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textSecondary))
            }
        }
        .padding(WontonTheme.spacing.s16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: WontonTheme.radii.card).fill(Color(colors.surface)))
        .overlay(
            RoundedRectangle(cornerRadius: WontonTheme.radii.card)
                .strokeBorder(Color(colors.separator), lineWidth: WontonTheme.size.hairline)
        )
    }
}

/// 测试的结果：通了、没通（按类别）、测试本身没做成，分开写。
private struct ModelTestLine: View {
    let test: ModelTestState
    @Environment(\.wontonColors) private var colors

    var body: some View {
        switch test {
        case .running:
            line(icon: nil, text: String(localized: .AgentModels0.agentModelsTestRunning), tint: colors.textSecondary)
        case let .done(result):
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                if result.result == .ok {
                    line(icon: .checkCircleFill, text: result.text, tint: colors.success)
                } else {
                    line(icon: .warningFill, text: result.text, tint: colors.danger)
                }
                if let message = result.message, !message.isEmpty { ProblemDetail(text: message) }
            }
        case let .failed(problem):
            let problem = problem.problem
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                line(icon: .warningFill, text: problem.title, tint: colors.danger)
                if let nextStep = problem.nextStep {
                    Text(verbatim: nextStep)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                if let detail = problem.detail { ProblemDetail(text: detail) }
            }
        }
    }

    private func line(icon: WontonIcon?, text: String, tint: Rgb) -> some View {
        HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
            if let icon { icon.view(size: WontonTheme.size.iconSmall) }
            Text(verbatim: text).wontonText(WontonTheme.typography.supporting)
        }
        .foregroundStyle(Color(tint))
    }
}

/// 服务商：插件推荐的在前，其余收在“更多”里，展开后能搜。
private struct ModelProviderList: View {
    let view: AgentModelsViewModel
    let send: (AgentModelsAction) -> Void
    @State private var showsMore = false
    @Environment(\.wontonColors) private var colors

    /// 没有推荐的时候“更多”就是全部，不用再点开；搜着的时候也不收。
    private var moreShown: Bool { showsMore || view.recommended.isEmpty || !view.search.isEmpty }
    private var hasMore: Bool { !view.more.isEmpty || !view.search.isEmpty }

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            if !view.recommended.isEmpty { group(view.recommended) }
            if hasMore, !view.recommended.isEmpty { moreToggle }
            if hasMore, moreShown {
                WontonSearchField(
                    text: Binding(get: { view.search }, set: { send(.searchModelProviders(text: $0)) }),
                    prompt: String(localized: .AgentModels1.agentModelsProvidersSearch)
                )
                if view.more.isEmpty {
                    Text(.AgentModels1.agentModelsProvidersNoMatch)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, WontonTheme.spacing.s16)
                } else {
                    group(view.more)
                }
            }
        }
    }

    private func group(_ rows: [ModelProviderRow]) -> some View {
        WontonGroup {
            ForEach(Array(rows.enumerated()), id: \.element.id) { index, row in
                if index > 0 { WontonSeparator() }
                Button { send(.openModelProvider(provider: row.id)) } label: { ModelProviderRowView(row: row) }
                    .buttonStyle(.plain)
            }
        }
    }

    private var moreToggle: some View {
        Button {
            withAnimation(.easeOut(duration: WontonTheme.motion.state)) { showsMore.toggle() }
        } label: {
            HStack(spacing: WontonTheme.spacing.s4) {
                Text(.AgentModels2.agentModelsProvidersMore).wontonText(WontonTheme.typography.supporting)
                WontonIcon.forward.view(size: WontonTheme.size.iconTiny)
                    .rotationEffect(moreShown ? .degrees(90) : .zero)
                Spacer(minLength: 0)
            }
            .foregroundStyle(Color(colors.textSecondary))
            .padding(.horizontal, WontonTheme.spacing.s4)
            .frame(minHeight: WontonTheme.size.target)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityValue(moreShown ? Text(.Appearance1.appearanceExpanded) : Text(.Appearance0.appearanceCollapsed))
    }
}

private struct ModelProviderRowView: View {
    let row: ModelProviderRow
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(verbatim: row.name)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                if let account = row.account, !account.isEmpty {
                    Text(verbatim: account)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if row.connected {
                WontonIcon.checkCircleFill.view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(colors.success))
            }
            WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.textSecondary))
        }
        .padding(.horizontal, WontonTheme.spacing.s16)
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.target)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityValue(row.connected ? Text(.AgentModels2.agentModelsProvidersConnected) : Text(verbatim: ""))
    }
}
