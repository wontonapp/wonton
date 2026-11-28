package app.wonton.ui.music

import androidx.annotation.StringRes
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.wonton.media.MusicPlayback
import app.wonton.ui.ProblemDialog
import app.wonton.ui.R
import app.wonton.ui.design.LocalWontonBottomInset
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Back
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.Music
import app.wonton.ui.icons.PauseFill
import app.wonton.ui.icons.PlayFill
import app.wonton.ui.icons.WontonIcon

/** 迷你播放条连上下留白占的高度：内容的底部按它让开。 */
private val MINI_HEIGHT = 72.dp
private val MINI_ARTWORK = 40.dp
private val MINI_ELEVATION = 6.dp
private val PLAY_LARGE = 64.dp
private val SKIP_LARGE = 36.dp
private const val PLACEHOLDER_FRACTION = 0.4f

/** 音乐页的内容和底部的迷你播放条：有在放的才出现，内容的底部留白跟着让出它。点开是播放页。 */
@Composable
internal fun MusicPlayerFrame(content: @Composable () -> Unit) {
    val active = MusicPlayback.isActive
    val bottom = LocalWontonBottomInset.current
    var showsPlayer by rememberSaveable { mutableStateOf(false) }
    Box(Modifier.fillMaxSize()) {
        CompositionLocalProvider(LocalWontonBottomInset provides if (active) bottom + MINI_HEIGHT else bottom) {
            content()
        }
        if (active) {
            MusicMiniPlayer(Modifier.align(Alignment.BottomCenter).padding(bottom = bottom)) { showsPlayer = true }
        }
    }
    if (showsPlayer && active) MusicNowPlaying { showsPlayer = false }
    MusicPlayback.failure?.let { failure -> ProblemDialog(failure.problem) { MusicPlayback.failure = null } }
}

@Composable
private fun MusicMiniPlayer(modifier: Modifier, open: () -> Unit) {
    val item = MusicPlayback.view?.session?.item ?: return
    val colors = WontonTheme.colors
    val spacing = WontonTheme.spacing
    val shape = RoundedCornerShape(WontonTheme.radii.card)
    Row(
        modifier.padding(horizontal = spacing.pageInset, vertical = spacing.s8).fillMaxWidth()
            .shadow(MINI_ELEVATION, shape)
            .clip(shape)
            .background(colors.surface.toColor())
            .clickable(role = Role.Button, onClickLabel = stringResource(R.string.music_now_playing), onClick = open)
            .padding(start = spacing.s8),
        horizontalArrangement = Arrangement.spacedBy(spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        MusicArtwork(Modifier.padding(vertical = spacing.s8).size(MINI_ARTWORK))
        Column(Modifier.weight(1f)) {
            MusicLine(item.title, secondary = false, style = WontonTheme.typography.body)
            item.artist?.let { MusicLine(it, secondary = true) }
        }
        MusicPlayPause(WontonTheme.size.icon)
        MusicPlayerButton(WontonIcon.Forward, R.string.music_next, WontonTheme.size.icon, MusicPlayback::next)
    }
}

/** 播放页：大封面、歌名、进度和三个按钮。 */
@Composable
private fun MusicNowPlaying(onDismiss: () -> Unit) {
    val item = MusicPlayback.view?.session?.item ?: return
    val colors = WontonTheme.colors
    val spacing = WontonTheme.spacing
    WontonSheet(onDismiss) {
        Column(
            Modifier.fillMaxWidth().padding(horizontal = spacing.s32, vertical = spacing.s16),
            verticalArrangement = Arrangement.spacedBy(spacing.s24),
        ) {
            MusicArtwork(Modifier.fillMaxWidth())
            Column {
                Text(
                    item.title, style = WontonTheme.typography.sectionTitle, color = colors.textPrimary.toColor(),
                    maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
                (item.artist ?: item.album)?.let {
                    Text(
                        it, style = WontonTheme.typography.body, color = colors.textSecondary.toColor(),
                        maxLines = 1, overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            MusicProgress()
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceEvenly,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                MusicPlayerButton(WontonIcon.Back, R.string.music_previous, SKIP_LARGE, MusicPlayback::previous)
                MusicPlayPause(PLAY_LARGE)
                MusicPlayerButton(WontonIcon.Forward, R.string.music_next, SKIP_LARGE, MusicPlayback::next)
            }
        }
    }
}

/** 进度条和两头的时间；拖的时候只动条，松手才跳。 */
@Composable
private fun MusicProgress() {
    val colors = WontonTheme.colors
    val duration = MusicPlayback.durationMs.coerceAtLeast(1L)
    var scrubbing by remember { mutableStateOf<Float?>(null) }
    val position = scrubbing ?: MusicPlayback.positionMs.coerceIn(0L, duration).toFloat()
    Column {
        Slider(
            value = position,
            onValueChange = { scrubbing = it },
            valueRange = 0f..duration.toFloat(),
            onValueChangeFinished = {
                scrubbing?.let { MusicPlayback.seek(it.toLong()) }
                scrubbing = null
            },
            colors = SliderDefaults.colors(
                thumbColor = colors.textPrimary.toColor(),
                activeTrackColor = colors.textPrimary.toColor(),
                inactiveTrackColor = colors.separator.toColor(),
            ),
        )
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            val style = WontonTheme.typography.supporting
            val tint = colors.textSecondary.toColor()
            Text(MusicFormat.clock(position.toLong().toULong()), style = style, color = tint)
            val left = (duration - position.toLong()).coerceAtLeast(0L).toULong()
            Text("-" + MusicFormat.clock(left), style = style, color = tint)
        }
    }
}

/** 播放和暂停是同一个按钮。 */
@Composable
private fun MusicPlayPause(side: Dp) {
    val playing = MusicPlayback.isPlaying
    MusicPlayerButton(
        if (playing) WontonIcon.PauseFill else WontonIcon.PlayFill,
        if (playing) R.string.music_pause else R.string.music_play,
        side,
        MusicPlayback::togglePlayPause,
    )
}

@Composable
private fun MusicPlayerButton(icon: ImageVector, @StringRes label: Int, side: Dp, onClick: () -> Unit) {
    val text = stringResource(label)
    Box(
        Modifier.size(maxOf(side, WontonTheme.size.target))
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = text },
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            icon,
            // 装饰：外层已经带了名字。
            contentDescription = null,
            Modifier.size(side),
            tint = WontonTheme.colors.textPrimary.toColor(),
        )
    }
}

/** 正在放的那一项的封面：核心找同一家提供方取的，取不到画占位。 */
@Composable
private fun MusicArtwork(modifier: Modifier) {
    val colors = WontonTheme.colors
    val image = MusicPlayback.artwork
    Box(
        modifier.aspectRatio(1f).clip(RoundedCornerShape(WontonTheme.radii.control))
            .background(colors.surfaceSubtle.toColor()),
        contentAlignment = Alignment.Center,
    ) {
        if (image != null) {
            Image(image.asImageBitmap(), null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
        } else {
            Icon(
                WontonIcon.Music,
                // 装饰：旁边就是歌名。
                contentDescription = null,
                Modifier.fillMaxSize(PLACEHOLDER_FRACTION),
                tint = colors.textSecondary.toColor(),
            )
        }
    }
}
