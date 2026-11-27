import SwiftUI
import WontonClient

/// 登录这一家（ADR-0129 第 3 节）。没有回跳的设备码登录用 App 内的 Safari 页，核心轮询到批准就清掉 `login`，
/// 这里随之把页面收起；能回跳的授权用系统的登录面板，回跳网址原样交回核心。
struct ModelLoginSection: View {
    let page: ModelProviderPage
    let send: (AgentModelsAction) -> Void
    @State private var safari: SafariTarget?
    @State private var webAuth = ModelLoginWebAuth()
    /// 这个版本认不出的打开方式，或网址打不开。
    @State private var cannotOpen = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            if cannotOpen || webAuth.failedToOpen {
                ProblemNotice(problem: Problem(
                    title: String(localized: .AgentModels0.agentModelsLoginCannotOpen),
                    nextStep: String(localized: .Problem0.problemPluginProtocolNextStep)
                ))
            } else if let outcome = page.loginOutcome, let text = outcome.state.outcomeText {
                ProblemNotice(problem: Problem(title: text, detail: outcome.message))
            }
            if let login = page.login {
                progress(login)
            } else {
                Button { send(.startModelLogin) } label: {
                    HStack(spacing: WontonTheme.spacing.s8) {
                        if page.busy { ProgressView().tint(Color(colors.onPrimary)) }
                        Text(startTitle)
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle())
                .disabled(page.busy)
            }
        }
        .onChange(of: page.login, initial: true) { _, login in follow(login) }
        .onDisappear { webAuth.close() }
        .sheet(item: $safari, onDismiss: safariClosed) { target in
            SafariView(url: target.url).ignoresSafeArea()
        }
    }

    private var startTitle: LocalizedStringResource {
        if page.loginOutcome != nil { return .AgentModels0.agentModelsLoginAgain }
        return page.browserLogin ? .AgentModels0.agentModelsLoginOneTap : .AgentModels2.agentModelsLoginStart
    }

    @ViewBuilder private func progress(_ login: ModelLoginProgress) -> some View {
        if let code = login.code, !code.isEmpty {
            ModelLoginCode(code: code)
            Button {
                UIPasteboard.general.string = code
                open(login)
            } label: {
                Text(.AgentModels1.agentModelsLoginCodeCopyOpen).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle())
        } else {
            HStack(spacing: WontonTheme.spacing.s8) {
                ProgressView()
                Text(.AgentModels0.agentModelsLoginWaiting)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        Button { send(.cancelModelLogin) } label: {
            Text(.Common.commonCancel).frame(maxWidth: .infinity)
        }
        .buttonStyle(WontonButtonStyle(kind: .secondary))
    }

    /// `login` 一变就跟上：有了就开浏览器，没了就关。带码的先让人看到码，点按钮才开。
    private func follow(_ login: ModelLoginProgress?) {
        guard let login else {
            safari = nil
            webAuth.close()
            return
        }
        cannotOpen = false
        if login.code?.isEmpty == false { return }
        open(login)
    }

    private func open(_ login: ModelLoginProgress) {
        guard let url = URL(string: login.url) else { return fail() }
        switch login.browser {
        case .page:
            safari = SafariTarget(url: url)
        case .redirect:
            webAuth.open(url: url) { result in
                switch result {
                case let .callback(url): send(.finishModelLogin(callbackUrl: url.absoluteString))
                case .cancelled, .failed: send(.cancelModelLogin)
                }
            }
        case .other:
            fail()
        }
    }

    private func fail() {
        cannotOpen = true
        send(.cancelModelLogin)
    }

    /// 人自己关了登录页：没有码可看的登录就此取消；带码的留着，可能只是回来再看一眼码。
    private func safariClosed() {
        if let login = page.login, login.code?.isEmpty != false { send(.cancelModelLogin) }
    }
}

/// 设备码：大字、等宽、能选中；复制在下面的主按钮上。
private struct ModelLoginCode: View {
    let code: String
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            Text(verbatim: code)
                .font(.system(.largeTitle, design: .monospaced).weight(.semibold))
                .kerning(2)
                .minimumScaleFactor(0.5)
                .lineLimit(1)
                .foregroundStyle(Color(colors.textPrimary))
                .textSelection(.enabled)
            Text(.AgentModels1.agentModelsLoginCodeHint)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(WontonTheme.spacing.s24)
        .background(RoundedRectangle(cornerRadius: WontonTheme.radii.card).fill(Color(colors.surfaceSubtle)))
    }
}
