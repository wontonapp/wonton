@file:Suppress("MagicNumber") // 走查用的假数据：耗时这类数字就是字面量。

package app.wonton.android

import app.wonton.protocol.AgentModelsProblem
import app.wonton.protocol.AgentModelsReason
import app.wonton.protocol.AgentModelsState
import app.wonton.protocol.AgentModelsViewModel
import app.wonton.protocol.BaseUrlField
import app.wonton.protocol.BotNameProblem
import app.wonton.protocol.BotRef
import app.wonton.protocol.KeyCheck
import app.wonton.protocol.KeyCheckResult
import app.wonton.protocol.LoginState
import app.wonton.protocol.ModelChoice
import app.wonton.protocol.ModelConfirm
import app.wonton.protocol.ModelKeyForm
import app.wonton.protocol.ModelListState
import app.wonton.protocol.ModelLoginBrowser
import app.wonton.protocol.ModelLoginProgress
import app.wonton.protocol.ModelLoginState
import app.wonton.protocol.ModelOption
import app.wonton.protocol.ModelProviderPage
import app.wonton.protocol.ModelProviderRow
import app.wonton.protocol.ModelTest
import app.wonton.protocol.ModelTestResult
import app.wonton.protocol.ModelTestState
import app.wonton.protocol.OnboardingAssistantStep
import app.wonton.protocol.OnboardingCommit
import app.wonton.protocol.OnboardingModelStep
import app.wonton.protocol.OnboardingProfileStep
import app.wonton.protocol.OnboardingReview
import app.wonton.protocol.OnboardingStep
import app.wonton.protocol.OnboardingTips
import app.wonton.protocol.OnboardingViewModel
import app.wonton.protocol.StepNav

internal val PREVIEW_BOT = BotRef(pluginId = "app.example.agent", bot = "default")
internal const val SCREEN_MODELS_NONE = "models-none"

private val openRouter = ModelProviderRow("openrouter", "OpenRouter", connected = true, account = "…7f3a")
private val custom = ModelProviderRow("custom", "Custom endpoint", connected = false, account = null)

private fun row(id: String, name: String) = ModelProviderRow(id, name, connected = false, account = null)

private val recommended = listOf(
    openRouter, row("nous", "Nous Portal"), row("openai-codex", "ChatGPT"), row("xai", "xAI Grok"),
)
private val more = listOf(
    row("anthropic", "Anthropic"), row("deepseek", "DeepSeek"), row("gemini", "Google Gemini"),
    row("minimax", "MiniMax"), row("mistral", "Mistral"), custom,
)
private val choice = ModelChoice(provider = "openrouter", model = "openai/gpt-4o-mini")
private val someModels = listOf(
    ModelOption("openai/gpt-4o-mini", "GPT-4o mini", recommended = true),
    ModelOption("openai/gpt-4o", "GPT-4o", recommended = false),
    ModelOption("google/gemini-2.5-flash", "Gemini 2.5 Flash", recommended = false),
    ModelOption("deepseek/deepseek-chat", null, recommended = false),
    ModelOption("mistralai/mistral-large", "Mistral Large", recommended = false),
    ModelOption("x-ai/grok-4", "Grok 4", recommended = false),
    ModelOption("qwen/qwen3-max", "Qwen3 Max", recommended = false),
    ModelOption("meta-llama/llama-4-70b", "Llama 4 70B", recommended = false),
)

private fun problem(reason: AgentModelsReason, field: String? = null) = AgentModelsProblem(
    reason = reason,
    message = "The agent answered with an error.",
    field = field,
    detail = "exit status 1: connect to 192.0.2.10:8642 refused",
)

private fun models(
    current: ModelChoice? = choice,
    test: ModelTestState? = ModelTestState.Done(ModelTest(ModelTestResult.OK, null, 1240u)),
    search: String = "",
    provider: ModelProviderPage? = null,
    actionProblem: AgentModelsProblem? = null,
    state: AgentModelsState = AgentModelsState.Ready,
) = AgentModelsViewModel(
    bot = PREVIEW_BOT,
    state = state,
    current = current,
    test = test,
    recommended = recommended.map { if (current == null) it.copy(connected = false, account = null) else it },
    more = more.filter { it.name.contains(search, ignoreCase = true) },
    search = search,
    provider = provider,
    actionProblem = actionProblem,
)

private fun page(
    row: ModelProviderRow = openRouter.copy(connected = false, account = null),
    key: ModelKeyForm? = ModelKeyForm("https://example.com/keys", "sk-or-", baseUrl = null),
    deviceLogin: Boolean = false,
    login: ModelLoginProgress? = null,
    loginOutcome: ModelLoginState? = null,
    keyCheck: KeyCheck? = null,
    models: ModelListState = ModelListState.NotConnected,
    confirm: ModelConfirm? = null,
) = ModelProviderPage(
    row = row, key = key, deviceLogin = deviceLogin, browserLogin = false, login = login,
    loginOutcome = loginOutcome, busy = false, keyCheck = keyCheck, models = models, confirm = confirm,
)

private val nous = row("nous", "Nous Portal")
private val codeLogin = ModelLoginProgress("https://example.com/device", "WXYZ-1234", ModelLoginBrowser.PAGE)

/** 每个名字一种状态；不认识的名字是“接好了、测通了”的列表。 */
internal fun previewModels(screen: String): AgentModelsViewModel = when (screen) {
    SCREEN_MODELS_NONE -> models(current = null, test = null)
    "models-more" -> models(search = "mi")
    "models-test-rejected" -> models(
        test = ModelTestState.Done(ModelTest(ModelTestResult.REJECTED, "401 Unauthorized: invalid api key", null)),
    )
    "models-testing" -> models(test = ModelTestState.Running)
    "models-action-problem" -> models(test = null, actionProblem = problem(AgentModelsReason.UPSTREAM))
    "models-loading" -> models(state = AgentModelsState.Loading)
    "models-failed" -> models(state = AgentModelsState.Failed(problem(AgentModelsReason.NOT_READY)))
    "key" -> models(current = null, test = null, provider = page())
    "key-rejected" -> models(
        current = null, test = null,
        provider = page(keyCheck = KeyCheck(KeyCheckResult.REJECTED, "401 Unauthorized")),
    )
    "key-custom" -> models(
        current = null, test = null,
        provider = page(
            row = custom,
            key = ModelKeyForm(null, null, BaseUrlField(required = true, default = null)),
            keyCheck = KeyCheck(KeyCheckResult.UNVERIFIED, null),
        ),
        actionProblem = problem(AgentModelsReason.INVALID_INPUT, field = "base_url"),
    )
    "login" -> models(current = null, test = null, provider = page(row = nous, key = null, deviceLogin = true))
    "login-code" -> models(
        current = null, test = null,
        provider = page(row = nous, key = null, deviceLogin = true, login = codeLogin),
    )
    "login-denied" -> models(
        current = null, test = null,
        provider = page(
            row = nous, key = null, deviceLogin = true,
            loginOutcome = ModelLoginState(LoginState.DENIED, "access_denied"),
        ),
    )
    "provider-models" -> models(provider = page(row = openRouter, models = ModelListState.Ready(someModels)))
    "provider-models-loading" -> models(provider = page(row = openRouter, models = ModelListState.Loading))
    "provider-confirm" -> models(
        provider = page(
            row = openRouter, models = ModelListState.Ready(someModels),
            confirm = ModelConfirm(
                "openai/gpt-4o", "This model costs about 10x more per message than the recommended one.",
            ),
        ),
    )
    else -> models()
}

/** 引导的一步：只填这一步用到的字段。 */
internal fun previewOnboarding(step: OnboardingStep, position: UInt, blocked: Boolean) = OnboardingViewModel(
    step = step,
    nav = StepNav(
        position = position, total = 5u, canBack = true, canSkip = true, canNext = !blocked, canClose = false,
    ),
    loading = false,
    problem = null,
    catalogProblem = null,
    profile = OnboardingProfileStep("Ada Lovelace", "", null, null),
    assistant = OnboardingAssistantStep(
        name = if (blocked && step == OnboardingStep.ASSISTANT) "" else "Wonton",
        nameProblem = BotNameProblem.EMPTY.takeIf { blocked && step == OnboardingStep.ASSISTANT },
    ),
    model = OnboardingModelStep(PREVIEW_BOT),
    needs = emptyList(),
    review = OnboardingReview(emptyList(), OnboardingCommit.NOT_STARTED),
    defaultBotUserId = null,
    defaultBotVisibility = null,
    chatPrompts = emptyList(),
    tips = OnboardingTips(confirmationCard = false, appInstalled = false),
    tooOld = null,
)
