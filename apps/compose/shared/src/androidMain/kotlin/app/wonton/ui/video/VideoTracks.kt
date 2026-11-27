package app.wonton.ui.video

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.wonton.client.VideoDetail
import app.wonton.protocol.MediaTrack
import app.wonton.ui.R
import app.wonton.ui.design.WontonRadioRow
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Music
import app.wonton.ui.icons.WontonIcon
import java.util.Locale

private val CHIP_ICON = 16.dp

/** 菜单里显示的名字：提供方的标签，没有就用系统的语言名。 */
private val MediaTrack.name: String
    get() = label?.takeIf { it.isNotEmpty() }
        ?: Locale.forLanguageTag(language).getDisplayName(Locale.getDefault()).takeIf { it.isNotEmpty() }
        ?: language

/** 音轨和字幕两个胶囊，点开是底部面板选一条。起播时按选的告诉核心。 */
@Composable
fun VideoTrackChips(
    detail: VideoDetail,
    audio: UShort?,
    subtitle: UShort?,
    onAudio: (UShort?) -> Unit,
    onSubtitle: (UShort?) -> Unit,
) {
    var picking by remember { mutableStateOf<Picking?>(null) }
    val off = stringResource(R.string.video_subtitles_off)
    Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        if (detail.audio.size > 1) {
            Chip(WontonIcon.Music, detail.audio.firstOrNull { it.index == audio }?.name.orEmpty()) {
                picking = Picking.AUDIO
            }
        }
        if (detail.subtitles.isNotEmpty()) {
            Chip(null, detail.subtitles.firstOrNull { it.index == subtitle }?.name ?: off) {
                picking = Picking.SUBTITLE
            }
        }
    }
    when (picking) {
        Picking.AUDIO -> TrackSheet(stringResource(R.string.video_audio), detail.audio, audio, null, onAudio) {
            picking = null
        }
        Picking.SUBTITLE -> {
            val title = stringResource(R.string.video_subtitles)
            TrackSheet(title, detail.subtitles, subtitle, off, onSubtitle) { picking = null }
        }
        null -> Unit
    }
}

private enum class Picking { AUDIO, SUBTITLE }

@Composable
private fun Chip(icon: ImageVector?, text: String, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    Row(
        Modifier.background(colors.surfaceSubtle.toColor(), CircleShape).clickable(onClick = onClick)
            .padding(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s8),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        icon?.let {
            // 装饰：胶囊的文字就是选中的那条。
            Icon(it, contentDescription = null, Modifier.size(CHIP_ICON), tint = colors.textPrimary.toColor())
        }
        Text(
            text,
            style = WontonTheme.typography.supporting,
            color = colors.textPrimary.toColor(),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** 选一条轨道；[none] 不为空时第一行是“关”（字幕）。 */
@Suppress("LongParameterList") // 音轨和字幕共用一个面板，差别都在参数里。
@Composable
private fun TrackSheet(
    title: String,
    tracks: List<MediaTrack>,
    selected: UShort?,
    none: String?,
    onSelect: (UShort?) -> Unit,
    onDismiss: () -> Unit,
) {
    WontonSheet(onDismiss, title) { close ->
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState())
                .padding(horizontal = WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        ) {
            none?.let {
                WontonRadioRow(it, selected == null, true, { close { onSelect(null) } })
            }
            tracks.forEach { track ->
                WontonRadioRow(track.name, selected == track.index, true, { close { onSelect(track.index) } })
            }
        }
    }
}
