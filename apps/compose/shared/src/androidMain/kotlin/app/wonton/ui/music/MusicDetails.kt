package app.wonton.ui.music

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import app.wonton.protocol.AlbumDetail
import app.wonton.protocol.ArtistDetail
import app.wonton.protocol.DiscSection
import app.wonton.protocol.MusicPage
import app.wonton.protocol.MusicTarget
import app.wonton.protocol.PlaylistDetail
import app.wonton.ui.Problem
import app.wonton.ui.ProblemRow
import app.wonton.ui.R
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Star
import app.wonton.ui.icons.StarFill
import app.wonton.ui.icons.WontonIcon

private const val HEADER_COVER = 240
private const val HEADER_AVATAR = 180
private const val HEADER_SHADOW = 16

/** 专辑、歌手、歌单页的头里要的几样；[openSubtitle] 有值时副标题能点（专辑的歌手）。 */
private class MusicHeading(
    val cover: String?,
    val title: String,
    val round: Boolean = false,
    val subtitle: String? = null,
    val openSubtitle: (() -> Unit)? = null,
    val caption: String? = null,
    val star: Pair<Boolean, MusicTarget>? = null,
)

/** 页头：大封面、标题、副标题、一行说明，收藏按钮在最下面。 */
@Composable
private fun MusicHeader(heading: MusicHeading) {
    val colors = WontonTheme.colors
    val side = if (heading.round) HEADER_AVATAR else HEADER_COVER
    val shape = if (heading.round) CircleShape else RoundedCornerShape(WontonTheme.radii.webXs)
    Column(
        Modifier.fillMaxWidth().padding(bottom = WontonTheme.spacing.s8),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        MusicCover(
            heading.cover,
            Modifier.size(side.dp).shadow(HEADER_SHADOW.dp, shape),
            round = heading.round,
        )
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        ) {
            Text(heading.title, style = WontonTheme.typography.sectionTitle.copy(textAlign = TextAlign.Center),
                color = colors.textPrimary.toColor())
            heading.subtitle?.let { subtitle ->
                val open = heading.openSubtitle
                Text(
                    subtitle,
                    if (open != null) Modifier.clickable(role = Role.Button, onClick = open) else Modifier,
                    style = WontonTheme.typography.itemTitle.copy(textAlign = TextAlign.Center),
                    color = (if (open != null) colors.textPrimary else colors.textSecondary).toColor(),
                )
            }
            heading.caption?.let {
                Text(it, style = WontonTheme.typography.caption.copy(textAlign = TextAlign.Center),
                    color = colors.textSecondary.toColor())
            }
        }
        heading.star?.let { (starred, target) -> MusicStarButton(starred, target) }
    }
}

/** 页头的收藏按钮：收藏过的写“已收藏”，点了取消。 */
@Composable
private fun MusicStarButton(starred: Boolean, target: MusicTarget) {
    val music = LocalMusic.current
    val colors = WontonTheme.colors
    Row(
        Modifier.clip(RoundedCornerShape(WontonTheme.radii.pill)).background(colors.surfaceSubtle.toColor())
            .clickable(role = Role.Button) { music.star(target, !starred) }
            .heightIn(min = WontonTheme.size.smallButton)
            .padding(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s4),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            if (starred) WontonIcon.StarFill else WontonIcon.Star,
            // 装饰：文字就是按钮的意思。
            contentDescription = null,
            Modifier.size(WontonTheme.size.iconSmall),
            tint = colors.textPrimary.toColor(),
        )
        Text(
            stringResource(if (starred) R.string.music_starred_label else R.string.music_star),
            style = WontonTheme.typography.supporting, color = colors.textPrimary.toColor(),
        )
    }
}

/** 专辑页：曲目按碟分段，只有一碟时不写碟号；最底下一行几首、多少分钟。 */
@Composable
internal fun MusicAlbumView(detail: AlbumDetail, notice: Problem?, reload: () -> Unit) {
    val music = LocalMusic.current
    val album = detail.album
    val caption = listOfNotNull(detail.genre, album.year?.toString()).joinToString(" · ").ifEmpty { null }
    val heading = MusicHeading(
        album.cover, album.title, subtitle = album.artist,
        openSubtitle = album.artistId?.let { id -> { music.open(MusicPage.Artist(id)) } },
        caption = caption, star = album.starred to MusicTarget.Album(album.id),
    )
    MusicList(notice, reload) {
        item(key = "header") { MusicHeader(heading) }
        detail.discs.forEachIndexed { index, disc -> discRows(index, disc) }
        summary { MusicFormat.summary(detail.songCount, detail.durationMs) }
    }
}

/** 一碟：碟名是普通的一行，跟着曲目滚走。 */
private fun LazyListScope.discRows(index: Int, disc: DiscSection) {
    disc.disc?.let { number ->
        item(key = "disc-$index") {
            val label = stringResource(R.string.music_disc, number.toInt())
            Text(
                disc.title?.let { "$label · $it" } ?: label,
                Modifier.padding(top = WontonTheme.spacing.s16, bottom = WontonTheme.spacing.s4),
                style = WontonTheme.typography.supporting, color = WontonTheme.colors.textSecondary.toColor(),
            )
        }
    }
    items(disc.tracks, key = { "$index/${it.id}" }) { MusicTrackRow(it, inAlbum = true) }
}

private fun LazyListScope.summary(text: @Composable () -> String) {
    item(key = "summary") {
        Text(
            text(), Modifier.padding(top = WontonTheme.spacing.s12),
            style = WontonTheme.typography.caption, color = WontonTheme.colors.textSecondary.toColor(),
        )
    }
}

/** 歌手页：头像、名字，下面是他的专辑墙，第二行写年份。 */
@Composable
internal fun MusicArtistView(detail: ArtistDetail, notice: Problem?, reload: () -> Unit) {
    val artist = detail.artist
    val heading = MusicHeading(
        artist.cover, artist.name, round = true,
        caption = artist.albumCount.toInt().let { pluralStringResource(R.plurals.music_album_count, it, it) },
        star = artist.starred to MusicTarget.Artist(artist.id),
    )
    MusicGrid {
        notice?.let { fullLine("notice") { ProblemRow(it, reload) } }
        fullLine("header") { MusicHeader(heading) }
        items(detail.albums.size, key = { detail.albums[it].id }) {
            MusicAlbumTile(detail.albums[it], showsYear = true)
        }
    }
}

/** 歌单页：封面、名字、谁的、说明，下面是歌。 */
@Composable
internal fun MusicPlaylistView(detail: PlaylistDetail, notice: Problem?, reload: () -> Unit) {
    val playlist = detail.playlist
    val heading = MusicHeading(playlist.cover, playlist.name, subtitle = playlist.owner, caption = detail.comment)
    MusicList(notice, reload) {
        item(key = "header") { MusicHeader(heading) }
        // 同一首歌能在歌单里出现两次：按位置认行。
        itemsIndexed(detail.tracks, key = { index, _ -> index }) { _, track -> MusicTrackRow(track) }
        summary { MusicFormat.summary(playlist.songCount, playlist.durationMs) }
    }
}
