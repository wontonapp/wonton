#if DEBUG
import SwiftUI
import WontonClient

/// 模型页和引导新步骤的模拟器走查入口（ADR-0129）：启动参数带 `wonton-agent-models-preview <状态>` 时渲染那一屏
/// （同 `ProblemPreview` 的做法）。ViewModel 是这里造的假数据，点了不会有反应。
enum AgentModelsPreview {
    private static let flag = "wonton-agent-models-preview"

    static var active: Bool { ProcessInfo.processInfo.arguments.contains(flag) }

    private static var shot: String {
        let arguments = ProcessInfo.processInfo.arguments
        guard let index = arguments.firstIndex(of: flag), arguments.indices.contains(index + 1) else { return "list" }
        return arguments[index + 1]
    }

    @MainActor
    @ViewBuilder static func root(core: ClientCore) -> some View {
        switch shot {
        case "assistant":
            frame(step: 2, title: .Onboarding3.onboardingStepAssistant) {
                OnboardingAssistantStepView(
                    assistant: OnboardingAssistantStep(name: "Wonton", nameProblem: nil), send: { _ in }
                )
            }
        case "assistant-problem":
            frame(step: 2, title: .Onboarding3.onboardingStepAssistant) {
                OnboardingAssistantStepView(
                    assistant: OnboardingAssistantStep(name: "", nameProblem: .empty), send: { _ in }
                )
            }
        case "review", "review-failed", "review-none":
            frame(step: 5, title: .Onboarding3.onboardingStepReview, primary: .Onboarding3.onboardingDone) {
                OnboardingReviewStep(
                    review: AgentModelsFixtures.review(shot), plugins: PluginsModel(core: core)
                ) { _ in }
            }
        default:
            frame(step: 3, title: .Onboarding1.onboardingStepModel) {
                AgentModelsPage(view: AgentModelsFixtures.view(shot), send: { _ in })
            }
        }
    }

    @MainActor
    private static func frame<Content: View>(
        step: UInt32, title: LocalizedStringResource, primary: LocalizedStringResource = .Onboarding2.onboardingNext,
        @ViewBuilder content: () -> Content
    ) -> some View {
        OnboardingPreviewFrame(
            step: step, title: String(localized: title), primaryTitle: String(localized: primary), content: content
        )
    }
}

/// 走查用的假数据：服务商和模型都是编的，地址用 example.com。
private enum AgentModelsFixtures {
    /// 插件和服务商给的原文：夹具的字面量不落在界面文字的参数位上（界面文字检查的规则二）。
    static let detail0 = "401 Unauthorized: invalid api key"
    static let detail1 = "base_url must be an http(s) URL"
    static let detail2 = "401 Unauthorized"
    static let detail3 = "device code expired after 900 s"
    static let detail4 = "Model Max costs about 15× more than Model Large. Use it anyway?"

    static let bot = BotRef(pluginId: "app.example.agent", bot: "default")

    static let recommended = [
        row("openrouter", "OpenRouter", connected: true, account: "sk-or-…a1b2"),
        row("nous", "Nous Portal"),
        row("openai-codex", "ChatGPT"),
        row("anthropic", "Anthropic")
    ]

    static let more = [
        row("deepseek", "DeepSeek"),
        row("gemini", "Google Gemini"),
        row("minimax", "MiniMax"),
        row("xai", "xAI Grok"),
        row("custom", "OpenAI-compatible")
    ]

    static let models = [
        ModelOption(id: "example/model-large", name: "Model Large", recommended: true),
        ModelOption(id: "example/model-small", name: "Model Small", recommended: false),
        ModelOption(id: "example/model-reasoning", name: nil, recommended: false),
        ModelOption(id: "example/model-max", name: "Model Max", recommended: false)
    ]

    static let current = ModelChoice(provider: "openrouter", model: "example/model-large")

    static func row(_ id: String, _ name: String, connected: Bool = false, account: String? = nil) -> ModelProviderRow {
        ModelProviderRow(id: id, name: name, connected: connected, account: account)
    }

    static func view(_ shot: String) -> AgentModelsViewModel {
        var view = AgentModelsViewModel(
            bot: bot, state: .ready, current: current,
            test: .done(result: ModelTest(result: .ok, message: nil, latencyMs: 840)),
            recommended: recommended, more: more, search: "", provider: nil, actionProblem: nil
        )
        switch shot {
        case "list":
            break
        case "loading":
            view.state = .loading
        case "failed":
            view.state = .failed(problem: problem(.notReady, "exit 4: dashboard not listening on 127.0.0.1:9119"))
        case "empty":
            view.current = nil
            view.test = nil
            view.recommended = recommended.map { row($0.id, $0.name) }
        case "search":
            view.search = "mi"
            view.more = more.filter { $0.name.lowercased().contains("mi") }
        case "test-running":
            view.test = .running
        case "test-rejected":
            view.test = .done(result: ModelTest(
                result: .rejected, message: detail0, latencyMs: nil
            ))
        case "action-problem":
            view.actionProblem = problem(.upstream, "exit 1: Traceback (most recent call last) …")
        default:
            view.provider = page(shot)
            if shot == "key-invalid" {
                view.actionProblem = AgentModelsProblem(
                    reason: .invalidInput, message: detail1, field: "base_url", detail: nil
                )
            }
        }
        return view
    }

    private static func problem(_ reason: AgentModelsReason, _ message: String) -> AgentModelsProblem {
        AgentModelsProblem(reason: reason, message: message, field: nil, detail: nil)
    }

    private static func page(_ shot: String) -> ModelProviderPage {
        let keyForm = ModelKeyForm(keyUrl: "https://example.com/keys", keyPrefix: "sk-ex-", baseUrl: nil)
        var page = ModelProviderPage(
            row: row("deepseek", "DeepSeek"), key: keyForm, deviceLogin: false, browserLogin: false, login: nil,
            loginOutcome: nil, busy: false, keyCheck: nil, models: .notConnected, confirm: nil
        )
        switch shot {
        case "key-rejected":
            page.keyCheck = KeyCheck(check: .rejected, message: detail2)
        case "key-invalid":
            page.row = row("custom", "OpenAI-compatible")
            page.key = ModelKeyForm(
                keyUrl: nil, keyPrefix: nil, baseUrl: BaseUrlField(required: true, default: "example.com/v1")
            )
        case "login":
            page.row = row("nous", "Nous Portal")
            page.deviceLogin = true
        case "login-code":
            page.row = row("openai-codex", "ChatGPT")
            page.key = nil
            page.deviceLogin = true
            page.login = ModelLoginProgress(url: "https://example.com/device", code: "WXYZ-1234", browser: .page)
        case "login-expired":
            page.row = row("openai-codex", "ChatGPT")
            page.key = nil
            page.deviceLogin = true
            page.loginOutcome = ModelLoginState(state: .expired, message: detail3)
        case "models-loading":
            connect(&page)
            page.models = .loading
        case "models-failed":
            connect(&page)
            page.models = .failed(problem: problem(.upstream, "exit 1: GET /api/model/options 500"))
        case "unverified":
            connect(&page)
            page.keyCheck = KeyCheck(check: .unverified, message: nil)
        case "confirm":
            connect(&page)
            page.confirm = ModelConfirm(
                model: "example/model-max",
                message: detail4
            )
        default:
            connect(&page)
        }
        return page
    }

    private static func connect(_ page: inout ModelProviderPage) {
        page.row = recommended[0]
        page.models = .ready(models: models)
    }

    static func review(_ shot: String) -> OnboardingReview {
        let model = switch shot {
        case "review-failed": item("model", .model, ["DeepSeek", "example/model-small"], .model(test: .rejected))
        case "review-none": item("model", .model, [], .unchanged)
        default: item("model", .model, ["OpenRouter", "example/model-large"], .model(test: .ok))
        }
        return OnboardingReview(items: [
            item("profile", .profile, ["Ada Lovelace"], .email(email: "ada@example.com")),
            item("assistant", .assistant, ["Wonton"], nil),
            model,
            item("untested", .model, ["Nous Portal", "example/model-reasoning"], .model(test: nil))
        ], commit: .notStarted)
    }

    private static func item(
        _ id: String, _ kind: OnboardingReviewKind, _ titles: [String], _ detail: OnboardingReviewDetail?
    ) -> OnboardingReviewItem {
        OnboardingReviewItem(
            id: id, kind: kind, titles: titles, detail: detail, icon: nil, permissions: [], notices: [], state: .ready
        )
    }
}
#endif
