package app.wonton.ui

import android.content.ContentResolver
import android.net.Uri
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.content.ReceiveContentListener
import androidx.compose.foundation.content.consume

/**
 * 输入框收到的内容（长按粘贴、Ctrl+V、键盘的图片插入、拖放）。带正文的条目是链接或富文本，照常粘成文字；
 * 只带 URI 的条目按类型交给 [onImage] 和 [onFile]，都进附件区（ADR-0094）；一次粘贴几个就放几个。
 */
@OptIn(ExperimentalFoundationApi::class)
internal fun pasteContentListener(
    resolver: ContentResolver,
    onImage: (Uri) -> Unit,
    onFile: (Uri) -> Unit,
): ReceiveContentListener = ReceiveContentListener { transferable ->
    val clip = transferable.clipEntry?.clipData ?: return@ReceiveContentListener transferable
    transferable.consume { item ->
        val uri = item.uri ?: return@consume false
        if (item.text != null) return@consume false
        // 有的来源（如键盘插入）URI 上查不到类型，剪贴板描述里带；描述只有通配符时当文件。
        val mime = runCatching { resolver.getType(uri) }.getOrNull()
            ?: clip.description.filterMimeTypes("image/*").firstOrNull()
        if (mime?.startsWith("image/") == true) onImage(uri) else onFile(uri)
        true
    }
}
