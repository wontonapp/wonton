package app.wonton.ui

import app.wonton.protocol.PluginSettingField
import app.wonton.protocol.PluginSettingKind
import app.wonton.protocol.PluginSettingOption
import app.wonton.protocol.SettingValue

/** 现在生效的值：主人改过的优先，没动过的用存着的。 */
internal fun effectiveValue(field: PluginSettingField, edits: Map<String, SettingValue?>): SettingValue? =
    if (edits.containsKey(field.name)) edits[field.name] else field.value

/** 比这个 App 新的控件或值：这一项改不了，整份表单也提交不了（ADR-0077）。 */
internal fun isUnsupported(field: PluginSettingField, edits: Map<String, SettingValue?>): Boolean =
    field.kind is PluginSettingKind.Other || effectiveValue(field, edits) is SettingValue.Other

/** 必填项现在没有值、也没有默认值：提交不了，核心也会拒。 */
internal fun isMissingRequired(field: PluginSettingField, edits: Map<String, SettingValue?>): Boolean =
    field.required && effectiveValue(field, edits) == null && field.default == null

/** 文本草稿换成设置值：还没填成合法值的中间状态先按清空处理，提交前核心会再校验。 */
internal fun draftValue(field: PluginSettingField, draft: String): SettingValue? = when (field.kind) {
    is PluginSettingKind.Integer -> draft.trim().toLongOrNull()?.let { SettingValue.Integer(it) }
    is PluginSettingKind.Decimal -> draft.trim().takeIf { it.isNotEmpty() }?.let { SettingValue.Decimal(it) }
    is PluginSettingKind.Text -> draft.takeIf { it.isNotEmpty() }?.let { SettingValue.Text(it) }
    else -> null
}

/** 文本类控件显示的草稿：主人没动过时显示存着的值。 */
internal fun textDraft(
    field: PluginSettingField,
    edits: Map<String, SettingValue?>,
    drafts: Map<String, String>,
): String = drafts[field.name] ?: when (val value = effectiveValue(field, edits)) {
    is SettingValue.Integer -> value.value.toString()
    is SettingValue.Decimal -> value.value
    is SettingValue.Text -> value.value
    else -> ""
}

/** 选项能带到值里的那串字；带不动的选项（新值类型）选不了。 */
internal fun optionText(option: PluginSettingOption): String? = when (val value = option.value) {
    is SettingValue.Text -> value.value
    is SettingValue.Integer -> value.value.toString()
    is SettingValue.Decimal -> value.value
    is SettingValue.Flag -> value.value.toString()
    is SettingValue.Texts -> null
    SettingValue.Other -> null
}

/** 文本控件按 Schema 的长度上限截住草稿；没写上限就照原样。 */
internal fun draftLimit(field: PluginSettingField): Int = when (val kind = field.kind) {
    is PluginSettingKind.Text -> kind.maxLength?.toInt() ?: Int.MAX_VALUE
    else -> Int.MAX_VALUE
}
