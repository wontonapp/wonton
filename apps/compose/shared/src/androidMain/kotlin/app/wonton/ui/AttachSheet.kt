package app.wonton.ui

import android.Manifest
import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import android.provider.Settings
import android.util.Size
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts.RequestPermission
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import app.wonton.ui.R
import app.wonton.ui.design.SheetSendBar
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.File
import app.wonton.ui.icons.Image
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** 相册能不能看：没问过、给过了、被拒了。被拒以后不再弹系统对话框，改给去设置的按钮。 */
private enum class AlbumAccess { Unknown, Granted, Denied }

/** 相册网格一次最多显示多少张：面板里只看得见头几张，翻太远没意义。 */
private const val ALBUM_LIMIT = 300

/** 网格几列。 */
private const val ALBUM_COLUMNS = 3

/**
 * 回形针打开的底部面板：上面是相册的照片网格，点一张选一张；没选时底下是相册和文件的切换，
 * 选了就换成附言条，点发送直接发出去（照 Telegram）。转码期间发送键灰着，附言照常能改，后补的字留下来（ADR-0109）。
 * 附言就是输入框的草稿，面板里改的正是那一份。相册权限只在真的要看网格时问。旁聊里只有相册，文件不加（见 `Composer`）。
 */
@Composable
internal fun AttachmentSheet(
    remaining: Int,
    allowFiles: Boolean,
    note: String,
    onNote: (String) -> Unit,
    onSend: (List<Uri>, String) -> Unit,
    onFiles: () -> Unit,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val permission = remember { photoPermission() }
    var asked by remember { mutableStateOf(false) }
    var access by remember {
        mutableStateOf(
            if (ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED) {
                AlbumAccess.Granted
            } else {
                AlbumAccess.Unknown
            },
        )
    }
    val request = rememberLauncherForActivityResult(RequestPermission()) { granted ->
        access = if (granted) AlbumAccess.Granted else AlbumAccess.Denied
    }
    LaunchedEffect(access, asked) {
        if (access == AlbumAccess.Unknown && !asked) {
            asked = true
            request.launch(permission)
        }
    }
    val selected = remember { mutableStateListOf<Uri>() }
    // 转码和进附件区要一会儿，键先灰掉，再点一下不会拿同一份选中重发一遍。
    var sending by remember { mutableStateOf(false) }
    WontonSheet(onDismiss, title = stringResource(R.string.composer_attach)) { close ->
        Column(Modifier.fillMaxWidth()) {
            Box(Modifier.fillMaxWidth().heightIn(max = 420.dp)) {
                when (access) {
                    AlbumAccess.Unknown -> Unit
                    AlbumAccess.Denied -> DeniedAlbum(onOpenSettings = { openAppSettings(context) })
                    AlbumAccess.Granted -> AlbumGrid(remaining, selected) { uri ->
                        if (uri in selected) selected.remove(uri) else selected.add(uri)
                    }
                }
            }
            if (selected.isNotEmpty()) {
                SheetSendBar(
                    note,
                    onNote,
                    selected.size,
                    stringResource(R.string.sheet_note_hint),
                    onSend = {
                        if (!sending) {
                            sending = true
                            onSend(selected.toList(), note.trim())
                        }
                    },
                    busy = sending,
                    sendTag = "attach-send",
                )
            } else {
                Row(
                    Modifier.fillMaxWidth()
                        .padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s8),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
                ) {
                    SheetTab(stringResource(R.string.attach_album), WontonIcon.Image, selected = true) {}
                    if (allowFiles) {
                        SheetTab(stringResource(R.string.composer_file), WontonIcon.File, selected = false) {
                            close(onFiles)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun SheetTab(label: String, icon: ImageVector, selected: Boolean, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    val tint = (if (selected) colors.textPrimary else colors.textSecondary).toColor()
    Column(
        Modifier.heightIn(min = WontonTheme.size.target)
            .selectable(selected, role = Role.Tab, onClick = onClick)
            .padding(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s4),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(icon, // 装饰：底下有文字。
            contentDescription = null, modifier = Modifier.size(WontonTheme.size.icon), tint = tint)
        Text(
            label,
            style = WontonTheme.typography.caption,
            color = tint,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** 相册里还没有照片。 */
@Composable
private fun EmptyAlbum() {
    Box(Modifier.fillMaxSize().padding(WontonTheme.spacing.s48), contentAlignment = Alignment.Center) {
        Text(
            stringResource(R.string.attach_album_empty),
            style = WontonTheme.typography.body,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
    }
}

/** 相册权限被拒：说清看不到什么，给一个去设置的按钮。 */
@Composable
private fun DeniedAlbum(onOpenSettings: () -> Unit) {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Text(
                stringResource(R.string.attach_permission_denied),
                Modifier.padding(horizontal = WontonTheme.spacing.s48),
                style = WontonTheme.typography.body,
                color = WontonTheme.colors.textSecondary.toColor(),
            )
            Text(
                stringResource(R.string.attach_open_settings),
                Modifier.selectable(selected = false, role = Role.Button, onClick = onOpenSettings)
                    .heightIn(min = WontonTheme.size.target)
                    .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s8),
                style = WontonTheme.typography.body,
                color = WontonTheme.colors.primary.toColor(),
            )
        }
    }
}

/**
 * 相册里的照片，最新的在前。能选的张数用完以后其余的不能再点，已经在里面的照常能取消。
 * 缩略图按格子大小解码，解不出来留一个空底。
 */
@Composable
private fun AlbumGrid(remaining: Int, selected: List<Uri>, onToggle: (Uri) -> Unit) {
    val context = LocalContext.current
    val photos by produceState<List<Uri>?>(null, context) {
        value = withContext(Dispatchers.IO) { queryPhotos(context) }
    }
    val listed = photos
    when {
        listed == null -> Unit
        listed.isEmpty() -> EmptyAlbum()
        else -> LazyVerticalGrid(
            GridCells.Fixed(ALBUM_COLUMNS),
            Modifier.fillMaxSize().testTag("album-grid"),
            contentPadding = PaddingValues(WontonTheme.spacing.s4),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        ) {
            items(listed, key = { it.toString() }) { uri ->
                AlbumCell(
                    uri,
                    picked = uri in selected,
                    enabled = uri in selected || selected.size < remaining,
                    onToggle = onToggle,
                )
            }
        }
    }
}

@Composable
private fun AlbumCell(uri: Uri, picked: Boolean, enabled: Boolean, onToggle: (Uri) -> Unit) {
    val context = LocalContext.current
    val colors = WontonTheme.colors
    val tint = WontonTheme.colors.onPrimary.toColor()
    val thumbnail by produceState<ImageBitmap?>(null, uri) {
        value = withContext(Dispatchers.IO) {
            runCatching {
                val size = Size(THUMBNAIL_PX, THUMBNAIL_PX)
                context.contentResolver.loadThumbnail(uri, size, null).asImageBitmap()
            }.getOrNull()
        }
    }
    Box(
        Modifier.fillMaxWidth()
            .aspectRatio(1f)
            .background(WontonTheme.colors.surfaceSubtle.toColor(), RoundedCornerShape(WontonTheme.radii.control))
            .selectable(selected = picked, enabled = enabled, role = Role.Checkbox, onClick = { onToggle(uri) })
            .testTag("album-photo"),
    ) {
        thumbnail?.let {
            Image(it, // 装饰：勾选状态在语义里。
                contentDescription = null, modifier = Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
        }
        if (picked) {
            Box(
                Modifier.align(Alignment.TopEnd)
                    .padding(WontonTheme.spacing.s4)
                    .size(WontonTheme.size.iconTiny)
                    .background(WontonTheme.colors.primary.toColor(), CircleShape),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    WontonIcon.Check,
                    contentDescription = stringResource(R.string.attach_selected),
                    modifier = Modifier.size(WontonTheme.size.iconTiny).padding(WontonTheme.spacing.s4),
                    tint = tint,
                )
            }
        }
    }
}

private const val THUMBNAIL_PX = 240

private fun photoPermission(): String =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        Manifest.permission.READ_MEDIA_IMAGES
    } else {
        Manifest.permission.READ_EXTERNAL_STORAGE
    }

/** 相册里的照片，最新的在前。 */
@Suppress("DEPRECATION")
private fun queryPhotos(context: Context): List<Uri> {
    val collection = MediaStore.Images.Media.EXTERNAL_CONTENT_URI
    val columns = arrayOf(MediaStore.Images.Media._ID)
    return context.contentResolver
        .query(collection, columns, null, null, "${MediaStore.Images.Media.DATE_ADDED} DESC")
        ?.use { cursor ->
            val id = cursor.getColumnIndexOrThrow(MediaStore.Images.Media._ID)
            buildList {
                while (cursor.moveToNext() && size < ALBUM_LIMIT) {
                    add(ContentUris.withAppendedId(collection, cursor.getLong(id)))
                }
            }
        }
        .orEmpty()
}

private fun openAppSettings(context: Context) {
    val detail = Uri.fromParts("package", context.packageName, null)
    val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, detail)
    intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    context.startActivity(intent)
}
