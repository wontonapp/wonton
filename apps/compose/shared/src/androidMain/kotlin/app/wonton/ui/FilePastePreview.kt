package app.wonton.ui

import android.net.Uri
import android.provider.OpenableColumns
import android.text.format.Formatter
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

private data class PastedFile(val name: String, val mime: String?, val size: Long?)

/** Only metadata is read for this preview; the existing upload action runs after confirmation. */
@Composable
internal fun FilePastePreviewSheet(
    uri: Uri,
    roomId: String,
    onSend: (Action) -> Unit,
    onDismiss: () -> Unit,
    onReject: (Problem) -> Unit,
) {
    val context = LocalContext.current
    var preview by remember(uri) { mutableStateOf<Result<PastedFile>?>(null) }
    LaunchedEffect(uri) {
        preview = withContext(Dispatchers.IO) {
            runCatching {
                val resolver = context.contentResolver
                val metadata = resolver.query(
                    uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null,
                )?.use { cursor ->
                    if (!cursor.moveToFirst()) null else {
                        val name = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                        val size = cursor.getColumnIndex(OpenableColumns.SIZE)
                        Pair(
                            if (name >= 0 && !cursor.isNull(name)) cursor.getString(name) else null,
                            if (size >= 0 && !cursor.isNull(size)) cursor.getLong(size).takeIf { it >= 0 } else null,
                        )
                    }
                }
                PastedFile(
                    metadata?.first ?: uri.lastPathSegment ?: context.getString(R.string.file_unnamed),
                    resolver.getType(uri), metadata?.second,
                )
            }
        }
    }
    LaunchedEffect(preview) {
        preview?.exceptionOrNull()?.let { failure ->
            onReject(Problem(titleRes = R.string.composer_file_unreadable, detail = failure.message))
            onDismiss()
        }
    }
    WontonSheet(onDismiss = onDismiss) { close ->
        Column(Modifier.padding(WontonTheme.spacing.pageInset)) {
            val ready = preview?.getOrNull()
            if (ready == null) {
                CircularProgressIndicator()
            } else {
                Text(ready.name, style = WontonTheme.typography.itemTitle)
                ready.mime?.let { Text(it, style = WontonTheme.typography.supporting) }
                ready.size?.let { Text(Formatter.formatShortFileSize(context, it)) }
                Row(Modifier.padding(vertical = WontonTheme.spacing.s8)) {
                    WontonButton(
                        stringResource(R.string.common_cancel), { close {} }, Modifier.weight(1f),
                        WontonButtonKind.SECONDARY,
                    )
                    WontonButton(
                        stringResource(R.string.composer_send),
                        {
                            close {
                                runCatching { uploadAction(context, uri, roomId) }.fold(
                                    onSuccess = onSend,
                                    onFailure = { failure ->
                                        onReject(Problem(
                                            titleRes = R.string.composer_file_unreadable, detail = failure.message,
                                        ))
                                    },
                                )
                            }
                        },
                        Modifier.weight(1f),
                    )
                }
            }
        }
    }
}
