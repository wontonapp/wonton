package app.wonton.ui

import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.view.inputmethod.EditorInfo
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.platform.InterceptPlatformTextInput
import androidx.compose.ui.platform.LocalAutofillManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.PlatformTextInputMethodRequest
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import app.wonton.protocol.Action
import app.wonton.protocol.ModelProvider
import app.wonton.protocol.ModelProviderFailure
import app.wonton.protocol.ModelProviderInput
import app.wonton.protocol.ModelProviderProblem
import app.wonton.ui.design.StepOptionRow
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

@Composable
fun OnboardingModelStep(
    model: app.wonton.protocol.OnboardingModelStep,
    send: (Action) -> Unit,
    openGateway: (String) -> Unit,
) {
    var expandedProvider by remember { mutableStateOf<String?>(null) }
    var showsMore by remember { mutableStateOf(false) }
    val openKeyUrl = rememberKeyBrowser(send)
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        model.problem?.let { problem ->
            key(problem) { ModelProblem(problem, send) }
        }
        if (model.providers == null && model.problem == null) ModelSpinner()
        model.providers?.let { providers ->
            val items = providers.providers.orEmpty()
            items.forEach { provider ->
                if (provider.recommended || showsMore) {
                    key(provider.id) {
                        ModelProviderRow(
                            provider = provider,
                            expanded = expandedProvider == provider.id,
                            saving = model.testing,
                            onExpand = {
                                expandedProvider = if (expandedProvider == provider.id) null else provider.id
                            },
                            send = send,
                            openKeyUrl = openKeyUrl,
                        )
                    }
                }
            }
            if (items.any { !it.recommended }) {
                WontonButton(
                    if (showsMore) "收起服务商" else "更多服务商",
                    { showsMore = !showsMore },
                    kind = WontonButtonKind.SECONDARY,
                )
            }
            if (items.isEmpty()) {
                providers.webUrl?.let { url ->
                    WontonButton("打开模型设置", { openGateway(url) }, kind = WontonButtonKind.SECONDARY)
                }
                WontonButton("重试", { send(Action.RefreshOnboarding) }, kind = WontonButtonKind.SECONDARY)
            }
        }
        model.logins.forEach { login ->
            key(login.id) {
                StepOptionRow(
                    title = "用${login.title}",
                    selected = false,
                    onSelect = { send(Action.ContinueOnboarding) },
                )
            }
        }
    }
}

@Composable
private fun ModelProviderRow(
    provider: ModelProvider,
    expanded: Boolean,
    saving: Boolean,
    onExpand: () -> Unit,
    send: (Action) -> Unit,
    openKeyUrl: (String) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        StepOptionRow(
            title = provider.title,
            selected = provider.configured,
            onSelect = onExpand,
            trailing = {
                if (saving && expanded) ModelSpinner()
            },
        )
        if (expanded) {
            ModelKeyInput(provider.id, saving, send)
            provider.keyUrl?.let { url ->
                WontonButton(
                    "获取 API key",
                    { openKeyUrl(url) },
                    kind = WontonButtonKind.SECONDARY,
                )
            }
        }
    }
}

@OptIn(ExperimentalComposeUiApi::class)
@Composable
private fun ModelKeyInput(id: String, saving: Boolean, send: (Action) -> Unit) {
    // 不进可恢复状态；收起、换服务商或离开这一步就丢弃 key。
    var apiKey by remember(id) { mutableStateOf("") }
    var focused by remember { mutableStateOf(false) }
    val visible = remember { BringIntoViewRequester() }
    val keyboardBottom = WindowInsets.ime.getBottom(LocalDensity.current)
    LaunchedEffect(focused, keyboardBottom) {
        if (focused && keyboardBottom > 0) visible.bringIntoView()
    }
    val autofill = LocalAutofillManager.current
    DisposableEffect(autofill) {
        autofill?.cancel()
        onDispose { autofill?.cancel() }
    }
    Column(
        Modifier.fillMaxWidth().bringIntoViewRequester(visible),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        CompositionLocalProvider(LocalAutofillManager provides null) {
            InterceptPlatformTextInput(
                interceptor = { request, nextHandler ->
                    nextHandler.startInputMethod(
                        PlatformTextInputMethodRequest { info ->
                            request.createInputConnection(info).also {
                                info.imeOptions = info.imeOptions or EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
                            }
                        },
                    )
                },
            ) {
                WontonTextField(
                    value = apiKey,
                    onValueChange = {
                        apiKey = it
                        send(Action.DismissModelProviderProblem)
                    },
                    modifier = Modifier.fillMaxWidth().onFocusChanged { focused = it.isFocused },
                    label = "API key",
                    placeholder = "API key",
                    enabled = !saving,
                    visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(
                        capitalization = KeyboardCapitalization.None,
                        autoCorrectEnabled = false,
                        keyboardType = KeyboardType.Password,
                    ),
                )
            }
        }
        WontonButton(
            "保存 key",
            {
                val action = Action.TestModelProvider(ModelProviderInput.Preset(id, apiKey), true)
                apiKey = ""
                autofill?.cancel()
                send(action)
            },
            enabled = !saving && apiKey.isNotEmpty(),
        )
    }
}

@Composable
private fun ModelProblem(problem: ModelProviderProblem, send: (Action) -> Unit) {
    val text = when (problem.reason) {
        ModelProviderFailure.KEY_REJECTED -> "服务商拒绝了这个 key，没有保留。请修改后重新保存。"
        ModelProviderFailure.PROVIDER_UNREACHABLE -> "key 已保存，但暂时连不上服务商。你可以继续下一步。"
        ModelProviderFailure.GATEWAY_NOT_READY -> "模型网关还没就绪，请重试或跳过这一步。"
        ModelProviderFailure.TRANSPORT_UNAVAILABLE -> "连不上家里的服务器，请检查网络后重试。"
        ModelProviderFailure.UNAUTHORIZED -> "家里的服务器没有授权这台设备，请检查访问权限后重试。"
        ModelProviderFailure.UPSTREAM -> "家里的服务器没能完成请求，请稍后重试。"
        ModelProviderFailure.PROTOCOL -> "App 和家里服务器的版本不兼容，请更新后重试。"
    }
    WontonNotice(WontonIcon.Warning, text, actions = {
        if (problem.message.isNotEmpty()) {
            ProblemDetail(problem.message)
        }
        if (problem.reason != ModelProviderFailure.KEY_REJECTED) {
            WontonButton(
                "重试",
                { send(Action.RefreshOnboarding) },
                kind = WontonButtonKind.SECONDARY,
            )
        }
    })
}

@Composable
private fun ModelSpinner() {
    CircularProgressIndicator(
        modifier = Modifier.size(WontonTheme.size.icon),
        color = WontonTheme.colors.textPrimary.toColor(),
        strokeWidth = WontonTheme.size.hairline,
    )
}

@Composable
private fun rememberKeyBrowser(send: (Action) -> Unit): (String) -> Unit {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val currentSend by rememberUpdatedState(send)
    var browserOpened by remember { mutableStateOf(false) }
    var browserUnavailable by remember { mutableStateOf(false) }
    DisposableEffect(lifecycle) {
        var leftApp = false
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_PAUSE && browserOpened) leftApp = true
            if (event == Lifecycle.Event.ON_RESUME && browserOpened && leftApp) {
                browserOpened = false
                leftApp = false
                currentSend(Action.RefreshOnboarding)
            }
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    if (browserUnavailable) {
        WontonNotice(WontonIcon.Warning, "没有浏览器能打开这个链接，请安装或启用浏览器后重试。")
    }
    return { url ->
        try {
            context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
            browserOpened = true
            browserUnavailable = false
        } catch (_: ActivityNotFoundException) {
            browserUnavailable = true
        }
    }
}
