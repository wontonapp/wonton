package app.wonton.ui

import android.content.ClipData
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties
import app.wonton.client.ClientCore
import app.wonton.protocol.MessageAction
import app.wonton.protocol.TimelineMessage
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Copy
import app.wonton.ui.icons.WontonIcon
import kotlin.math.roundToInt
import kotlinx.coroutines.launch

/** 窗口的左上角，气泡的位置也是从这里算的。 */
private object WindowOrigin : PopupPositionProvider {
    override fun calculatePosition(
        anchorBounds: IntRect,
        windowSize: IntSize,
        layoutDirection: LayoutDirection,
        popupContentSize: IntSize,
    ) = IntOffset.Zero
}

/// 消息预览留在原来的位置，菜单贴着它：下面放得下放下面，放不下翻到上面，两头都夹在可视区里。
/// `bubble` 是气泡在屏幕上的位置（根坐标），`visible` 是能用的那一片（同样坐标，已经让开系统栏和键盘）。
@Composable
fun MessageMenu(
    message: TimelineMessage,
    bubble: Rect,
    visible: Rect,
    core: ClientCore,
    onDismiss: () -> Unit,
) {
    var origin by remember { mutableStateOf(Offset.Zero) }
    val bounds = bubble.translate(-origin)
    // 弹层里的坐标从让开边距以后的左上角算起，可视区也要跟着挪。
    val area = visible.translate(-origin)
    val gap = WontonTheme.spacing.s8
    val scrim = Color.Black.copy(alpha = WontonTheme.ratio.scrim)
    val width = with(LocalDensity.current) { bounds.width.toDp() }
    // 这两样要在弹层外面取：弹层是单独的窗口，收不到系统边距和返回键。它不抢焦点，键盘还开着，页面不会动。
    val insets = WindowInsets.safeDrawing.asPaddingValues()
    BackHandler(onBack = onDismiss)
    Popup(WindowOrigin, onDismiss, PopupProperties(clippingEnabled = false)) {
        Box(
            Modifier
                .fillMaxSize()
                .background(scrim)
                .clickable(remember { MutableInteractionSource() }, indication = null, onClick = onDismiss),
        ) {
            Layout(
                content = {
                    // 预览不接受操作，读屏也不再念一遍。
                    Box(Modifier.width(width).clearAndSetSemantics {}) {
                        BubbleSurface(message, core, false, false, Modifier, {}, {}, {})
                    }
                    MenuSurface(message.actions, onDismiss)
                },
                modifier = Modifier
                    .fillMaxSize()
                    .padding(insets)
                    .onGloballyPositioned { origin = it.positionInRoot() },
            ) { measurables, constraints ->
                val loose = Constraints(maxWidth = constraints.maxWidth, maxHeight = constraints.maxHeight)
                val preview = measurables[0].measure(loose)
                val menu = measurables[1].measure(loose)
                val spacing = gap.roundToPx()
                // 弹层的窗口可能比看得见的地方长（键盘、系统栏那么一块照画不误），两样都别越过。
                val left = area.left.roundToInt().coerceIn(0, constraints.maxWidth)
                val top = area.top.roundToInt().coerceIn(0, constraints.maxHeight)
                val right = area.right.roundToInt().coerceIn(left, constraints.maxWidth)
                val bottom = area.bottom.roundToInt().coerceIn(top, constraints.maxHeight)
                val previewTop = bounds.top.roundToInt().coerceIn(top, maxOf(top, bottom - preview.height))
                val below = previewTop + preview.height + spacing
                val above = previewTop - spacing - menu.height
                val wanted = if (below + menu.height <= bottom) below else above
                val menuTop = wanted.coerceIn(top, maxOf(top, bottom - menu.height))
                val menuX = if (message.isOwn) bounds.right.roundToInt() - menu.width else bounds.left.roundToInt()
                layout(constraints.maxWidth, constraints.maxHeight) {
                    preview.place(
                        bounds.left.roundToInt().coerceIn(left, maxOf(left, right - preview.width)),
                        previewTop,
                    )
                    menu.place(menuX.coerceIn(left, maxOf(left, right - menu.width)), menuTop)
                }
            }
        }
    }
}

@Composable
private fun MenuSurface(actions: List<MessageAction>, onDismiss: () -> Unit) {
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    val copyLabel = stringResource(R.string.message_copy)
    // 剪贴板条目的标签：别的 App 粘贴时可能显示。
    val clipLabel = stringResource(R.string.message_clip_label)
    Surface(
        shape = MaterialTheme.shapes.medium,
        color = WontonTheme.colors.surface.toColor(),
        shadowElevation = WontonTheme.size.menuElevation,
    ) {
        Column(Modifier.width(WontonTheme.size.menuWidth)) {
            actions.forEach { action ->
                when (action) {
                    is MessageAction.Copy -> DropdownMenuItem(
                        text = { Text(copyLabel) },
                        leadingIcon = {
                            Icon(WontonIcon.Copy, contentDescription = null, Modifier.size(WontonTheme.size.iconSmall))
                        },
                        onClick = {
                            scope.launch {
                                clipboard.setClipEntry(ClipEntry(ClipData.newPlainText(clipLabel, action.text)))
                            }
                            onDismiss()
                        },
                    )
                    // 核心和 App 分开升级时，不认识的菜单项不显示。
                    else -> Unit
                }
            }
        }
    }
}
