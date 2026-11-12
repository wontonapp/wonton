package app.wonton.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.protocol.AttachmentKind
import app.wonton.protocol.RoomRow
import app.wonton.protocol.ShareInItem
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Expand
import app.wonton.ui.icons.File
import app.wonton.ui.icons.Image
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** 分享页的去向行：选中的聊天的头像和名字，点开换一个；还没选时提示去选。 */
@Composable
internal fun ShareRoomRow(row: RoomRow?, avatar: @Composable (RoomRow) -> Unit, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = WontonTheme.size.target)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(vertical = WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        if (row != null) avatar(row)
        Text(
            row?.name ?: stringResource(R.string.share_in_choose_chat),
            Modifier.weight(1f),
            style = WontonTheme.typography.itemTitle,
            color = (if (row != null) colors.textPrimary else colors.textSecondary).toColor(),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Icon(
            WontonIcon.Expand,
            // 装饰：整行能点，名字已经读出来了。
            contentDescription = null,
            modifier = Modifier.size(WontonTheme.size.iconSmall),
            tint = colors.textSecondary.toColor(),
        )
    }
}

/** 分享进来的条目：照片是缩略图，文件是图标和名字，每项右上角能删。 */
@Composable
internal fun ShareItemStrip(items: List<ShareInItem>, onRemove: (String) -> Unit) {
    LazyRow(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        items(items, key = { it.id }) { item -> ShareItemTile(item) { onRemove(item.id) } }
    }
}

@Composable
private fun ShareItemTile(item: ShareInItem, onRemove: () -> Unit) {
    val colors = WontonTheme.colors
    val tile = WontonTheme.size.previewIcon
    val removeLabel = stringResource(R.string.share_in_remove_item, item.name)
    Column(Modifier.width(tile), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        Box(
            Modifier.size(tile)
                .clip(RoundedCornerShape(WontonTheme.radii.control))
                .background(colors.surfaceSubtle.toColor()),
        ) {
            val thumbnail = if (item.kind is AttachmentKind.Image) shareThumbnail(item.path) else null
            if (thumbnail != null) {
                Image(
                    thumbnail.asImageBitmap(),
                    contentDescription = item.name,
                    modifier = Modifier.fillMaxSize(),
                    contentScale = ContentScale.Crop,
                )
            } else {
                Icon(
                    if (item.kind is AttachmentKind.Image) WontonIcon.Image else WontonIcon.File,
                    // 文件的名字就在下面；照片解不出缩略图时，删除按钮的名字里有它。
                    contentDescription = null,
                    modifier = Modifier.align(Alignment.Center).size(WontonTheme.size.icon),
                    tint = colors.textSecondary.toColor(),
                )
            }
            Box(
                Modifier.align(Alignment.TopEnd)
                    .size(WontonTheme.size.smallButton)
                    .semantics { contentDescription = removeLabel }
                    .clickable(role = Role.Button, onClick = onRemove),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    WontonIcon.Close,
                    contentDescription = null,
                    modifier = Modifier.background(colors.surface.toColor(), CircleShape)
                        .padding(WontonTheme.spacing.s4 / 2)
                        .size(WontonTheme.size.iconTiny),
                    tint = colors.textPrimary.toColor(),
                )
            }
        }
        if (item.kind !is AttachmentKind.Image) {
            Text(
                item.name,
                style = WontonTheme.typography.caption,
                color = colors.textSecondary.toColor(),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

/** 收件目录里的照片按格子大小缩着解码，不把原图整张读进内存；解不出来时显示文件图标。 */
@Composable
private fun shareThumbnail(path: String): Bitmap? {
    val longerSide = with(LocalDensity.current) { WontonTheme.size.previewIcon.roundToPx() }
    val bitmap by produceState<Bitmap?>(null, path, longerSide) {
        value = withContext(Dispatchers.IO) { decodeScaled(path, longerSide) }
    }
    return bitmap
}

private fun decodeScaled(path: String, longerSide: Int): Bitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeFile(path, bounds)
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
    var sample = 1
    while (minOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= longerSide) sample *= 2
    return BitmapFactory.decodeFile(path, BitmapFactory.Options().apply { inSampleSize = sample })
}
