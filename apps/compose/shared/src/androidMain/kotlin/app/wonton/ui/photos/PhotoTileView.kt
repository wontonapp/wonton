package app.wonton.ui.photos

import androidx.compose.animation.core.animateDpAsState
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.wonton.protocol.MediaKind
import app.wonton.protocol.PhotoTile
import app.wonton.protocol.PhotoTileOwner
import app.wonton.ui.R
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.PlayFill
import app.wonton.ui.icons.StarFill
import app.wonton.ui.icons.WontonIcon

/** 压在照片上的角标底色：照片什么颜色都有，主题色保证不了对比，固定用半透明黑底白字。 */
internal val PhotoScrim = Color.Black.copy(alpha = 0.45f)
private val BADGE_INSET = 4.dp
private val BADGE_ICON = 12.dp
private val CHECK_SIZE = 22.dp
private val SELECTED_INSET = 10.dp
private const val LIVE_RING = 0.36f
private const val LIVE_DOT = 0.14f

/** 网格里的一格：占位、缩略图、角标（视频时长、实况、收藏、是谁的），多选时左上角的勾。 */
@Composable
internal fun PhotoTileView(
    tile: PhotoTile,
    ref: PhotoRef,
    selecting: Boolean,
    selected: Boolean,
    modifier: Modifier = Modifier,
) {
    val colors = WontonTheme.colors
    val inset by animateDpAsState(if (selected) SELECTED_INSET else 0.dp, label = "tile-selected")
    Box(modifier.aspectRatio(1f).background(colors.surfaceSubtle.toColor())) {
        Box(
            Modifier.fillMaxSize().padding(inset)
                .clip(RoundedCornerShape(if (selected) WontonTheme.radii.control else 0.dp)),
        ) {
            PhotoImage(ref, tile.thumbhash, tileLabel(tile), Modifier.fillMaxSize())
            TileBadges(tile)
        }
        if (selecting) SelectionMark(selected, Modifier.align(Alignment.TopStart).padding(BADGE_INSET), onPhoto = true)
    }
}

@Composable
private fun tileLabel(tile: PhotoTile): String {
    val day = takenAtText(LocalContext.current, tile.takenAt)
    return when (tile.kind) {
        MediaKind.Video -> stringResource(R.string.photo_library_video_from, day)
        else -> stringResource(R.string.photo_library_photo_from, day)
    }
}

@Composable
private fun BoxScope.TileBadges(tile: PhotoTile) {
    Row(
        Modifier.align(Alignment.BottomStart).padding(BADGE_INSET),
        horizontalArrangement = Arrangement.spacedBy(BADGE_INSET),
    ) {
        if (tile.kind == MediaKind.Video) {
            Badge(WontonIcon.PlayFill, tile.durationMs?.let { durationText(it) })
        }
        if (tile.live) LiveBadge()
    }
    Row(
        Modifier.align(Alignment.BottomEnd).padding(BADGE_INSET),
        horizontalArrangement = Arrangement.spacedBy(BADGE_INSET),
    ) {
        when (val owner = tile.owner) {
            is PhotoTileOwner.Person -> Badge(null, owner.user.name)
            // 别人的，但不是这台服务器上的哪位家人：没有名字可写，只标不是自己的。
            PhotoTileOwner.Unknown -> Badge(null, stringResource(R.string.photo_library_owner_unknown))
            PhotoTileOwner.Me -> Unit
        }
        if (tile.favorite) Badge(WontonIcon.StarFill, null)
    }
}

@Composable
private fun Badge(icon: ImageVector?, text: String?) {
    Row(
        Modifier.background(PhotoScrim, CircleShape).padding(horizontal = 6.dp, vertical = 3.dp),
        horizontalArrangement = Arrangement.spacedBy(3.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // 装饰：格子的读屏名在图片上。
        icon?.let { Icon(it, contentDescription = null, Modifier.size(BADGE_ICON), tint = Color.White) }
        text?.let { Text(it, style = WontonTheme.typography.messageMeta, color = Color.White, maxLines = 1) }
    }
}

/** 实况照片的记号：一个圆环套一个点。 */
@Composable
internal fun LiveBadge(size: Dp = BADGE_ICON) {
    Box(Modifier.background(PhotoScrim, CircleShape).padding(3.dp)) {
        Canvas(Modifier.size(size)) {
            val radius = this.size.minDimension
            drawCircle(Color.White, radius * LIVE_RING, style = Stroke(width = radius / BADGE_ICON.value))
            drawCircle(Color.White, radius * LIVE_DOT)
        }
    }
}

/**
 * 多选时的圈：选中是主色底加勾，没选中是空心圈。[onPhoto] 是不是压在照片上：压在照片上的空心圈带半透明黑底和白边，
 * 排在页面底色上的（日期标题、面板里的行）用主题的次要文字色描边。
 */
@Composable
internal fun SelectionMark(selected: Boolean, modifier: Modifier = Modifier, onPhoto: Boolean = false) {
    val colors = WontonTheme.colors
    val shape = CircleShape
    Box(
        modifier.size(CHECK_SIZE).then(
            when {
                selected -> Modifier.background(colors.primary.toColor(), shape)
                onPhoto -> Modifier.background(PhotoScrim, shape).border(1.5.dp, Color.White, shape)
                else -> Modifier.border(1.5.dp, colors.textSecondary.toColor(), shape)
            },
        ),
        contentAlignment = Alignment.Center,
    ) {
        if (selected) {
            // 装饰：选没选中写在格子的语义里。
            Icon(
                WontonIcon.Check,
                contentDescription = null,
                Modifier.size(WontonTheme.size.iconTiny),
                tint = colors.onPrimary.toColor(),
            )
        }
    }
}
