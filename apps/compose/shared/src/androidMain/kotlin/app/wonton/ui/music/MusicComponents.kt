@file:Suppress("TooManyFunctions") // 音乐页共用的小件（封面、各种行、段标题）放在一起，拆开反而难找。
package app.wonton.ui.music

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.wonton.protocol.AlbumTile
import app.wonton.protocol.ArtistRow
import app.wonton.protocol.GenreRow
import app.wonton.protocol.MusicPage
import app.wonton.protocol.MusicTarget
import app.wonton.protocol.PlaylistRow
import app.wonton.protocol.TrackRow
import app.wonton.ui.R
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Music
import app.wonton.ui.icons.StarFill
import app.wonton.ui.icons.WontonIcon

private const val PLACEHOLDER_FRACTION = 0.36f
private const val PLACEHOLDER_ALPHA = 0.5f
private const val OUTLINE_ALPHA = 0.5f
private const val THUMBNAIL = 44
private const val ARTIST_AVATAR = 48
private const val PLAYLIST_THUMBNAIL = 56
private const val TRACK_NUMBER_WIDTH = 24

/** 封面：取到之前是带音符的底色块，取不到也一直是它（插件没给封面、封面坏了）。边长跟着放它的格子。 */
@Composable
internal fun MusicCover(cover: String?, modifier: Modifier = Modifier, round: Boolean = false) {
    val music = LocalMusic.current
    val colors = WontonTheme.colors
    var pixels by remember { mutableIntStateOf(0) }
    val image by produceState(
        cover?.takeIf { pixels > 0 }?.let { MusicCovers.cached(music.pluginId, it, pixels) }, cover, pixels,
    ) {
        if (cover != null && pixels > 0) value = music.cover(cover, pixels)
    }
    val shape = if (round) CircleShape else RoundedCornerShape(WontonTheme.radii.webXs)
    Box(
        modifier.aspectRatio(1f).clip(shape).background(colors.surfaceSubtle.toColor())
            .border(WontonTheme.size.hairline, colors.separator.toColor().copy(alpha = OUTLINE_ALPHA), shape)
            .onSizeChanged { pixels = MusicCovers.pixels(it.width) },
        contentAlignment = Alignment.Center,
    ) {
        val loaded = image
        if (loaded != null) {
            Image(loaded.asImageBitmap(), null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
        } else {
            Icon(
                WontonIcon.Music,
                // 装饰：旁边就是名字。
                contentDescription = null,
                Modifier.fillMaxSize(PLACEHOLDER_FRACTION),
                tint = colors.textSecondary.toColor().copy(alpha = PLACEHOLDER_ALPHA),
            )
        }
    }
}

/**
 * 能长按收藏的一块：长按弹出收藏或取消收藏。[onClick] 为空时只认长按（曲目行在接上播放之前点了没反应）。
 */
@Composable
internal fun MusicStarMenuBox(
    starred: Boolean, target: MusicTarget, onClick: (() -> Unit)?, modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    val music = LocalMusic.current
    var menu by remember { mutableStateOf(false) }
    val gesture = if (onClick != null) {
        Modifier.combinedClickable(role = Role.Button, onLongClick = { menu = true }, onClick = onClick)
    } else {
        Modifier.pointerInput(Unit) { detectTapGestures(onLongPress = { menu = true }) }
    }
    Box(modifier.then(gesture)) {
        content()
        DropdownMenu(menu, { menu = false }, containerColor = WontonTheme.colors.surface.toColor()) {
            DropdownMenuItem(
                text = { Text(stringResource(if (starred) R.string.music_unstar else R.string.music_star)) },
                onClick = {
                    menu = false
                    music.star(target, !starred)
                },
            )
        }
    }
}

/** 封面墙和横排里的一张专辑：点开是专辑页，长按收藏。歌手页上每张都是同一个歌手，第二行改写年份（[showsYear]）。 */
@Composable
internal fun MusicAlbumTile(album: AlbumTile, modifier: Modifier = Modifier, showsYear: Boolean = false) {
    val music = LocalMusic.current
    MusicStarMenuBox(
        album.starred, MusicTarget.Album(album.id), { music.open(MusicPage.Album(album.id)) }, modifier,
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            MusicCover(album.cover, Modifier.fillMaxWidth())
            MusicLine(album.title, secondary = false)
            (if (showsYear) album.year?.toString() else album.artist)?.let { MusicLine(it, secondary = true) }
        }
    }
}

/** 一首歌：曲序（专辑页里）或封面、歌名、署名、时长。播放接上之前不能点。 */
@Composable
internal fun MusicTrackRow(track: TrackRow, inAlbum: Boolean = false) {
    val colors = WontonTheme.colors
    val subtitle = if (inAlbum) track.artist else listOfNotNull(track.artist, track.album).joinToString(" · ")
    MusicStarMenuBox(track.starred, MusicTarget.Song(track.id), onClick = null, Modifier.fillMaxWidth()) {
        Row(
            Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target).padding(vertical = WontonTheme.spacing.s4),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (inAlbum) {
                Text(
                    track.track?.toString().orEmpty(), Modifier.width(TRACK_NUMBER_WIDTH.dp),
                    style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor(),
                )
            } else {
                MusicCover(track.cover, Modifier.size(THUMBNAIL.dp))
            }
            Column(Modifier.weight(1f)) {
                MusicLine(track.title, secondary = false, style = WontonTheme.typography.body)
                subtitle?.takeIf { it.isNotEmpty() }?.let { MusicLine(it, secondary = true) }
            }
            if (track.starred) {
                Icon(
                    WontonIcon.StarFill,
                    // 装饰：收藏的状态在长按菜单里也读得到。
                    contentDescription = null,
                    Modifier.size(WontonTheme.size.iconTiny),
                    tint = colors.textSecondary.toColor(),
                )
            }
            Text(
                MusicFormat.clock(track.durationMs),
                style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor(),
            )
        }
    }
}

/** 一位歌手：圆头像、名字、几张专辑。 */
@Composable
internal fun MusicArtistRow(artist: ArtistRow) {
    val music = LocalMusic.current
    MusicStarMenuBox(
        artist.starred, MusicTarget.Artist(artist.id), { music.open(MusicPage.Artist(artist.id)) },
        Modifier.fillMaxWidth(),
    ) {
        MusicRowLayout(
            { MusicCover(artist.cover, Modifier.size(ARTIST_AVATAR.dp), round = true) },
            artist.name,
            pluralStringResource(R.plurals.music_album_count, artist.albumCount.toInt(), artist.albumCount.toInt()),
        )
    }
}

/** 一张歌单：封面、名字、谁的（别人的才写）和几首。 */
@Composable
internal fun MusicPlaylistRow(playlist: PlaylistRow) {
    val music = LocalMusic.current
    Box(Modifier.fillMaxWidth().clickable(role = Role.Button) { music.open(MusicPage.Playlist(playlist.id)) }) {
        MusicRowLayout(
            { MusicCover(playlist.cover, Modifier.size(PLAYLIST_THUMBNAIL.dp)) },
            playlist.name,
            MusicFormat.playlistLine(playlist),
        )
    }
}

@Composable
internal fun MusicGenreRow(genre: GenreRow) {
    val music = LocalMusic.current
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .clickable(role = Role.Button) { music.open(MusicPage.Genre(genre.name)) },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(genre.name, Modifier.weight(1f), style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
        Text(
            pluralStringResource(R.plurals.music_album_count, genre.albumCount.toInt(), genre.albumCount.toInt()),
            style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor(),
        )
    }
}

/** 首页、收藏和搜索里的一段：标题，可选的“全部”入口。 */
@Composable
internal fun MusicSectionHeader(title: String, all: MusicPage? = null) {
    val music = LocalMusic.current
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth().padding(horizontal = WontonTheme.spacing.pageInset),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(title, Modifier.weight(1f), style = WontonTheme.typography.sectionTitle,
            color = colors.textPrimary.toColor())
        all?.let { page ->
            Text(
                stringResource(R.string.music_see_all),
                Modifier.clip(RoundedCornerShape(WontonTheme.radii.webXs))
                    .clickable(role = Role.Button) { music.open(page) }
                    .padding(WontonTheme.spacing.s4),
                style = WontonTheme.typography.supporting, color = colors.textPrimary.toColor(),
            )
        }
    }
}

@Composable
private fun MusicRowLayout(leading: @Composable () -> Unit, title: String, subtitle: String) {
    Row(
        Modifier.fillMaxWidth().padding(vertical = WontonTheme.spacing.s4),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        leading()
        Column(Modifier.weight(1f)) {
            MusicLine(title, secondary = false, style = WontonTheme.typography.body)
            MusicLine(subtitle, secondary = true)
        }
    }
}

/** 一行字，超长截断。 */
@Composable
internal fun MusicLine(text: String, secondary: Boolean, style: TextStyle? = null) {
    val colors = WontonTheme.colors
    Text(
        text,
        style = style ?: WontonTheme.typography.supporting,
        color = (if (secondary) colors.textSecondary else colors.textPrimary).toColor(),
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
    )
}
