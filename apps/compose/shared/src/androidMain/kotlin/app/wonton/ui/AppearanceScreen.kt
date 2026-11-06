package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.hideFromAccessibility
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.client.accentHex
import app.wonton.client.parseAccent
import app.wonton.protocol.Action
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.FontStyle
import app.wonton.protocol.Rgb
import app.wonton.protocol.TextSize
import app.wonton.protocol.ThemeMode
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.accentFill
import app.wonton.ui.design.family
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.WontonIcon

private const val EXPANDED_DEGREES = 90f

/** 外观设置都在核心，改了整个 App 当场跟着变。 */
@Composable
internal fun AppearanceScreen(core: ClientCore, appearance: AppearanceViewModel) {
    var rejection by remember { mutableStateOf<Problem?>(null) }
    val send = { action: Action -> rejection = core.send(action) }
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        (rejection ?: appearance.problem?.problem)?.let { ProblemNotice(it) }
        SettingsPreview()
        Section("明暗") {
            WontonGroup {
                appearance.themeModes.forEachIndexed { index, option ->
                    if (index > 0) SettingsSeparator()
                    ChoiceRow(option.label, option == appearance.themeMode) {
                        send(Action.SetThemeMode(option))
                    }
                }
            }
        }
        Section("重点色") { AccentGroup(appearance, send) }
        Section("字号") {
            WontonGroup {
                appearance.textSizes.forEachIndexed { index, option ->
                    if (index > 0) SettingsSeparator()
                    ChoiceRow(option.label, option == appearance.textSize) {
                        send(Action.SetTextSize(option))
                    }
                }
            }
        }
        Section("字体") {
            WontonGroup {
                appearance.fonts.forEachIndexed { index, option ->
                    if (index > 0) SettingsSeparator()
                    ChoiceRow(option.label, option == appearance.font, FontSample(option.family)) {
                        send(Action.SetFont(option))
                    }
                }
            }
        }
    }
}

@Composable
private fun Section(title: String, content: @Composable () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Text(
            title,
            Modifier.padding(horizontal = WontonTheme.spacing.s16).semantics { heading() },
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
        content()
    }
}

@Composable
private fun SettingsSeparator() {
    Box(
        Modifier.padding(start = WontonTheme.spacing.s16)
            .fillMaxWidth()
            .height(WontonTheme.size.hairline)
            .background(WontonTheme.colors.separator.toColor()),
    )
}

/** 设置里各个选项的名字，顺序由核心给。 */
private val ThemeMode.label: String
    get() = when (this) {
        ThemeMode.SYSTEM -> "跟随系统"
        ThemeMode.LIGHT -> "浅色"
        ThemeMode.DARK -> "深色"
    }

private val TextSize.label: String
    get() = when (this) {
        TextSize.SYSTEM -> "跟随系统"
        TextSize.SMALL -> "小"
        TextSize.STANDARD -> "标准"
        TextSize.LARGE -> "大"
        TextSize.EXTRA_LARGE -> "特大"
    }

private val FontStyle.label: String
    get() = when (this) {
        FontStyle.DEFAULT -> "默认"
        FontStyle.ROUNDED -> "圆体"
        FontStyle.SERIF -> "衬线"
        FontStyle.MONOSPACED -> "等宽"
    }

/** 每一行用自己的字体写，选之前就看得出差别。系统字体的中文不一定跟着变，所以要有拉丁字母的样例。 */
private class FontSample(val family: FontFamily?)

@Composable
private fun ChoiceRow(title: String, isSelected: Boolean, sample: FontSample? = null, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    val body = WontonTheme.typography.body
    val style = if (sample != null) body.copy(fontFamily = sample.family) else body
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = WontonTheme.size.target)
            .selectable(selected = isSelected, role = Role.RadioButton, onClick = onClick)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Text(title, style = style, color = colors.textPrimary.toColor())
        if (sample != null) {
            Text(
                "Aa",
                Modifier.semantics { hideFromAccessibility() },
                style = style,
                color = colors.textSecondary.toColor(),
            )
        }
        Spacer(Modifier.weight(1f))
        if (isSelected) {
            Icon(WontonIcon.Check, null, Modifier.size(WontonTheme.size.iconSmall), tint = colors.accentInk.toColor())
        }
    }
}

@Composable
private fun AccentGroup(appearance: AppearanceViewModel, send: (Action) -> Unit) {
    val isCustom = appearance.accent !in appearance.presets
    var showsMore by rememberSaveable { mutableStateOf(isCustom) }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        WontonGroup {
            Swatches(appearance.presets, appearance, "重点色", send, Modifier.padding(WontonTheme.spacing.s8))
            SettingsSeparator()
            MoreColorsRow(if (isCustom) accentHex(appearance.accent) else null, showsMore) { showsMore = !showsMore }
            if (showsMore) {
                SettingsSeparator()
                Swatches(appearance.moreColors, appearance, "更多颜色", send, Modifier.padding(WontonTheme.spacing.s8))
            }
        }
        // 放在组外面：输入框的底和组的底同色，放进去就看不见了。
        if (showsMore) HexInput(appearance.accent, send)
    }
}

/** [custom] 是选的颜色不在色板里时的色值。 */
@Composable
private fun MoreColorsRow(custom: String?, isExpanded: Boolean, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = WontonTheme.size.target)
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { stateDescription = if (isExpanded) "已展开" else "已收起" }
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Text(
            "更多",
            Modifier.weight(1f),
            style = WontonTheme.typography.body,
            color = colors.textPrimary.toColor(),
        )
        if (custom != null) {
            Text(custom, style = WontonTheme.typography.body, color = colors.textSecondary.toColor())
        }
        Icon(
            WontonIcon.Forward,
            null,
            Modifier.size(WontonTheme.size.iconSmall).rotate(if (isExpanded) EXPANDED_DEGREES else 0f),
            tint = colors.textSecondary.toColor(),
        )
    }
}

@Composable
private fun Swatches(
    swatches: List<Rgb>,
    appearance: AppearanceViewModel,
    label: String,
    send: (Action) -> Unit,
    modifier: Modifier = Modifier,
) {
    val columns = appearance.swatchColumns.toInt()
    Column(modifier) {
        swatches.chunked(columns).forEachIndexed { row, chunk ->
            Row(Modifier.fillMaxWidth()) {
                chunk.forEachIndexed { column, swatch ->
                    Swatch(
                        color = swatch,
                        isSelected = swatch == appearance.accent,
                        label = "$label ${row * columns + column + 1}",
                        modifier = Modifier.weight(1f),
                    ) { send(Action.SetAccentColor(swatch)) }
                }
            }
        }
    }
}

/** 选中时外面多一圈：只靠颜色的话，和页面底色接近的色板看不出选没选中。 */
@Composable
private fun Swatch(color: Rgb, isSelected: Boolean, label: String, modifier: Modifier, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    Box(
        modifier.height(WontonTheme.size.target)
            .selectable(selected = isSelected, role = Role.RadioButton, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier.size(36.dp)
                .background(color.toColor(), CircleShape)
                .border(1.dp, colors.separator.toColor(), CircleShape),
        )
        if (isSelected) Box(Modifier.size(46.dp).border(2.dp, colors.textPrimary.toColor(), CircleShape))
    }
}

@Composable
private fun HexInput(accent: Rgb, send: (Action) -> Unit) {
    var text by remember { mutableStateOf(accentHex(accent)) }
    var isInvalid by remember { mutableStateOf(false) }
    LaunchedEffect(accent) {
        text = accentHex(accent)
        isInvalid = false
    }
    WontonTextField(
        value = text,
        onValueChange = {
            text = it
            isInvalid = false
        },
        modifier = Modifier.fillMaxWidth(),
        label = "颜色值",
        keyboardOptions = KeyboardOptions(
            capitalization = KeyboardCapitalization.Characters,
            autoCorrectEnabled = false,
            keyboardType = KeyboardType.Ascii,
            imeAction = ImeAction.Done,
        ),
        keyboardActions = KeyboardActions(
            onDone = {
                val color = parseAccent(text)
                if (color != null) send(Action.SetAccentColor(color)) else isInvalid = true
            },
        ),
    )
    if (isInvalid) {
        Text(
            "颜色要写成 #RRGGBB",
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.danger.toColor(),
        )
    }
}

/** 一段小对话：改的每一项都在这里当场看到。 */
@Composable
private fun SettingsPreview() {
    val colors = WontonTheme.colors
    val bubble = RoundedCornerShape(WontonTheme.radii.bubble)
    val inset = Modifier.padding(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s8)
    Column(
        Modifier.fillMaxWidth().semantics { hideFromAccessibility() },
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Row(Modifier.fillMaxWidth().padding(end = WontonTheme.spacing.s48)) {
            Text(
                "今晚吃什么？",
                Modifier.background(colors.surfaceSubtle.toColor(), bubble).then(inset),
                style = WontonTheme.typography.body,
                color = colors.textPrimary.toColor(),
            )
        }
        Row(Modifier.fillMaxWidth().padding(start = WontonTheme.spacing.s48)) {
            Spacer(Modifier.weight(1f))
            Row(
                Modifier.accentFill(bubble).then(inset),
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                Text(
                    "云吞",
                    Modifier.alignByBaseline(),
                    style = WontonTheme.typography.body,
                    color = colors.textOnAccentFill.toColor(),
                )
                Text(
                    "21:08",
                    Modifier.alignByBaseline(),
                    style = WontonTheme.typography.caption,
                    color = colors.accentInkOnFill.toColor(),
                )
            }
        }
    }
}
