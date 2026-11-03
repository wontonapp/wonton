package app.wonton.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.hideFromAccessibility
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.wonton.client.ClientCore
import app.wonton.client.accentHex
import app.wonton.client.parseAccent
import app.wonton.client.previewAccent
import app.wonton.protocol.Action
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.Rgb
import app.wonton.protocol.ThemeColors
import kotlin.math.roundToInt

private val BODY = TextStyle(fontSize = 16.sp, lineHeight = 22.sp)
private val SUPPORTING = TextStyle(fontSize = 14.sp, lineHeight = 20.sp)
private val CAPTION = TextStyle(fontSize = 12.sp, lineHeight = 16.sp)
private val THUMB = 24.dp
private const val MAX_CHANNEL = 255f

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AppearancePage(core: ClientCore, appearance: AppearanceViewModel, onBack: () -> Unit) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("外观") },
                navigationIcon = { TextButton(onClick = onBack) { Text("返回") } },
            )
        },
    ) { padding ->
        AppearanceScreen(core, appearance, Modifier.padding(padding))
    }
}

/** 选重点色：预设、自定义 RGB 和恢复默认。颜色只由核心推导，这里只渲染。拖动滑块时只预览，松手才保存。 */
@Composable
private fun AppearanceScreen(core: ClientCore, appearance: AppearanceViewModel, modifier: Modifier) {
    val colors = WontonTheme.colors
    var draft by remember { mutableStateOf(appearance.accent) }
    var hexText by remember { mutableStateOf(accentHex(appearance.accent)) }
    var isHexInvalid by remember { mutableStateOf(false) }
    var rejection by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(appearance.accent) {
        draft = appearance.accent
        hexText = accentHex(appearance.accent)
        isHexInvalid = false
    }
    val send = { action: Action -> rejection = core.send(action) }
    val preview = remember(draft) { previewAccent(draft) }
    val theme = if (isSystemInDarkTheme()) preview.dark else preview.light
    val updateDraft = { color: Rgb ->
        draft = color
        hexText = accentHex(color)
    }
    Column(
        modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp),
    ) {
        (rejection ?: appearance.problem?.message)?.let { problem ->
            Text(problem, style = SUPPORTING, color = colors.danger.toColor())
        }
        AppearancePreview(theme)
        Presets(appearance.presets, draft) { send(Action.SetAccentColor(it)) }
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            HexField(
                text = hexText,
                onChange = {
                    hexText = it
                    isHexInvalid = false
                },
                onSubmit = {
                    val color = parseAccent(hexText)
                    if (color != null) send(Action.SetAccentColor(color)) else isHexInvalid = true
                },
            )
            if (isHexInvalid) Text("颜色要写成 #RRGGBB", style = SUPPORTING, color = colors.danger.toColor())
            val ink = theme.accentInk.toColor()
            val commit = { send(Action.SetAccentColor(draft)) }
            ChannelSlider("R", "红", draft.red, ink, { updateDraft(draft.copy(red = it)) }, commit)
            ChannelSlider("G", "绿", draft.green, ink, { updateDraft(draft.copy(green = it)) }, commit)
            ChannelSlider("B", "蓝", draft.blue, ink, { updateDraft(draft.copy(blue = it)) }, commit)
        }
        Box(
            Modifier.fillMaxWidth()
                .heightIn(min = 48.dp)
                .background(colors.surfaceSubtle.toColor(), RoundedCornerShape(12.dp))
                .clickable(role = Role.Button) { send(Action.ResetAccentColor) },
            contentAlignment = Alignment.Center,
        ) {
            Text("恢复默认", style = BODY, color = colors.textPrimary.toColor())
        }
    }
}

@Composable
private fun AppearancePreview(theme: ThemeColors) {
    Column(
        Modifier.fillMaxWidth().semantics { hideFromAccessibility() },
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Row(Modifier.fillMaxWidth().padding(end = 48.dp)) {
            Bubble(theme.surfaceSubtle.toColor(), null) {
                Text("今晚吃什么？", style = BODY, color = theme.textPrimary.toColor())
            }
        }
        Row(Modifier.fillMaxWidth().padding(start = 48.dp), horizontalArrangement = Arrangement.End) {
            Bubble(theme.accentFill.toColor(), theme.accentFillBorder?.toColor()) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        "云吞",
                        Modifier.alignByBaseline(),
                        style = BODY,
                        color = theme.textOnAccentFill.toColor(),
                    )
                    Text(
                        "21:08",
                        Modifier.alignByBaseline(),
                        style = CAPTION,
                        color = theme.accentInkOnFill.toColor(),
                    )
                }
            }
        }
    }
}

@Composable
private fun Bubble(fill: Color, border: Color?, content: @Composable () -> Unit) {
    val shape = RoundedCornerShape(16.dp)
    Box(
        Modifier.background(fill, shape)
            .then(if (border != null) Modifier.border(1.dp, border, shape) else Modifier)
            .padding(horizontal = 12.dp, vertical = 8.dp),
    ) {
        content()
    }
}

@Composable
private fun Presets(presets: List<Rgb>, selected: Rgb, onSelect: (Rgb) -> Unit) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
        presets.forEachIndexed { index, preset ->
            PresetSwatch(preset, preset == selected, "重点色 ${index + 1}") { onSelect(preset) }
        }
    }
}

/** 选中时外面多一圈：只靠颜色的话，和页面底色接近的预设看不出选没选中。 */
@Composable
private fun PresetSwatch(color: Rgb, isSelected: Boolean, label: String, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    Box(
        Modifier.size(48.dp)
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
private fun HexField(text: String, onChange: (String) -> Unit, onSubmit: () -> Unit) {
    val colors = WontonTheme.colors
    BasicTextField(
        value = text,
        onValueChange = onChange,
        singleLine = true,
        textStyle = BODY.copy(color = colors.textPrimary.toColor()),
        cursorBrush = SolidColor(colors.textPrimary.toColor()),
        keyboardOptions = KeyboardOptions(
            capitalization = KeyboardCapitalization.Characters,
            autoCorrectEnabled = false,
            keyboardType = KeyboardType.Ascii,
            imeAction = ImeAction.Done,
        ),
        keyboardActions = KeyboardActions(onDone = { onSubmit() }),
        modifier = Modifier.fillMaxWidth().semantics { contentDescription = "颜色值" },
        decorationBox = { field ->
            Box(
                Modifier.fillMaxWidth()
                    .heightIn(min = 48.dp)
                    .background(colors.surfaceSubtle.toColor(), RoundedCornerShape(12.dp))
                    .padding(horizontal = 12.dp),
                contentAlignment = Alignment.CenterStart,
            ) {
                field()
            }
        },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ChannelSlider(
    name: String,
    spokenName: String,
    value: UByte,
    ink: Color,
    onChange: (UByte) -> Unit,
    onCommit: () -> Unit,
) {
    val colors = WontonTheme.colors
    val text = colors.textPrimary.toColor()
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(name, Modifier.width(16.dp).semantics { hideFromAccessibility() }, style = BODY, color = text)
        Slider(
            value = value.toFloat(),
            onValueChange = { onChange(it.roundToInt().toUByte()) },
            onValueChangeFinished = onCommit,
            valueRange = 0f..MAX_CHANNEL,
            modifier = Modifier.weight(1f).semantics { contentDescription = spokenName },
            thumb = { Box(Modifier.size(THUMB).background(ink, CircleShape)) },
            track = { state ->
                val track = colors.surfaceSubtle.toColor()
                val fraction = state.value / MAX_CHANNEL
                Canvas(Modifier.fillMaxWidth().height(THUMB)) {
                    val thickness = 4.dp.toPx()
                    val top = (size.height - thickness) / 2
                    val radius = CornerRadius(thickness / 2)
                    drawRoundRect(track, Offset(0f, top), Size(size.width, thickness), radius)
                    drawRoundRect(ink, Offset(0f, top), Size(size.width * fraction, thickness), radius)
                }
            },
        )
        Text(
            value.toString(),
            Modifier.width(32.dp).semantics { hideFromAccessibility() },
            style = BODY.copy(textAlign = TextAlign.End, fontFeatureSettings = "tnum"),
            color = text,
        )
    }
}
