package app.wonton.ui.video

import android.icu.text.MeasureFormat
import android.icu.util.Measure
import android.icu.util.MeasureUnit
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.wonton.client.VideoEntry
import app.wonton.client.VideoEntryKind
import app.wonton.ui.R
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.CheckCircleFill
import app.wonton.ui.icons.WontonIcon
import java.util.Locale

private const val MS_PER_MINUTE = 60_000L
private const val MINUTES_PER_HOUR = 60L
private const val POSTER_RATIO = 2f / 3f
private const val LANDSCAPE_RATIO = 16f / 9f
private const val SCRIM_ALPHA = 0.55f
private const val TRACK_ALPHA = 0.45f
private val POSTER_RADIUS = 10.dp
private val LANDSCAPE_RADIUS = 12.dp
private val BAR_HEIGHT = 4.dp
private val BADGE_SIZE = 20.dp

/** 卡片和详情里的几种写法。 */
object VideoFormat {
    /** `S1 · E3`；只有集号时 `E3`。 */
    fun episodeLabel(entry: VideoEntry): String? {
        val season = entry.season
        val episode = entry.episode ?: return null
        return if (season != null) "S$season · E$episode" else "E$episode"
    }

    /** 片长或剩下多久，按系统语言写（“1小时42分钟”）。 */
    fun duration(milliseconds: ULong): String {
        val minutes = milliseconds.toLong() / MS_PER_MINUTE
        val format = MeasureFormat.getInstance(Locale.getDefault(), MeasureFormat.FormatWidth.SHORT)
        val hours = minutes / MINUTES_PER_HOUR
        return if (hours > 0) {
            format.formatMeasures(
                Measure(hours, MeasureUnit.HOUR),
                Measure(minutes % MINUTES_PER_HOUR, MeasureUnit.MINUTE),
            )
        } else {
            format.formatMeasures(Measure(minutes, MeasureUnit.MINUTE))
        }
    }

    @Composable
    fun remaining(entry: VideoEntry): String? {
        val duration = entry.durationMs
        val position = entry.positionMs
        return if (duration != null && position != null && position < duration) {
            stringResource(R.string.video_remaining, duration(duration - position))
        } else {
            null
        }
    }

    /** 卡片下面那行小字：单集写剧名和集号，其余写年份。 */
    fun caption(entry: VideoEntry): String? =
        if (entry.kind == VideoEntryKind.EPISODE) {
            listOfNotNull(entry.seriesTitle, episodeLabel(entry)).joinToString(" · ")
        } else {
            entry.year?.toString()
        }
}

/** 竖版海报卡：2:3 的图，片名和年份在下面。看完的角上打勾，剧集角上是没看的集数。 */
@Composable
fun VideoPosterCard(entry: VideoEntry, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val colors = WontonTheme.colors
    Column(
        modifier.clickable(onClick = onClick),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        Box(Modifier.fillMaxWidth().aspectRatio(POSTER_RATIO).clip(RoundedCornerShape(POSTER_RADIUS))) {
            VideoArtwork(entry.poster, Modifier.fillMaxWidth().aspectRatio(POSTER_RATIO)) {
                Text(
                    entry.title,
                    Modifier.align(Alignment.Center).padding(WontonTheme.spacing.s8),
                    style = WontonTheme.typography.caption.copy(textAlign = TextAlign.Center),
                    color = colors.textSecondary.toColor(),
                )
            }
            PosterBadge(entry, Modifier.align(Alignment.TopEnd).padding(WontonTheme.spacing.s4))
            entry.progress?.let {
                VideoProgressBar(it, Modifier.align(Alignment.BottomCenter).padding(WontonTheme.spacing.s8))
            }
        }
        Text(
            entry.title,
            style = WontonTheme.typography.supporting,
            color = colors.textPrimary.toColor(),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        VideoFormat.caption(entry)?.let {
            Text(
                it,
                style = WontonTheme.typography.caption,
                color = colors.textSecondary.toColor(),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun PosterBadge(entry: VideoEntry, modifier: Modifier) {
    val unplayed = entry.unplayedCount?.takeIf {
        it > 0u && (entry.kind == VideoEntryKind.SERIES || entry.kind == VideoEntryKind.SEASON)
    }
    if (entry.played) {
        Icon(
            WontonIcon.CheckCircleFill,
            contentDescription = null,
            modifier.size(BADGE_SIZE).background(Color.Black.copy(alpha = SCRIM_ALPHA), CircleShape),
            tint = Color.White,
        )
    } else if (unplayed != null) {
        Box(
            modifier.size(BADGE_SIZE).background(Color.Black.copy(alpha = SCRIM_ALPHA), CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                unplayed.toString(),
                style = WontonTheme.typography.caption.copy(fontWeight = FontWeight.Bold),
                color = Color.White,
            )
        }
    }
}

/** 横版卡：16:9 的剧照，片名、集号和还剩多久在下面。“继续观看”“下一集”用它。 */
@Composable
fun VideoLandscapeCard(entry: VideoEntry, width: Dp, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    val title = if (entry.kind == VideoEntryKind.EPISODE) entry.seriesTitle ?: entry.title else entry.title
    val parts = buildList {
        if (entry.kind == VideoEntryKind.EPISODE) {
            VideoFormat.episodeLabel(entry)?.let(::add)
            add(entry.title)
        }
        if (entry.progress != null) VideoFormat.remaining(entry)?.let(::add)
    }
    Column(
        Modifier.width(width).clickable(onClick = onClick),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        Box(Modifier.fillMaxWidth().aspectRatio(LANDSCAPE_RATIO).clip(RoundedCornerShape(LANDSCAPE_RADIUS))) {
            VideoArtwork(entry.landscape ?: entry.backdrop, Modifier.fillMaxWidth().aspectRatio(LANDSCAPE_RATIO))
            entry.progress?.let {
                VideoProgressBar(it, Modifier.align(Alignment.BottomCenter).padding(WontonTheme.spacing.s8))
            }
        }
        Text(
            title,
            style = WontonTheme.typography.supporting.copy(fontWeight = FontWeight.Medium),
            color = colors.textPrimary.toColor(),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            parts.joinToString(" · "),
            style = WontonTheme.typography.caption,
            color = colors.textSecondary.toColor(),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** 卡片底部的细进度条。 */
@Composable
fun VideoProgressBar(progress: Float, modifier: Modifier = Modifier) {
    Box(
        modifier.fillMaxWidth().height(BAR_HEIGHT).clip(CircleShape)
            .background(Color.Black.copy(alpha = TRACK_ALPHA)),
    ) {
        Box(Modifier.fillMaxWidth(progress.coerceIn(0f, 1f)).height(BAR_HEIGHT).background(Color.White))
    }
}

/** 一排横着滑的卡，标题右边可以有“全部”。 */
@Composable
fun VideoShelfRow(
    title: String,
    entries: List<VideoEntry>,
    seeAll: (() -> Unit)? = null,
    card: @Composable (VideoEntry) -> Unit,
) {
    val colors = WontonTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = WontonTheme.spacing.pageInset),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                title,
                Modifier.weight(1f).semantics { heading() },
                style = WontonTheme.typography.sectionTitle,
                color = colors.textPrimary.toColor(),
            )
            seeAll?.let {
                Text(
                    stringResource(R.string.video_see_all),
                    Modifier.clickable(onClick = it).padding(WontonTheme.spacing.s8),
                    style = WontonTheme.typography.supporting,
                    color = colors.textSecondary.toColor(),
                )
            }
        }
        LazyRow(
            contentPadding = PaddingValues(horizontal = WontonTheme.spacing.pageInset),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            items(entries, key = { it.id }) { card(it) }
        }
    }
}
