package app.wonton.ui.design

import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.res.stringResource
import app.wonton.ui.R
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.Copy
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.delay

/**
 * App 里所有的“复制”按钮：点了先由 [copy] 写进剪贴板，再把图标换成对勾、文字换成“已复制”，带一下确认触觉，
 * 1.5 秒后复原。不给反馈时主人点了以为没反应（11-27 反馈恢复密钥的复制）。iOS 是同名的 `WontonCopyButton`。
 * 敏感内容（恢复密钥、插件生成的密钥）的 [copy] 用 `copySensitive`，系统剪贴板提示不显示原文。
 */
@Composable
fun WontonCopyButton(
    copy: () -> Unit,
    modifier: Modifier = Modifier,
    look: WontonCopyLook = WontonCopyLook.SMALL,
    label: String? = null,
    tint: Color? = null,
) {
    val haptics = LocalHapticFeedback.current
    // 每点一次加一：再点一次重新计时，触觉也再响一次。
    var copies by remember { mutableIntStateOf(0) }
    var done by remember { mutableStateOf(false) }
    LaunchedEffect(copies) {
        if (copies == 0) return@LaunchedEffect
        done = true
        delay(COPIED_MS)
        done = false
    }
    val onClick = {
        copy()
        haptics.performHapticFeedback(HapticFeedbackType.Confirm)
        copies += 1
    }
    val text = stringResource(if (done) R.string.design_copied else R.string.design_copy)
    val icon = if (done) WontonIcon.Check else WontonIcon.Copy
    when (look) {
        WontonCopyLook.WIDE -> WontonButton(text, onClick, modifier, WontonButtonKind.SECONDARY, icon = icon)
        WontonCopyLook.SMALL -> Box(modifier) { WontonSmallButton(text, onClick, icon = icon) }
        WontonCopyLook.ICON -> Box(modifier) {
            WontonIconButton(icon, if (done) text else label ?: text, onClick, tint)
        }
    }
}

/** 够看清对勾，又不会让人以为还要再点什么（两端同一个时长）。 */
private const val COPIED_MS = 1_500L
