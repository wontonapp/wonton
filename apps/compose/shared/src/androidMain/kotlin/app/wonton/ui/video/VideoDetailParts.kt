package app.wonton.ui.video

import android.text.format.Formatter
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.wonton.client.OfflineState
import app.wonton.client.VideoDetail
import app.wonton.client.VideoEntry
import app.wonton.client.VideoEntryKind
import app.wonton.client.VideoMediaInfo
import app.wonton.client.VideoPerson
import app.wonton.client.VideoRole
import app.wonton.ui.ProblemRow
import app.wonton.ui.R
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.CheckCircleFill
import app.wonton.ui.icons.Person
import app.wonton.ui.icons.WontonIcon
import java.util.Locale

private val HERO_HEIGHT = 300.dp
private val LOGO_WIDTH = 240.dp
private val LOGO_HEIGHT = 80.dp
private val EPISODE_WIDTH = 144.dp
private val EPISODE_BADGE = 20.dp
private val EPISODE_HEIGHT = 81.dp
private val EPISODE_RADIUS = 8.dp
private val PERSON_PHOTO = 72.dp
private val PERSON_WIDTH = 84.dp
private val BADGE_RADIUS = 4.dp
private const val HERO_FADE_START = 0.35f
private const val SCRIM_ALPHA = 0.55f
private const val HERO_TOP_SCRIM = 0.7f
/** 顶上那层底色渐隐到图高的这么多：盖住状态栏和导航栏那一截。 */
private const val HERO_TOP_SCRIM_END = 0.4f
private const val UHD_WIDTH = 3200u
private const val FHD_WIDTH = 1800u
private const val HD_WIDTH = 1200u
private const val STEREO = 2
private const val MAX_GENRES = 3

/** 详情顶上的大图：背景图往下渐隐到底色，片名（有台标图用台标图）压在下沿。 */
@Composable
fun VideoHero(entry: VideoEntry) {
    val canvas = WontonTheme.colors.canvas.toColor()
    Box(Modifier.fillMaxWidth().height(HERO_HEIGHT)) {
        VideoArtwork(entry.backdrop ?: entry.landscape ?: entry.poster, Modifier.fillMaxWidth().height(HERO_HEIGHT))
        Box(
            Modifier.fillMaxWidth().height(HERO_HEIGHT).background(
                Brush.verticalGradient(HERO_FADE_START to canvas.copy(alpha = 0f), 1f to canvas),
            ),
        )
        // 状态栏图标和返回键按主题配色（浅色主题是深色），背景图顶上多是暗的：压一层底色，两种主题都看得清。
        Box(
            Modifier.fillMaxWidth().height(HERO_HEIGHT).background(
                Brush.verticalGradient(
                    0f to canvas.copy(alpha = HERO_TOP_SCRIM),
                    HERO_TOP_SCRIM_END to canvas.copy(alpha = 0f),
                ),
            ),
        )
        Box(Modifier.align(Alignment.BottomStart).padding(horizontal = WontonTheme.spacing.pageInset)) {
            val logo = entry.logo
            if (logo != null) {
                VideoArtwork(logo, Modifier.widthIn(max = LOGO_WIDTH).height(LOGO_HEIGHT), fit = true) {
                    HeroTitle(entry)
                }
            } else {
                HeroTitle(entry)
            }
        }
    }
}

@Composable
private fun HeroTitle(entry: VideoEntry) {
    val colors = WontonTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        val series = entry.seriesTitle
        if (series != null && (entry.kind == VideoEntryKind.EPISODE || entry.kind == VideoEntryKind.SEASON)) {
            Text(series, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
        }
        Text(
            entry.title,
            Modifier.semantics { heading() },
            style = WontonTheme.typography.screenTitle,
            color = colors.textPrimary.toColor(),
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** 片名下面一行：集号、年份、分级、评分、片长或季集数、类型；再一行清晰度和音频的小标。 */
@Composable
fun VideoFacts(detail: VideoDetail) {
    val colors = WontonTheme.colors
    val entry = detail.entry
    val parts = buildList {
        VideoFormat.episodeLabel(entry)?.let(::add)
        entry.year?.let { add(it.toString()) }
        entry.officialRating?.let(::add)
        entry.communityRating?.let { add("★ " + String.format(Locale.getDefault(), "%.1f", it)) }
        entry.durationMs?.takeIf { entry.kind != VideoEntryKind.SERIES }?.let { add(VideoFormat.duration(it)) }
        val count = entry.childCount?.toInt()
        val counted = when (entry.kind) {
            VideoEntryKind.SERIES -> R.plurals.video_seasons
            VideoEntryKind.SEASON -> R.plurals.video_episodes
            else -> null
        }
        if (count != null && counted != null) add(pluralStringResource(counted, count, count))
        if (detail.genres.isNotEmpty()) add(detail.genres.take(MAX_GENRES).joinToString("/"))
    }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Text(
            parts.joinToString(" · "),
            style = WontonTheme.typography.supporting,
            color = colors.textSecondary.toColor(),
        )
        val badges = detail.media?.badges().orEmpty()
        if (badges.isNotEmpty()) {
            Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                badges.forEach { badge ->
                    Text(
                        badge,
                        Modifier
                            .border(
                                WontonTheme.size.hairline,
                                colors.separator.toColor(),
                                RoundedCornerShape(BADGE_RADIUS),
                            )
                            .padding(horizontal = WontonTheme.spacing.s4),
                        style = WontonTheme.typography.caption.copy(fontWeight = FontWeight.SemiBold),
                        color = colors.textSecondary.toColor(),
                    )
                }
            }
        }
    }
}

/** 4K、HDR、编码、声道：从源的规格里来，没有就不写。 */
private fun VideoMediaInfo.badges(): List<String> = buildList {
    width?.let {
        add(
            when {
                it >= UHD_WIDTH -> "4K"
                it >= FHD_WIDTH -> "1080p"
                it >= HD_WIDTH -> "720p"
                else -> "SD"
            },
        )
    }
    if (hdr) add("HDR")
    videoCodec?.let { add(it.uppercase(Locale.ROOT)) }
    audioChannels?.toInt()?.let { add(if (it > STEREO) "${it - 1}.1" else "$it.0") }
}

/**
 * 离线副本这一行：在下的是进度条和一句“下载时保持 App 打开”；等着接着下的是停着的进度条和上次没下成的原因；
 * 暂停了的是停着的进度条和“继续下载”（核心不会自己叫醒它）；失败的是原因和重试。写完了和没下过的不占这一行。
 */
@Composable
fun VideoOfflineRow(state: OfflineState?, retry: () -> Unit, resume: () -> Unit) {
    val colors = WontonTheme.colors
    when (state) {
        is OfflineState.Downloading -> OfflineProgress(state.done, state.total, R.string.video_downloading) {
            Text(stringResource(R.string.video_keep_app_open))
        }
        is OfflineState.Waiting -> OfflineProgress(state.done, state.total, R.string.video_waiting) {
            state.failure?.let { Text(it.videoProblem.titleText) }
        }
        is OfflineState.Paused -> OfflineProgress(state.done, state.total, R.string.video_paused) {
            WontonSmallButton(stringResource(R.string.video_resume_download), resume)
        }
        is OfflineState.Failed -> Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(
                stringResource(R.string.video_download_failed),
                style = WontonTheme.typography.caption,
                color = colors.textSecondary.toColor(),
            )
            ProblemRow(state.failure.videoProblem, retry)
        }
        is OfflineState.Ready, null -> Unit
    }
}

@Composable
private fun OfflineProgress(done: ULong, total: ULong?, label: Int, note: @Composable () -> Unit) {
    val context = LocalContext.current
    val colors = WontonTheme.colors
    val size = { bytes: ULong -> Formatter.formatShortFileSize(context, bytes.toLong()) }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        if (total != null && total > 0u) {
            LinearProgressIndicator(
                progress = { (done.toDouble() / total.toDouble()).toFloat() },
                Modifier.fillMaxWidth(),
                color = colors.textPrimary.toColor(),
                trackColor = colors.surfaceSubtle.toColor(),
            )
        } else if (label == R.string.video_downloading) {
            LinearProgressIndicator(
                Modifier.fillMaxWidth(),
                color = colors.textPrimary.toColor(),
                trackColor = colors.surfaceSubtle.toColor(),
            )
        }
        val style = WontonTheme.typography.caption
        Row {
            Text(stringResource(label), Modifier.weight(1f), style = style, color = colors.textSecondary.toColor())
            Text(
                if (total != null) "${size(done)} / ${size(total)}" else size(done),
                style = style,
                color = colors.textSecondary.toColor(),
            )
        }
        androidx.compose.material3.ProvideTextStyle(style.copy(color = colors.textSecondary.toColor())) { note() }
    }
}

/** 一季的集：左边剧照（带进度），右边集号、名字和片长。 */
@Composable
fun VideoEpisodeRow(entry: VideoEntry, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = WontonTheme.spacing.pageInset),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(EPISODE_WIDTH, EPISODE_HEIGHT).clip(RoundedCornerShape(EPISODE_RADIUS))) {
            VideoArtwork(entry.landscape ?: entry.backdrop, Modifier.size(EPISODE_WIDTH, EPISODE_HEIGHT))
            entry.progress?.let {
                VideoProgressBar(it, Modifier.align(Alignment.BottomCenter).padding(WontonTheme.spacing.s8))
            }
            if (entry.played) {
                Icon(
                    WontonIcon.CheckCircleFill,
                    contentDescription = null,
                    Modifier.align(Alignment.TopEnd).padding(WontonTheme.spacing.s4).size(EPISODE_BADGE)
                        .background(Color.Black.copy(alpha = SCRIM_ALPHA), CircleShape),
                    tint = Color.White,
                )
            }
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(
                entry.episode?.let { "$it. ${entry.title}" } ?: entry.title,
                style = WontonTheme.typography.supporting.copy(fontWeight = FontWeight.Medium),
                color = colors.textPrimary.toColor(),
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            (VideoFormat.remaining(entry) ?: entry.durationMs?.let(VideoFormat::duration))?.let {
                Text(it, style = WontonTheme.typography.caption, color = colors.textSecondary.toColor())
            }
        }
    }
}

/** 演职员：圆头像一排。 */
@Composable
fun VideoCast(people: List<VideoPerson>) {
    val colors = WontonTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        Text(
            stringResource(R.string.video_cast),
            Modifier.padding(horizontal = WontonTheme.spacing.pageInset).semantics { heading() },
            style = WontonTheme.typography.sectionTitle,
            color = colors.textPrimary.toColor(),
        )
        LazyRow(
            contentPadding = PaddingValues(horizontal = WontonTheme.spacing.pageInset),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16),
        ) {
            itemsIndexed(people) { _, person -> Person(person) }
        }
    }
}

@Composable
private fun Person(person: VideoPerson) {
    val colors = WontonTheme.colors
    Column(
        Modifier.width(PERSON_WIDTH),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        VideoArtwork(person.photo, Modifier.size(PERSON_PHOTO).clip(CircleShape)) {
            Icon(
                WontonIcon.Person,
                // 装饰：名字在下面。
                contentDescription = null,
                Modifier.align(Alignment.Center),
                tint = colors.textSecondary.toColor(),
            )
        }
        Text(
            person.name,
            style = WontonTheme.typography.caption,
            color = colors.textPrimary.toColor(),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            person.character ?: person.role.label(),
            style = WontonTheme.typography.caption,
            color = colors.textSecondary.toColor(),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun VideoRole.label(): String = stringResource(
    when (this) {
        VideoRole.ACTOR -> R.string.video_role_actor
        VideoRole.DIRECTOR -> R.string.video_role_director
        VideoRole.WRITER -> R.string.video_role_writer
        VideoRole.PRODUCER -> R.string.video_role_producer
        VideoRole.OTHER -> R.string.video_role_other
    },
)
