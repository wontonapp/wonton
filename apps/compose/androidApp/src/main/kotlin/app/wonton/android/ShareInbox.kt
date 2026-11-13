package app.wonton.android

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import androidx.core.content.IntentCompat
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException
import app.wonton.ui.Problem
import app.wonton.ui.R
import app.wonton.ui.ShareIntake
import app.wonton.ui.problem
import java.io.File
import java.io.IOException
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/** 系统分享进来的东西（`ACTION_SEND`、`ACTION_SEND_MULTIPLE`）：一段文字和若干 `content://`。 */
internal class SharedContent(val text: String?, val uris: List<Uri>)

/** 取出分享的内容并从 Intent 里删掉：屏幕旋转重建 Activity 时拿到的还是这个 Intent，不能再收一次。 */
internal fun takeShare(intent: Intent): SharedContent? {
    if (intent.action != Intent.ACTION_SEND && intent.action != Intent.ACTION_SEND_MULTIPLE) return null
    val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()?.trim()?.takeIf { it.isNotEmpty() }
    val uris = buildList {
        if (intent.action == Intent.ACTION_SEND) {
            IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)?.let(::add)
        } else {
            IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)?.let(::addAll)
        }
        // 有的应用只放在 ClipData 里（读权限也是随它授予的）。
        val clip = intent.clipData
        if (isEmpty() && clip != null) {
            for (index in 0 until clip.itemCount) clip.getItemAt(index).uri?.let(::add)
        }
    }.distinct()
    intent.removeExtra(Intent.EXTRA_TEXT)
    intent.removeExtra(Intent.EXTRA_STREAM)
    intent.clipData = null
    return if (text == null && uris.isEmpty()) null else SharedContent(text, uris)
}

/**
 * 把分享进来的东西写进收件目录，交给核心认领（ADR-0097）。发送方给的 `content://` 只在这个 Activity 活着时能读，
 * 所以收到就复制，不等主人点发送。目录格式和校验在 `wonton-protocol` 的 `share_in` 模块。
 */
internal class ShareInbox(
    private val context: Context,
    private val core: ClientCore,
    private val intake: ShareIntake,
) {
    suspend fun receive(content: SharedContent) {
        intake.copying = true
        try {
            val token = UUID.randomUUID().toString()
            val written = withContext(Dispatchers.IO) { write(token, content) }
            if (written.skipped > 0) {
                intake.problem = Problem(
                    titleRes = R.string.share_in_unreadable_title,
                    nextStepRes = R.string.share_in_unreadable_next_step,
                    detail = written.firstFailure,
                )
            }
            if (written.complete) withContext(Dispatchers.IO) { core.claimShare(token) }
        } catch (rejection: CoreException) {
            intake.problem = rejection.problem
        } catch (failure: IOException) {
            intake.problem = Problem(
                titleRes = R.string.share_in_unreadable_title,
                nextStepRes = R.string.share_in_unreadable_next_step,
                detail = failure.message,
            )
        } finally {
            intake.copying = false
        }
    }

    private class Written(val complete: Boolean, val skipped: Int, val firstFailure: String?)

    private fun write(token: String, content: SharedContent): Written {
        val dir = File(File(context.filesDir, INBOX_DIR), token)
        if (!dir.mkdirs()) throw IOException("mkdir ${dir.path}")
        val items = JSONArray()
        var copied = 0
        var skipped = 0
        var firstFailure: String? = null
        content.uris.forEachIndexed { index, uri ->
            val file = index.toString()
            val name = displayName(uri) ?: uri.lastPathSegment ?: file
            val mime = context.contentResolver.getType(uri)
            // 超过上限的只记进清单不复制：核心只读前 10 项，其余计数后在分享页提示丢了几项。
            if (copied < MAX_ITEMS) {
                val failure = copy(uri, File(dir, file))
                if (failure != null) {
                    skipped += 1
                    firstFailure = firstFailure ?: failure
                    return@forEachIndexed
                }
                copied += 1
            }
            items.put(JSONObject().put("file", file).put("name", name).put("mime", mime))
        }
        if (content.text == null && copied == 0) {
            dir.deleteRecursively()
            return Written(complete = false, skipped, firstFailure)
        }
        val manifest = JSONObject().put("v", MANIFEST_VERSION).put("text", content.text).put("items", items)
        File(dir, MANIFEST_FILE).writeText(manifest.toString())
        // 最后写：核心见到它才认领（没写完的目录被删掉）。
        File(dir, READY_FILE).writeText("")
        return Written(complete = true, skipped, firstFailure)
    }

    /** 读不了的一项返回原因，不中断整次分享；目标文件写了一半的删掉。 */
    private fun copy(uri: Uri, target: File): String? =
        try {
            val input = context.contentResolver.openInputStream(uri) ?: throw IOException(uri.toString())
            input.use { source -> target.outputStream().use { source.copyTo(it) } }
            null
        } catch (failure: IOException) {
            target.delete()
            failure.message ?: failure.javaClass.simpleName
        } catch (failure: SecurityException) {
            target.delete()
            failure.message ?: failure.javaClass.simpleName
        }

    private fun displayName(uri: Uri): String? =
        try {
            context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) cursor.getString(0)?.takeIf { it.isNotBlank() } else null
            }
        } catch (_: SecurityException) {
            null
        }

    private companion object {
        // 同 wonton-protocol 的 SHARE_INBOX_DIR、SHARE_MANIFEST_FILE、SHARE_READY_FILE、SHARE_MAX_ITEMS、
        // ShareInManifest::VERSION；UniFFI 不导出常量。
        const val INBOX_DIR = "share-inbox"
        const val MANIFEST_FILE = "manifest.json"
        const val READY_FILE = ".ready"
        const val MAX_ITEMS = 10
        const val MANIFEST_VERSION = 1
    }
}
