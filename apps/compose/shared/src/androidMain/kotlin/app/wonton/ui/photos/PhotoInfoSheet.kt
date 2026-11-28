package app.wonton.ui.photos

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.PhotoDetailView
import app.wonton.protocol.PhotoInfo
import app.wonton.protocol.PhotoTileOwner
import app.wonton.protocol.PhotosAction
import app.wonton.ui.ProblemRow
import app.wonton.ui.R
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Clock
import app.wonton.ui.icons.Device
import app.wonton.ui.icons.Document
import app.wonton.ui.icons.Globe
import app.wonton.ui.icons.Image
import app.wonton.ui.icons.Person
import app.wonton.ui.icons.WontonIcon
import java.util.Locale

/**
 * 大图页的信息面板：拍摄时间、文件、相机、地点、人物、描述，有哪项出哪项。时间来自网格里的那一格，马上就有；
 * 其余等核心取详情，取不成时留一条带重试的提示。
 */
@Composable
internal fun PhotoInfoSheet(state: PhotosState, detail: PhotoDetailView, onDismiss: () -> Unit) {
    val context = LocalContext.current
    WontonSheet(onDismiss) {
        SheetList {
            val tile = detail.tile
            tile?.let { InfoRow(WontonIcon.Clock, takenAtText(context, it.takenAt), null) }
            when (val owner = tile?.owner) {
                is PhotoTileOwner.Person -> {
                    val from = stringResource(R.string.photo_library_owned_by, owner.user.name)
                    InfoRow(WontonIcon.Person, from, null)
                }
                PhotoTileOwner.Unknown ->
                    InfoRow(WontonIcon.Person, stringResource(R.string.photo_library_owned_by_unknown), null)
                PhotoTileOwner.Me, null -> Unit
            }
            detail.info?.let { InfoRows(it) }
            val failure = detail.problem
            if (failure != null) {
                Box(Modifier.padding(WontonTheme.spacing.pageInset)) {
                    ProblemRow(failure.problem) { state.send(PhotosAction.OpenDetail(detail.id)) }
                }
            } else if (detail.loading) {
                Box(Modifier.fillMaxWidth().padding(WontonTheme.spacing.s16), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator(color = WontonTheme.colors.textSecondary.toColor())
                }
            }
        }
    }
}

@Composable
private fun InfoRows(info: PhotoInfo) {
    FileRow(info)
    // 两项里有哪项写哪项：都有时相机在上、镜头在下。
    PairRow(WontonIcon.Device, info.camera, info.lens)
    val coordinates = info.location?.let { String.format(Locale.ROOT, "%.5f, %.5f", it.lat, it.lon) }
    PairRow(WontonIcon.Globe, info.place.joinToString(", ").ifEmpty { null }, coordinates)
    if (info.people.isNotEmpty()) InfoRow(WontonIcon.Person, info.people.joinToString(", ") { it.name }, null)
    info.description?.takeIf { it.isNotBlank() }?.let { InfoRow(WontonIcon.Document, it, null) }
}

/** 文件名在上，像素尺寸和大小在下；没有文件名时尺寸和大小顶上来。 */
@Composable
private fun FileRow(info: PhotoInfo) {
    val context = LocalContext.current
    val width = info.width?.toInt()
    val height = info.height?.toInt()
    val pixels = if (width != null && height != null) {
        stringResource(R.string.photo_library_dimensions, width, height)
    } else null
    val facts = listOfNotNull(pixels, info.size?.let { sizeText(context, it) }).joinToString(" · ")
    PairRow(WontonIcon.Image, info.fileName, facts.ifEmpty { null })
}

@Composable
private fun PairRow(icon: ImageVector, first: String?, second: String?) {
    val title = first ?: second ?: return
    InfoRow(icon, title, second.takeIf { first != null })
}

@Composable
private fun InfoRow(icon: ImageVector, title: String, detail: String?) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth()
            .padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s12),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // 装饰：这一行是什么由右边的内容说明。
        Icon(
            icon,
            contentDescription = null,
            Modifier.size(WontonTheme.size.icon),
            tint = colors.textSecondary.toColor(),
        )
        Column {
            Text(title, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
            detail?.let { Text(it, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor()) }
        }
    }
}
