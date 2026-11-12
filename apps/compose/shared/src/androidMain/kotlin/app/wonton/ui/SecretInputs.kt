package app.wonton.ui

import android.view.WindowManager
import androidx.activity.compose.LocalActivity
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
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.protocol.SecretInput
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Eye
import app.wonton.ui.icons.Lock
import app.wonton.ui.icons.WontonIcon

/**
 * 这一屏有密钥输入时开 FLAG_SECURE，离开就关：最近任务和后台快照里看不到填过的值。
 * 卡片在聊天里，只在框里有值或点过“显示”、还没答完时开，整页聊天不常开。
 */
@Composable
fun SecureScreen() {
    val activity = LocalActivity.current ?: return
    DisposableEffect(activity) {
        activity.window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        onDispose { activity.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE) }
    }
}

/** 安装、卡片的密钥一节：每项一个输入框，标题、说明和“去哪里拿”都由核心给好。 */
@Composable
fun SecretInputsSection(
    inputs: List<SecretInput>,
    values: Map<String, String>,
    enabled: Boolean = true,
    onShownChange: (Boolean) -> Unit = {},
    onChange: (name: String, value: String) -> Unit,
) {
    if (inputs.isEmpty()) return
    var shown by remember(inputs) { mutableStateOf<Set<String>>(emptySet()) }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        inputs.forEach { input ->
            SecretInputField(input, values[input.name].orEmpty(), enabled, onShownChange = { on ->
                shown = if (on) shown + input.name else shown - input.name
                onShownChange(shown.isNotEmpty())
            }) { onChange(input.name, it) }
        }
    }
}

/** 一项密钥的输入框：值默认打码，右边是显示按钮；标题下面写说明和链接。 */
@Composable
fun SecretInputField(
    input: SecretInput,
    value: String,
    enabled: Boolean = true,
    onShownChange: (Boolean) -> Unit = {},
    onValueChange: (String) -> Unit,
) {
    val colors = WontonTheme.colors
    var shown by remember(input.name) { mutableStateOf(false) }
    val uri = LocalUriHandler.current
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                input.title,
                Modifier.weight(1f),
                style = WontonTheme.typography.body,
                color = colors.textPrimary.toColor(),
            )
            if (!input.required) {
                Text(
                    stringResource(R.string.secret_optional),
                    style = WontonTheme.typography.caption,
                    color = colors.textSecondary.toColor(),
                )
            }
        }
        input.description?.let {
            Text(it, style = WontonTheme.typography.caption, color = colors.textSecondary.toColor())
        }
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
                cursorBrush = androidx.compose.ui.graphics.SolidColor(colors.textPrimary.toColor()),
                // 关掉自动更正和拼写检查；密码类型让系统不弹自动填充建议。
                keyboardOptions = KeyboardOptions(
                    autoCorrectEnabled = false,
                    keyboardType = androidx.compose.ui.text.input.KeyboardType.Password,
                ),
                keyboardActions = KeyboardActions.Default,
                visualTransformation = if (shown) VisualTransformation.None else PasswordVisualTransformation(),
                modifier = Modifier
                    .weight(1f)
                    .semantics { contentDescription = input.title },
                decorationBox = { field ->
                    Box(
                        Modifier.fillMaxWidth()
                            .heightIn(min = WontonTheme.size.target)
                            .background(colors.surfaceSubtle.toColor(), RoundedCornerShape(WontonTheme.radii.control))
                            .padding(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s12),
                        contentAlignment = Alignment.CenterStart,
                    ) { field() }
                },
            )
            val showLabel = stringResource(if (shown) R.string.secret_hide else R.string.secret_show)
            Box(
                Modifier.size(WontonTheme.size.target)
                    .clickable(enabled = enabled, role = androidx.compose.ui.semantics.Role.Button) {
                        shown = !shown
                        onShownChange(shown)
                    }
                    .semantics { contentDescription = showLabel },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    if (shown) WontonIcon.Lock else WontonIcon.Eye,
                    null,
                    Modifier.size(WontonTheme.size.iconSmall),
                    tint = colors.textSecondary.toColor(),
                )
            }
        }
        input.url?.let { url ->
            Text(
                stringResource(R.string.secret_get_one),
                Modifier
                    .clickable(role = androidx.compose.ui.semantics.Role.Button) { uri.openUri(url) }
                    .semantics { contentDescription = url },
                style = WontonTheme.typography.caption,
                color = colors.accentInk.toColor(),
            )
        }
    }
}

/** 请求里带的密钥：只带填了的，可选项没填就不发。 */
fun secretValues(inputs: List<SecretInput>, values: Map<String, String>): Map<String, String> =
    inputs.mapNotNull { input ->
        values[input.name]?.trim()?.takeIf { it.isNotEmpty() }?.let { input.name to it }
    }.toMap()

/** 还有必填项没填：确认按钮不可点。 */
fun missingRequiredSecret(inputs: List<SecretInput>, values: Map<String, String>): Boolean =
    inputs.any { it.required && values[it.name]?.isBlank() != false }

/** 详情里“改这一项”的弹窗标题用：标题过长时截断交给调用方。 */
@Composable
fun SecretTitle(input: SecretInput): String = input.title

/** 详情里的一项状态文字；值本身不显示。 */
@Composable
fun secretStateText(input: app.wonton.protocol.SecretInputState, changed: String?): String =
    if (input.filled) {
        stringResource(R.string.secret_filled) + (changed?.let { " · $it" } ?: "")
    } else {
        stringResource(R.string.secret_unfilled)
    }

/** 小字说明在卡片里也最多两行。 */
@Composable
fun SecretDescription(text: String) {
    Text(
        text,
        style = WontonTheme.typography.caption,
        color = WontonTheme.colors.textSecondary.toColor(),
        maxLines = 2,
        overflow = TextOverflow.Ellipsis,
    )
}
