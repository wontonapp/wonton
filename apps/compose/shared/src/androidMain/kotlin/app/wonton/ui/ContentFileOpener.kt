package app.wonton.ui

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import androidx.core.content.FileProvider
import app.wonton.client.ClientCore
import app.wonton.client.FileOpenException
import app.wonton.client.FileOpener
import app.wonton.protocol.CoreException
import app.wonton.protocol.TimelineTarget
import app.wonton.protocol.UploadSource
import java.io.File
import java.io.FileNotFoundException

/** 按内容 URI 打开要上传的文件。文件描述符交给核心，App 重启后接着传时再打开一次。 */
class ContentFileOpener(context: Context) : FileOpener {
    private val appContext = context.applicationContext
    private val resolver = appContext.contentResolver

    override fun openSource(handle: String): Int =
        try {
            resolver.openFileDescriptor(Uri.parse(handle), "r")?.detachFd()
                ?: throw FileOpenException.Unavailable(appContext.getString(R.string.file_system_no_file))
        } catch (failure: SecurityException) {
            throw FileOpenException.PermissionDenied(reason(failure))
        } catch (failure: FileNotFoundException) {
            throw FileOpenException.Unavailable(reason(failure))
        }

    private fun reason(failure: Exception) = failure.message ?: failure.toString()
}

/**
 * 选好、粘贴的文件放进附件区（ADR-0094），马上开始传。按内容 URI 记下，App 重启后接着传时再打开一次。
 * 放不进去时返回要给人看的问题；成功时返回 null。
 */
fun ClientCore.attachFile(context: Context, uri: Uri, roomId: String): Problem? {
    val resolver = context.contentResolver
    try {
        resolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
    } catch (_: SecurityException) {
        // 有的提供方不给长期权限。这次照样传得了；App 重启后接着传时读不了，核心如实报读不了原文件。
    }
    val name = try {
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) cursor.getString(0) else null
        }
    } catch (failure: SecurityException) {
        return Problem(titleRes = R.string.composer_file_unreadable, detail = failure.message)
    }
    return try {
        addFileAttachment(
            TimelineTarget(roomId, null),
            UploadSource.Opaque(uri.toString()),
            name ?: uri.lastPathSegment ?: context.getString(R.string.file_unnamed),
            resolver.getType(uri),
        )
        null
    } catch (rejection: CoreException) {
        rejection.problem
    }
}

/**
 * 下载好的文件交给系统打开。Bot 给的文件内容不可信，不在 App 里渲染（ADR-0020）。
 *
 * 没有 App 能打开时返回原因，供界面提示；成功时返回 null。
 */
fun openWithSystem(context: Context, path: String, mimeType: String?): Problem? {
    val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", File(path))
    // `Intent` 按字面匹配类型：带参数的匹配不上，`application/octet-stream` 没有 App 认，都退回按扩展名。
    val type = mimeType?.let(Intent::normalizeMimeType)?.takeUnless { it == "application/octet-stream" }
    val intent = Intent(Intent.ACTION_VIEW)
        .setDataAndType(uri, type ?: context.contentResolver.getType(uri) ?: "*/*")
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    return try {
        context.startActivity(intent)
        null
    } catch (_: ActivityNotFoundException) {
        Problem(context.getString(R.string.file_no_app_opens_it))
    }
}

/** 下载好的文件交给系统的分享：存到别处、发给别的 App。 */
fun shareWithSystem(context: Context, path: String, mimeType: String? = null) {
    val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", File(path))
    val send = Intent(Intent.ACTION_SEND)
        .setType(mimeType ?: context.contentResolver.getType(uri) ?: "*/*")
        .putExtra(Intent.EXTRA_STREAM, uri)
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    context.startActivity(Intent.createChooser(send, null))
}
