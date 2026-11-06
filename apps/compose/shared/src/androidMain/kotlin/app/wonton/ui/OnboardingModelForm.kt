package app.wonton.ui

import android.view.inputmethod.EditorInfo
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.platform.InterceptPlatformTextInput
import androidx.compose.ui.platform.LocalAutofillManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.PlatformTextInputMethodRequest
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import app.wonton.protocol.Action
import app.wonton.protocol.ModelProviderInput
import app.wonton.protocol.OnboardingModelChoice
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme

internal class OnboardingModelForm(choice: OnboardingModelChoice?) {
    var expandedProvider by mutableStateOf((choice as? OnboardingModelChoice.Provider)?.id)
        private set
    var customExpanded by mutableStateOf(choice is OnboardingModelChoice.Custom)
        private set
    var baseUrl by mutableStateOf((choice as? OnboardingModelChoice.Custom)?.baseUrl.orEmpty())
    var modelName by mutableStateOf((choice as? OnboardingModelChoice.Custom)?.model.orEmpty())
    // 只留在当前步骤的输入状态，不进 rememberSaveable 或日志。
    var apiKey by mutableStateOf("")

    val canTest: Boolean
        get() = apiKey.isNotEmpty() && if (customExpanded) {
            baseUrl.isNotEmpty() && modelName.isNotEmpty()
        } else {
            expandedProvider != null
        }

    fun expandProvider(id: String) {
        apiKey = ""
        customExpanded = false
        expandedProvider = id.takeUnless { expandedProvider == id }
    }

    fun expandCustom() {
        apiKey = ""
        expandedProvider = null
        customExpanded = !customExpanded
    }

    fun collapse() {
        apiKey = ""
        expandedProvider = null
        customExpanded = false
    }

    fun submit(send: (Action) -> Unit, thenContinue: Boolean) {
        val input = when {
            apiKey.isEmpty() -> null
            customExpanded -> ModelProviderInput.Custom(baseUrl, modelName, apiKey)
            else -> expandedProvider?.let { ModelProviderInput.Preset(it, apiKey) }
        }
        if (input != null) {
            apiKey = ""
            send(Action.TestModelProvider(input, thenContinue))
        } else if (thenContinue) {
            send(Action.ContinueOnboarding)
        }
    }
}

@Composable
internal fun ModelProviderForm(
    form: OnboardingModelForm,
    testing: Boolean,
    send: (Action) -> Unit,
    keyLink: @Composable () -> Unit = {},
) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        if (form.customExpanded) {
            WontonTextField(
                form.baseUrl,
                { form.baseUrl = it; send(Action.DismissModelProviderProblem) },
                Modifier.fillMaxWidth(),
                placeholder = "https://api.example.com/v1",
                label = stringResource(R.string.onboarding_model_form_base_url),
                enabled = !testing,
                keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, keyboardType = KeyboardType.Uri),
            )
            WontonTextField(
                form.modelName,
                { form.modelName = it; send(Action.DismissModelProviderProblem) },
                Modifier.fillMaxWidth(),
                placeholder = stringResource(R.string.onboarding_model_form_model_name),
                label = stringResource(R.string.onboarding_model_form_model_name),
                enabled = !testing,
                keyboardOptions = KeyboardOptions(autoCorrectEnabled = false),
            )
        }
        ModelKeyField(form.apiKey, { form.apiKey = it; send(Action.DismissModelProviderProblem) }, !testing)
        keyLink()
        if (testing) OnboardingLoading()
        WontonButton(
            stringResource(R.string.onboarding_model_test), { form.submit(send, thenContinue = false) },
            enabled = !testing && form.apiKey.isNotEmpty(),
            kind = WontonButtonKind.SECONDARY,
        )
    }
}

@OptIn(ExperimentalComposeUiApi::class)
@Composable
private fun ModelKeyField(value: String, onValueChange: (String) -> Unit, enabled: Boolean) {
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
                value, onValueChange,
                Modifier.fillMaxWidth().bringIntoViewRequester(visible).onFocusChanged { focused = it.isFocused },
                label = stringResource(R.string.onboarding_model_form_api_key),
                placeholder = stringResource(R.string.onboarding_model_form_api_key),
                enabled = enabled,
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.None,
                    autoCorrectEnabled = false,
                    keyboardType = KeyboardType.Password,
                ),
            )
        }
    }
}
