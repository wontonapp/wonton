import SwiftUI
import WontonClient

/// 用现成的 Matrix 账号登录（ADR-0051）。进展只来自核心的 `ServersViewModel.matrixLogin`：壳不猜服务器支持哪种方式，
/// 回调地址也用核心给的。成功后核心已经切到新槽位，这一页退回；返回时还没结束的流程交给核心取消。
struct MatrixLoginView: View {
    let model: HomeModel
    let onFinished: () -> Void
    /// 起手填的服务器，和核心不填时选的一样（ADR-0051）。
    @State private var server = "matrix.org"
    /// 核心当场拒绝的原因，比如用户 ID 写得不对。
    @State private var refusal: Problem?
    @State private var browser = MatrixLoginBrowser()

    private var login: MatrixLoginViewModel? { model.servers.matrixLogin }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let refusal { ProblemNotice(problem: refusal, icon: .error) }
                if let login {
                    flow(login)
                } else {
                    MatrixLoginStart(server: $server, isRetry: false, start: start)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(WontonTheme.spacing.pageInset)
        }
        .scrollDismissesKeyboard(.interactively)
        .wontonPushedScreen(String(localized: .Matrixlogin1.matrixLoginTitle), usesSystemToolbar: true)
        .onChange(of: login?.state, initial: true) { old, state in follow(state, changed: old != state) }
        .onDisappear(perform: leave)
    }

    @ViewBuilder private func flow(_ login: MatrixLoginViewModel) -> some View {
        switch login.state {
        case .cancelled, .succeeded:
            MatrixLoginStart(server: $server, isRetry: false, start: start)
        case let .failed(reason):
            ProblemNotice(problem: reason.problem(homeserver: login.homeserver), icon: .error)
            MatrixLoginStart(server: $server, isRetry: true, start: start)
        case .discovering:
            MatrixLoginProgress(text: String(localized: .Matrixlogin1.matrixLoginContacting(login.homeserver)))
        case .ready:
            MatrixLoginMethods(login: login, send: send)
        case .authenticating:
            MatrixLoginProgress(text: String(localized: .Matrixlogin1.matrixLoginSigningIn))
        case let .awaitingOidc(authorizationUrl):
            MatrixLoginProgress(text: String(localized: .Matrixlogin0.matrixLoginFinishInBrowser))
            if browser.failedToOpen {
                WontonNotice(icon: .warning, title: String(localized: .Matrixlogin0.matrixLoginBrowserFailed))
            }
            Button {
                openBrowser(login, url: authorizationUrl)
            } label: {
                Text(.Matrixlogin0.matrixLoginOpenBrowser).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle(kind: .secondary))
        case .saving:
            MatrixLoginProgress(text: String(localized: .Matrixlogin0.matrixLoginSaving))
        case .other:
            // 比这个壳新的状态：如实说走不下去，不装成进行中。
            WontonNotice(icon: .warning, title: String(localized: .Matrixlogin0.matrixLoginUnknownState))
            MatrixLoginStart(server: $server, isRetry: true, start: start)
        }
    }

    /// 授权地址一出来就打开网页登录面板；离开“等浏览器”这一步时收起它。这一页上登录成功就退回，进来时看到的
    /// 是上一次的成功（`changed` 为假）不算。
    private func follow(_ state: MatrixLoginState?, changed: Bool) {
        switch state {
        case let .awaitingOidc(authorizationUrl):
            if let login, !browser.isOpen { openBrowser(login, url: authorizationUrl) }
        case .succeeded:
            browser.close()
            if changed { onFinished() }
        default:
            browser.close()
        }
    }

    private func openBrowser(_ login: MatrixLoginViewModel, url: String) {
        browser.open(url: url, redirectURI: login.oidcRedirectUri, loginId: login.loginId, send: send)
    }

    private func start() {
        let text = server.trimmingCharacters(in: .whitespacesAndNewlines)
        send(.startMatrixLogin(homeserver: text.isEmpty ? nil : text))
    }

    private func send(_ action: Action) {
        refusal = model.matrixLogin(action)
    }

    private func leave() {
        browser.close()
        guard let login, login.state.isUnderway else { return }
        _ = model.matrixLogin(.cancelMatrixLogin(loginId: login.loginId))
    }
}

/// “用 Matrix 账号登录”的入口行，放在服务器列表和配对页的“添加服务器”旁边。
struct MatrixLoginEntryLabel: View {
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            WontonIcon.globe.view(size: WontonTheme.size.iconSmall)
            Text(.Matrixlogin0.matrixLoginEntry).wontonText(WontonTheme.typography.body)
            Spacer(minLength: 0)
        }
        .foregroundStyle(Color(colors.textPrimary))
        .padding(.horizontal, WontonTheme.spacing.s16)
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.target)
        .contentShape(Rectangle())
    }
}

/// 起手和失败后：填服务器，继续后由核心发现它支持的登录方式。
private struct MatrixLoginStart: View {
    @Binding var server: String
    let isRetry: Bool
    let start: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(.Matrixlogin0.matrixLoginServer)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
                .accessibilityHidden(true)
            TextField(String(localized: .Matrixlogin0.matrixLoginServer), text: $server)
                .accessibilityIdentifier("matrix-login-server")
                .textFieldStyle(.plain)
                .keyboardType(.URL)
                .textContentType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .submitLabel(.continue)
                .onSubmit(start)
                .wontonField()
        }
        Button(action: start) {
            Text(isRetry ? .Common.commonRetry : .Matrixlogin0.matrixLoginContinue).frame(maxWidth: .infinity)
        }
        .buttonStyle(WontonButtonStyle())
        .accessibilityIdentifier("matrix-login-start")
    }
}

/// 发现完成：登录哪台服务器是数据，列出来；方式按核心给的列，只有密码时表单直接铺开。
private struct MatrixLoginMethods: View {
    let login: MatrixLoginViewModel
    let send: (Action) -> Void
    @State private var showsPassword = false
    @State private var userId = ""
    @State private var password = ""
    @Environment(\.wontonColors) private var colors

    private var offersOidc: Bool { login.methods.contains(.oidc) }
    private var offersPassword: Bool { login.methods.contains(.password) }

    var body: some View {
        Text(login.homeserver)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.textSecondary))
        if !offersOidc && !offersPassword {
            WontonNotice(icon: .warning, title: String(localized: .Matrixlogin0.matrixLoginFailedUnsupportedMethod))
        }
        if offersOidc {
            Button {
                send(.startMatrixOidc(login: MatrixOidcLogin(
                    loginId: login.loginId, redirectUri: login.oidcRedirectUri, deviceName: UIDevice.current.name
                )))
            } label: {
                Text(.Matrixlogin0.matrixLoginMethodOidc).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle())
            .accessibilityIdentifier("matrix-login-oidc")
        }
        if offersPassword {
            if showsPassword || !offersOidc {
                passwordForm
            } else {
                Button { showsPassword = true } label: {
                    Text(.Matrixlogin1.matrixLoginMethodPassword).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
        }
    }

    private var passwordForm: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            TextField(String(localized: .Matrixlogin0.matrixLoginUserId), text: $userId)
                .textFieldStyle(.plain)
                .keyboardType(.emailAddress)
                .textContentType(.username)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .wontonField()
            SecureField(String(localized: .Matrixlogin0.matrixLoginPassword), text: $password)
                .textFieldStyle(.plain)
                .textContentType(.password)
                .submitLabel(.go)
                .onSubmit(submit)
                .wontonField()
            Button(action: submit) {
                Text(.Matrixlogin1.matrixLoginSignIn).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle())
            .disabled(userId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || password.isEmpty)
        }
    }

    private func submit() {
        let user = userId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !user.isEmpty, !password.isEmpty else { return }
        send(.submitMatrixPassword(login: MatrixPasswordLogin(
            loginId: login.loginId, userId: user, password: password, deviceName: UIDevice.current.name
        )))
        // 密码交给核心后就不留在界面状态里。
        password = ""
    }
}

private struct MatrixLoginProgress: View {
    let text: String
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            ProgressView()
            Text(text).wontonText(WontonTheme.typography.body)
        }
        .foregroundStyle(Color(colors.textPrimary))
        .accessibilityElement(children: .combine)
    }
}

extension MatrixLoginState {
    /// 还在进行、返回时要交给核心取消的。
    var isUnderway: Bool {
        switch self {
        case .discovering, .ready, .authenticating, .awaitingOidc, .saving: true
        case .succeeded, .cancelled, .failed, .other: false
        }
    }
}

extension MatrixLoginFailure {
    /// 一句人话说发生了什么；服务器和类别收进默认收起的详情。不认识的类别落到通用文案。
    func problem(homeserver: String) -> Problem {
        Problem(title: title, detail: "\(homeserver)\n\(self)")
    }

    private var title: String {
        switch self {
        case .invalidHomeserver: String(localized: .Matrixlogin0.matrixLoginFailedInvalidHomeserver)
        case .discovery: String(localized: .Matrixlogin1.matrixLoginFailedDiscovery)
        case .unsupportedMethod: String(localized: .Matrixlogin0.matrixLoginFailedUnsupportedMethod)
        case .transport: String(localized: .Matrixlogin0.matrixLoginFailedTransport)
        case .invalidCredentials: String(localized: .Matrixlogin0.matrixLoginFailedInvalidCredentials)
        case .oidcRejected: String(localized: .Matrixlogin1.matrixLoginFailedOidcRejected)
        case .invalidCallback: String(localized: .Matrixlogin0.matrixLoginFailedInvalidCallback)
        case .expired: String(localized: .Matrixlogin0.matrixLoginFailedExpired)
        case .storage: String(localized: .Matrixlogin1.matrixLoginFailedStorage)
        case .upstream: String(localized: .Matrixlogin1.matrixLoginFailedUpstream)
        case .other: String(localized: .Matrixlogin1.matrixLoginFailedOther)
        }
    }
}
