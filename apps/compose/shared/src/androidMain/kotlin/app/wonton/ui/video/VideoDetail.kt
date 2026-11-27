package app.wonton.ui.video

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import app.wonton.client.OfflineState
import app.wonton.client.VideoDetail
import app.wonton.client.VideoEntry
import app.wonton.client.VideoEntryKind
import app.wonton.protocol.MediaItemRef
import app.wonton.ui.LoadingView
import app.wonton.ui.ProblemRow
import app.wonton.ui.ProblemView
import app.wonton.ui.R
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.design.wontonContentPaddingValues
import app.wonton.ui.icons.CheckCircle
import app.wonton.ui.icons.CheckCircleFill
import app.wonton.ui.icons.Download
import app.wonton.ui.icons.DownloadFill
import app.wonton.ui.icons.Refresh
import app.wonton.ui.icons.Star
import app.wonton.ui.icons.StarFill
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch

private const val OVERVIEW_LINES = 4

/**
 * 详情：大背景图和片名，播放（接着看）、从头看、看过、收藏、下载，音轨字幕，简介，季和集，演职员。
 * 放完回来（[VideoModel.playbackEnded] 变了）原地重取，不回到加载中。
 */
@Composable
fun VideoDetailScreen(
    model: VideoModel,
    id: String,
    source: String?,
    push: (VideoRoute) -> Unit,
    onBack: () -> Unit,
) {
    val state = remember(id) { VideoDetailState(model, id, source) }
    LaunchedEffect(state, model.playbackEnded, state.attempt) {
        state.refresh()
        state.loadEpisodes(showLoading = false)
    }
    LaunchedEffect(state.season?.id) { state.loadEpisodes(showLoading = true) }
    LaunchedEffect(state.inProgress) { state.watchDownload() }
    WontonScreen(bar = { WontonPushedBar("", onBack = onBack) }) {
        when (val load = state.load) {
            VideoLoad.Loading -> LoadingView()
            is VideoLoad.Failed -> ProblemView(load.problem) { state.retry() }
            is VideoLoad.Loaded -> DetailContent(state, load.value, push)
        }
    }
}

@Composable
private fun DetailContent(state: VideoDetailState, detail: VideoDetail, push: (VideoRoute) -> Unit) {
    val entry = detail.entry
    LazyColumn(
        Modifier.fillMaxSize(),
        // 大图顶到状态栏下面，栏浮在它上面；底下让开导航条。
        contentPadding = wontonContentPaddingValues(bottom = WontonTheme.spacing.s32).let {
            PaddingValues(bottom = it.calculateBottomPadding())
        },
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16),
    ) {
        item { VideoHero(entry) }
        item { Summary(state, detail) }
        if (entry.kind == VideoEntryKind.SERIES && detail.children.isNotEmpty()) {
            seasons(state, detail.children, push)
        } else if (entry.kind == VideoEntryKind.SEASON) {
            items(detail.children, key = { it.id }) { VideoEpisodeRow(it) { push(it.route) } }
        }
        if (detail.people.isNotEmpty()) item { VideoCast(detail.people) }
    }
}

/** 片名下面的一块：小标、按钮、要放哪一集、离线副本、音轨字幕、简介。 */
@Composable
private fun Summary(state: VideoDetailState, detail: VideoDetail) {
    val colors = WontonTheme.colors
    val scope = rememberCoroutineScope()
    Column(
        Modifier.padding(horizontal = WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16),
    ) {
        VideoFacts(detail)
        Actions(state, detail)
        val target = playTarget(detail)
        if (target != null && target.id != detail.entry.id) {
            Text(
                VideoFormat.episodeLabel(target)?.let { "$it · ${target.title}" } ?: target.title,
                style = WontonTheme.typography.supporting,
                color = colors.textSecondary.toColor(),
            )
        }
        detail.entry.item?.let { item ->
            VideoOfflineRow(
                state.offline,
                retry = { scope.launch { state.retryDownload(item) } },
                resume = { scope.launch { state.resumeDownload(item) } },
            )
        }
        if (detail.audio.size > 1 || detail.subtitles.isNotEmpty()) {
            VideoTrackChips(detail, state.audio, state.subtitle, { state.audio = it }) { state.subtitle = it }
        }
        Overview(detail)
    }
}

/** 剧放“下一集”，季放第一集没看完的，其余放自己。 */
private fun playTarget(detail: VideoDetail): VideoEntry? = when (detail.entry.kind) {
    VideoEntryKind.SERIES -> detail.nextUp
    VideoEntryKind.SEASON -> detail.nextUp ?: detail.children.firstOrNull { !it.played && it.item != null }
    else -> detail.entry.takeIf { it.item != null }
}

@Composable
private fun Actions(state: VideoDetailState, detail: VideoDetail) {
    val scope = rememberCoroutineScope()
    val entry = detail.entry
    val target = playTarget(detail)
    val item = target?.item
    val resume = target?.positionMs?.takeIf { it > 0u }
    Row(
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (target != null && item != null) {
            val play = { from: ULong -> scope.launch { state.play(item, target.title, from) } }
            WontonButton(
                stringResource(if (resume == null) R.string.video_play else R.string.video_resume),
                { play(resume ?: 0u) },
                Modifier.weight(1f),
            )
            if (resume != null) {
                WontonIconButton(WontonIcon.Refresh, stringResource(R.string.video_restart), { play(0u) })
            }
        } else {
            Box(Modifier.weight(1f))
        }
        WontonIconButton(
            if (entry.played) WontonIcon.CheckCircleFill else WontonIcon.CheckCircle,
            stringResource(if (entry.played) R.string.video_mark_unplayed else R.string.video_mark_played),
            { scope.launch { state.change { setPlayed(entry, !entry.played) } } },
        )
        WontonIconButton(
            if (entry.favorite) WontonIcon.StarFill else WontonIcon.Star,
            stringResource(if (entry.favorite) R.string.video_unfavorite else R.string.video_favorite),
            { scope.launch { state.change { setFavorite(entry, !entry.favorite) } } },
        )
        entry.item?.let { DownloadButton(state, it) }
    }
}

/** 没下过是下载；写完了点开问要不要删；在下、等待、失败的进度和重试在下面那一行。 */
@Composable
private fun DownloadButton(state: VideoDetailState, item: MediaItemRef) {
    val scope = rememberCoroutineScope()
    var confirming by remember { mutableStateOf(false) }
    when (state.offline) {
        null -> WontonIconButton(WontonIcon.Download, stringResource(R.string.video_download), {
            scope.launch { state.download(item) }
        })
        is OfflineState.Ready -> WontonIconButton(WontonIcon.DownloadFill, stringResource(R.string.video_downloaded), {
            confirming = true
        })
        else -> Unit
    }
    if (confirming) {
        WontonDialog(
            title = stringResource(R.string.video_remove_download_confirm),
            onDismiss = { confirming = false },
            actions = listOf(
                WontonDialogAction(stringResource(R.string.common_cancel)) { confirming = false },
                WontonDialogAction(stringResource(R.string.video_remove_download), WontonButtonKind.DANGER) {
                    confirming = false
                    state.removeOffline(item)
                },
            ),
        )
    }
}

@Composable
private fun Overview(detail: VideoDetail) {
    val colors = WontonTheme.colors
    var expanded by remember { mutableStateOf(false) }
    detail.tagline?.takeIf { it.isNotEmpty() }?.let {
        Text(
            it,
            style = WontonTheme.typography.body.copy(fontWeight = FontWeight.SemiBold),
            color = colors.textPrimary.toColor(),
        )
    }
    detail.overview?.takeIf { it.isNotEmpty() }?.let {
        Text(
            it,
            Modifier.clickable { expanded = !expanded },
            style = WontonTheme.typography.body,
            color = colors.textSecondary.toColor(),
            maxLines = if (expanded) Int.MAX_VALUE else OVERVIEW_LINES,
        )
    }
}

/** 剧的季：一排季的胶囊，下面是选中那一季的集。 */
private fun LazyListScope.seasons(state: VideoDetailState, seasons: List<VideoEntry>, push: (VideoRoute) -> Unit) {
    item { SeasonChips(seasons, state.season?.id) { state.season = it } }
    when (val episodes = state.episodes) {
        VideoLoad.Loading -> item {
            Box(Modifier.fillMaxWidth().padding(WontonTheme.spacing.s16), contentAlignment = Alignment.Center) {
                CircularProgressIndicator(color = WontonTheme.colors.textPrimary.toColor())
            }
        }
        is VideoLoad.Failed -> item {
            val scope = rememberCoroutineScope()
            Box(Modifier.padding(horizontal = WontonTheme.spacing.pageInset)) {
                ProblemRow(episodes.problem) { scope.launch { state.loadEpisodes(showLoading = true) } }
            }
        }
        is VideoLoad.Loaded -> items(episodes.value, key = { it.id }) { VideoEpisodeRow(it) { push(it.route) } }
    }
}

@Composable
private fun SeasonChips(seasons: List<VideoEntry>, selected: String?, onSelect: (VideoEntry) -> Unit) {
    val colors = WontonTheme.colors
    LazyRow(
        contentPadding = PaddingValues(horizontal = WontonTheme.spacing.pageInset),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        items(seasons, key = { it.id }) { season ->
            val on = season.id == selected
            Text(
                season.title,
                Modifier.background((if (on) colors.textPrimary else colors.surfaceSubtle).toColor(), CircleShape)
                    .clickable { onSelect(season) }
                    .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s8),
                style = WontonTheme.typography.supporting.copy(
                    fontWeight = if (on) FontWeight.SemiBold else FontWeight.Normal,
                ),
                color = (if (on) colors.canvas else colors.textPrimary).toColor(),
            )
        }
    }
}
