package app.wonton.ui

import android.content.Context
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException
import app.wonton.protocol.FileAttachment
import app.wonton.ui.design.Icon
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.File
import app.wonton.ui.icons.WontonIcon
import java.io.IOException
import java.security.MessageDigest
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * 文件消息（m.file）的卡片：图标、文件名、大小，说明文字画在下面。点开下载后交给系统打开，
 * "用其他 App 打开"走分享；下载和打开失败按真实原因显示（ADR-0020：内容不可信，不在 App 里渲染）。
 * 下载过的留在本机缓存里，再点直接打开。
 */
@Composable
fun MessageFileCard(file: FileAttachment, core: ClientCore, onLongPress: (() -> Unit)? = null) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var local by remember(file.media) { mutableStateOf<String?>(null) }
    var loading by remember(file.media) { mutableStateOf(false) }
    var problem by remember(file.media) { mutableStateOf<Problem?>(null) }

    // 下载成功后走 after（接着打开或分享），失败把真实原因写进 problem。
    val downloadThen: ((String) -> Unit) -> Unit = { after ->
        loading = true
        problem = null
        scope.launch {
            val path = try {
                downloadToShare(context, core, file, file.media.token)
            } catch (failure: CoreException) {
                problem = failure.problem
                null
            } catch (failure: IOException) {
                problem = Problem(title = failure.message ?: failure.toString())
                null
            }
            if (path != null) {
                // 记住本地路径：再点直接打开，分享入口出现。
                local = path
                after(path)
            }
            loading = false
        }
    }
    val open = {
        // 缓存目录会被系统和图片分享清掉：路径还在、文件没了就当作没下载过，再点重新下。
        local?.takeIf { !java.io.File(it).isFile }?.let { local = null }
        val path = local
        when {
            // 成功时返回 null：顺手清掉上一次的错误。
            path != null -> problem = openWithSystem(context, path, file.mimeType)
            loading -> Unit
            else -> downloadThen { downloaded -> problem = openWithSystem(context, downloaded, file.mimeType) }
        }
        Unit
    }
    Column {
        Row(
            Modifier.combinedClickable(
                role = Role.Button,
                onClick = open,
                onLongClick = onLongPress,
            ),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            FileGlyph(loading)
            Column(Modifier.widthIn(max = 200.dp), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                Text(file.filename, style = WontonTheme.typography.supporting, maxLines = 2)
                file.size?.let {
                    Text(
                        sizeText(context, it),
                        Modifier.alpha(WontonTheme.ratio.secondaryContent),
                        style = WontonTheme.typography.caption,
                    )
                }
                problem?.let {
                    Text(
                        it.titleText,
                        style = WontonTheme.typography.caption,
                        color = WontonTheme.colors.danger.toColor(),
                    )
                }
            }
        }
        if (local != null) {
            Text(
                stringResource(R.string.file_open_elsewhere),
                Modifier.clickable(role = Role.Button) {
                    local?.let { path ->
                        if (java.io.File(path).isFile) {
                            problem = shareWithSystem(context, path, file.mimeType)
                        } else {
                            // 缓存被清了：重新下载后接着分享。
                            local = null
                            if (!loading) {
                                downloadThen { downloaded ->
                                    problem = shareWithSystem(context, downloaded, file.mimeType)
                                }
                            }
                        }
                    }
                },
                style = WontonTheme.typography.supporting.copy(textDecoration = TextDecoration.Underline),
            )
        }
    }
    file.caption?.let { caption -> TextMessageBody(caption.blocks, emptyList(), core) }
}

@Composable
private fun FileGlyph(loading: Boolean) {
    val shape = Modifier.size(WontonTheme.size.fileThumbnail).clip(RoundedCornerShape(WontonTheme.radii.control))
    Box(shape, contentAlignment = Alignment.Center) {
        if (loading) {
            CircularProgressIndicator(Modifier.size(WontonTheme.size.icon))
        } else {
            // 装饰：旁边的文件名说明了这是什么。
            Icon(WontonIcon.File, contentDescription = null, modifier = Modifier.size(WontonTheme.size.icon))
        }
    }
}

/** 字节落进分享缓存（FileProvider 已盖住），名字按文件名清洗；先写临时名再落名，不留半份文件。 */
private suspend fun downloadToShare(context: Context, core: ClientCore, file: FileAttachment, media: String): String =
    withContext(Dispatchers.IO) {
        val bytes = core.loadMedia(file.media)
        // 目录名是完整句柄的 SHA-256：Ruma 序列化的媒体来源（加密附件带密钥材料）能超过文件系统单段名
        // 255 字节上限，原样进路径落不了盘；哈希定长、不含密钥材料。
        val root = java.io.File(context.cacheDir, "share")
        val dir = java.io.File(root, cacheKey(media)).apply { mkdirs() }
        // 文件系统单段名上限 255：超长的名字保扩展名截断，其他 App 靠扩展名认类型。
        val full = galleryName(file.filename, file.mimeType)
        val dot = full.lastIndexOf('.')
        val name = when {
            full.length <= 200 -> full
            dot < 0 || full.length - dot >= 200 -> full.take(200)
            else -> full.take(200 - (full.length - dot)) + full.substring(dot)
        }
        // 临时名随机唯一：并发下载同一媒体也不会写坏对方的 .part。
        val partial = java.io.File.createTempFile("wonton-file-", ".part", dir)
        val target = java.io.File(dir, name)
        partial.writeBytes(bytes)
        if (!partial.renameTo(target)) {
            partial.delete()
            throw IOException(context.getString(R.string.problem_storage_title))
        }
        target.path
    }

/** 缓存目录名：完整句柄的 SHA-256，64 个十六进制字符。同一媒体恒定，不同媒体（含同名文件）互不覆盖。 */
private fun cacheKey(media: String): String =
    MessageDigest.getInstance("SHA-256").digest(media.toByteArray())
        .joinToString("") { "%02x".format(it) }
