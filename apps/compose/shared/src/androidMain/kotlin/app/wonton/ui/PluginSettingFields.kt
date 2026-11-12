package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Checkbox
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.input.KeyboardType
import app.wonton.protocol.PluginSettingField
import app.wonton.protocol.PluginSettingKind
import app.wonton.protocol.PluginSettingOption
import app.wonton.protocol.SettingValue
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonRadioRow
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

private const val DISABLED_ALPHA = 0.4f

/** 一项设置：控件和控件旁的问题。 */
@Composable
internal fun SettingsField(
    field: PluginSettingField,
    value: SettingValue?,
    draft: String,
    disabled: Boolean,
    missing: Boolean,
    onEdit: (name: String, value: SettingValue?) -> Unit,
    onDraft: (field: PluginSettingField, draft: String) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        when (val kind = field.kind) {
            // 比这个 App 新的控件画不出来：照常显示标题，问题行里说明要更新 App。
            is PluginSettingKind.Other -> Text(
                field.title,
                style = WontonTheme.typography.body,
                color = WontonTheme.colors.textPrimary.toColor(),
            )
            is PluginSettingKind.Toggle -> SettingsToggle(field, value, disabled, onEdit)
            is PluginSettingKind.Choice ->
                if (kind.multiple) SettingsMultiChoice(field, kind, value, disabled, onEdit)
                else SettingsSingleChoice(field, kind, value, disabled, onEdit)
            else -> SettingsTextInput(field, draft, disabled, onDraft)
        }
        SettingsFieldProblem(field, disabled, missing)
    }
}

/** 控件旁的问题：改不了的说明要更新 App；核心标出的问题和必填缺值标红。 */
@Composable
private fun SettingsFieldProblem(field: PluginSettingField, disabled: Boolean, missing: Boolean) {
    val colors = WontonTheme.colors
    if (disabled) {
        Text(
            stringResource(R.string.plugin_settings_update_app),
            style = WontonTheme.typography.caption,
            color = colors.warning.toColor(),
        )
    }
    field.problem?.let {
        Text(it.message, style = WontonTheme.typography.caption, color = colors.danger.toColor())
    }
    if (missing) {
        Text(
            stringResource(R.string.plugin_settings_required),
            style = WontonTheme.typography.caption,
            color = colors.danger.toColor(),
        )
    }
}

/** 开关：一行放标题和开关。 */
@Composable
private fun SettingsToggle(
    field: PluginSettingField,
    value: SettingValue?,
    disabled: Boolean,
    onEdit: (name: String, value: SettingValue?) -> Unit,
) {
    val checked = (value as? SettingValue.Flag)?.value ?: false
    WontonGroup {
        Row(
            Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
                .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            Text(
                field.title,
                Modifier.weight(1f),
                style = WontonTheme.typography.body,
                color = WontonTheme.colors.textPrimary.toColor(),
            )
            WontonSwitch(checked, { onEdit(field.name, SettingValue.Flag(it)) }, enabled = !disabled)
        }
    }
}

/** 文本输入（整数、小数、文本）：标题和说明在上，输入框在下。 */
@Composable
private fun SettingsTextInput(
    field: PluginSettingField,
    draft: String,
    disabled: Boolean,
    onDraft: (field: PluginSettingField, draft: String) -> Unit,
) {
    val keyboardType = when (field.kind) {
        is PluginSettingKind.Integer -> KeyboardType.Number
        is PluginSettingKind.Decimal -> KeyboardType.Decimal
        else -> KeyboardType.Text
    }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        Text(field.title, style = WontonTheme.typography.body, color = WontonTheme.colors.textPrimary.toColor())
        field.description?.let {
            Text(it, style = WontonTheme.typography.caption, color = WontonTheme.colors.textSecondary.toColor())
        }
        WontonTextField(
            draft,
            { text -> onDraft(field, text.take(draftLimit(field))) },
            Modifier.fillMaxWidth(),
            enabled = !disabled,
            keyboardOptions = KeyboardOptions(keyboardType = keyboardType),
        )
    }
}

/** 单选：一组单选项，选中哪项值就是哪项。 */
@Composable
private fun SettingsSingleChoice(
    field: PluginSettingField,
    kind: PluginSettingKind.Choice,
    value: SettingValue?,
    disabled: Boolean,
    onEdit: (name: String, value: SettingValue?) -> Unit,
) {
    SettingsChoiceTitles(field)
    WontonGroup {
        Column(
            Modifier.padding(WontonTheme.spacing.s12),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        ) {
            kind.options.forEach { option ->
                WontonRadioRow(
                    option.title,
                    selected = option.value == value,
                    enabled = !disabled,
                    onSelect = { onEdit(field.name, option.value) },
                )
            }
        }
    }
}

/** 多选：一组勾选项，值是选中的那几项（核心按字符串数组收）。 */
@Composable
private fun SettingsMultiChoice(
    field: PluginSettingField,
    kind: PluginSettingKind.Choice,
    value: SettingValue?,
    disabled: Boolean,
    onEdit: (name: String, value: SettingValue?) -> Unit,
) {
    SettingsChoiceTitles(field)
    val selected = (value as? SettingValue.Texts)?.value?.toSet() ?: emptySet()
    WontonGroup {
        Column(
            Modifier.padding(WontonTheme.spacing.s12),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        ) {
            kind.options.forEach { option ->
                val text = optionText(option)
                if (text == null) {
                    Text(
                        option.title,
                        Modifier.alpha(DISABLED_ALPHA),
                        style = WontonTheme.typography.body,
                        color = WontonTheme.colors.textPrimary.toColor(),
                    )
                } else {
                    MultiChoiceRow(
                        option.title,
                        checked = text in selected,
                        disabled = disabled,
                        onToggle = { on -> onEdit(field.name, toggleOption(option, on, kind, selected)) },
                    )
                }
            }
        }
    }
}

/** 一个勾选行：文字和勾选框都算命中区。 */
@Composable
private fun MultiChoiceRow(title: String, checked: Boolean, disabled: Boolean, onToggle: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .selectable(selected = checked, enabled = !disabled, role = Role.Checkbox) { onToggle(!checked) },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            title,
            Modifier.weight(1f),
            style = WontonTheme.typography.body,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        Checkbox(checked = checked, onCheckedChange = onToggle, enabled = !disabled)
    }
}

/** 勾选一项后的值：按选项顺序重排，带不进值的选项保持原样。 */
private fun toggleOption(
    option: PluginSettingOption,
    on: Boolean,
    kind: PluginSettingKind.Choice,
    selected: Set<String>,
): SettingValue.Texts = SettingValue.Texts(
    kind.options.mapNotNull { other ->
        val otherText = optionText(other)
        when {
            otherText == null -> null
            other.value == option.value -> if (on) otherText else null
            otherText in selected -> otherText
            else -> null
        }
    },
)

/** 选项组的标题和说明。 */
@Composable
private fun SettingsChoiceTitles(field: PluginSettingField) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        Text(field.title, style = WontonTheme.typography.body, color = WontonTheme.colors.textPrimary.toColor())
        field.description?.let {
            Text(it, style = WontonTheme.typography.caption, color = WontonTheme.colors.textSecondary.toColor())
        }
    }
}
