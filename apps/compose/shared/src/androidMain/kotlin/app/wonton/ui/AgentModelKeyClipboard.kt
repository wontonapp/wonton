package app.wonton.ui

import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import app.wonton.ui.design.WontonSmallButton

/**
 * 从“去拿 key”的网页回来时看一眼剪贴板：内容以这家 key 的固定开头起头，就给一个“粘贴刚复制的 key”（ADR-0129 第 3 节）。
 * 只在离开过 App 再回来、并且这一页在填 key 时读一次——Android 13 起读剪贴板系统会提示，不在别的时候读。
 * 没有固定开头的服务商不读：认不出哪段是 key。
 */
@Composable
internal fun ClipboardKeyOffer(prefix: String?, current: String, onPaste: (String) -> Unit) {
    if (prefix.isNullOrEmpty()) return
    val context = LocalContext.current
    var away by remember { mutableStateOf(false) }
    var look by remember { mutableStateOf(false) }
    var offer by remember { mutableStateOf<String?>(null) }
    LifecycleEventEffect(Lifecycle.Event.ON_STOP) { away = true }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) {
        if (away) {
            away = false
            look = true
        }
    }
    // resume 那一下窗口还没拿到焦点，系统不给读；等焦点回来。
    val focused = LocalWindowInfo.current.isWindowFocused
    LaunchedEffect(look, focused) {
        if (look && focused) {
            look = false
            offer = context.copiedKey(prefix)
        }
    }
    val found = offer?.takeIf { it != current } ?: return
    WontonSmallButton(stringResource(R.string.agent_models_key_paste), {
        offer = null
        onPaste(found)
    })
}

/** 剪贴板里像这家 key 的那段文字：以固定开头起头、后面还有内容、中间没有空白。不是文字时不取内容。 */
private fun Context.copiedKey(prefix: String): String? {
    val clipboard = getSystemService(ClipboardManager::class.java)
    val isText = clipboard?.primaryClipDescription?.let {
        it.hasMimeType(ClipDescription.MIMETYPE_TEXT_PLAIN) || it.hasMimeType(ClipDescription.MIMETYPE_TEXT_HTML)
    } == true
    val text = clipboard?.takeIf { isText }?.primaryClip?.takeIf { it.itemCount > 0 }
        ?.getItemAt(0)?.text?.toString()?.trim()
    return text?.takeIf { it.startsWith(prefix) && it.length > prefix.length && it.none(Char::isWhitespace) }
}
