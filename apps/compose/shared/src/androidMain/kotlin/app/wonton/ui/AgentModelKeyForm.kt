package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import app.wonton.protocol.AgentModelsAction
import app.wonton.protocol.AgentModelsProblem
import app.wonton.protocol.KeyCheck
import app.wonton.protocol.KeyCheckResult
import app.wonton.protocol.ModelKeyForm
import app.wonton.protocol.ModelProviderPage
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Eye
import app.wonton.ui.icons.Lock
import app.wonton.ui.icons.WontonIcon

/**
 * 填 key：密码框、要填服务地址时的地址框、“去拿 key”和保存。填的值只在这一页的内存里，存好就清空；
 * 框里有值时这一屏不进最近任务的快照。[fieldProblem] 是核心说哪一项填得不对。
 */
@Composable
internal fun ModelKeySection(
    page: ModelProviderPage,
    form: ModelKeyForm,
    fieldProblem: AgentModelsProblem?,
    send: (AgentModelsAction) -> Unit,
    browser: ModelBrowser,
) {
    var key by remember { mutableStateOf("") }
    val baseUrlField = form.baseUrl
    var baseUrl by remember { mutableStateOf("") }
    val check = page.keyCheck
    LaunchedEffect(check) {
        if (check?.check == KeyCheckResult.ACCEPTED || check?.check == KeyCheckResult.UNVERIFIED) key = ""
    }
    if (key.isNotEmpty()) SecureScreen()
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        ModelKeyField(key, enabled = !page.busy) { key = it }
        fieldProblem?.takeIf { it.field == MODEL_FIELD_KEY }?.let { FieldProblem(it) }
        ClipboardKeyOffer(form.keyPrefix, key) { key = it }
        if (baseUrlField != null) {
            WontonTextField(
                baseUrl, { baseUrl = it }, Modifier.fillMaxWidth(),
                placeholder = baseUrlField.default?.takeIf { it.isNotBlank() }
                    ?: stringResource(R.string.agent_models_base_url),
                label = stringResource(R.string.agent_models_base_url),
                enabled = !page.busy,
                keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, keyboardType = KeyboardType.Uri),
            )
            fieldProblem?.takeIf { it.field == MODEL_FIELD_BASE_URL }?.let { FieldProblem(it) }
        }
        check?.let { KeyCheckLine(it) }
        val address = baseUrl.trim().ifEmpty { baseUrlField?.default?.trim().orEmpty() }
        val complete = key.isNotBlank() && (baseUrlField?.required != true || address.isNotEmpty())
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
            form.keyUrl?.let { url ->
                WontonButton(
                    stringResource(R.string.agent_models_key_get), { browser.open(url) },
                    Modifier.weight(1f), kind = WontonButtonKind.SECONDARY,
                )
            }
            WontonButton(
                stringResource(R.string.common_save),
                {
                    val url = address.takeIf { baseUrlField != null && it.isNotEmpty() }
                    send(AgentModelsAction.SaveModelKey(key.trim(), url))
                },
                Modifier.weight(1f),
                enabled = complete && !page.busy,
            )
        }
    }
}

/** 存 key 的结果：被拒停在这页说 key 不对；没法提前验证的照存并说明；认了的不用说，上面已经是“已接好”。 */
@Composable
private fun KeyCheckLine(check: KeyCheck) {
    val colors = WontonTheme.colors
    val (text, tint) = when (check.check) {
        KeyCheckResult.REJECTED -> R.string.agent_models_key_rejected to colors.danger
        KeyCheckResult.UNVERIFIED -> R.string.agent_models_key_unverified to colors.textSecondary
        else -> return
    }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        Text(stringResource(text), style = WontonTheme.typography.supporting, color = tint.toColor())
        check.message?.takeIf { it.isNotBlank() }?.let { ProblemDetail(it) }
    }
}

@Composable
private fun FieldProblem(problem: AgentModelsProblem) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        Text(
            stringResource(R.string.agent_models_field_invalid),
            style = WontonTheme.typography.supporting, color = WontonTheme.colors.danger.toColor(),
        )
        problem.message.takeIf { it.isNotBlank() }?.let { ProblemDetail(it) }
    }
}

/** 密码框：默认打码，右边是显示按钮；不进自动更正和输入法的学习。 */
@Composable
private fun ModelKeyField(value: String, enabled: Boolean, onValueChange: (String) -> Unit) {
    val colors = WontonTheme.colors
    var shown by remember { mutableStateOf(false) }
    val label = stringResource(R.string.agent_models_key)
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BasicTextField(
            value = value,
            onValueChange = onValueChange,
            enabled = enabled,
            singleLine = true,
            textStyle = WontonTheme.typography.body.copy(color = colors.textPrimary.toColor()),
            cursorBrush = SolidColor(colors.textPrimary.toColor()),
            keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, keyboardType = KeyboardType.Password),
            visualTransformation = if (shown) VisualTransformation.None else PasswordVisualTransformation(),
            modifier = Modifier.weight(1f).semantics { contentDescription = label },
            decorationBox = { field ->
                Box(
                    Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
                        .background(colors.surfaceSubtle.toColor(), RoundedCornerShape(WontonTheme.radii.control))
                        .padding(WontonTheme.spacing.s12),
                    contentAlignment = Alignment.CenterStart,
                ) {
                    if (value.isEmpty()) {
                        Text(label, style = WontonTheme.typography.body, color = colors.textSecondary.toColor())
                    }
                    field()
                }
            },
        )
        val showLabel = stringResource(if (shown) R.string.secret_hide else R.string.secret_show)
        Box(
            Modifier.size(WontonTheme.size.target)
                .clickable(role = Role.Button) { shown = !shown }
                .semantics { contentDescription = showLabel },
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                if (shown) WontonIcon.Lock else WontonIcon.Eye, null,
                Modifier.size(WontonTheme.size.iconSmall), tint = colors.textSecondary.toColor(),
            )
        }
    }
}
