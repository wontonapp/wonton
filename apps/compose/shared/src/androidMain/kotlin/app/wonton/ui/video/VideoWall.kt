package app.wonton.ui.video

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.wonton.client.VideoEntry
import app.wonton.client.VideoWallQuery
import app.wonton.ui.LoadingView
import app.wonton.ui.Problem
import app.wonton.ui.ProblemRow
import app.wonton.ui.ProblemView
import app.wonton.ui.R
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.LocalWontonTopInset
import app.wonton.ui.design.toColor
import app.wonton.ui.design.wontonContentPaddingValues
import app.wonton.ui.icons.List
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** 打字停这么久再搜，别每个字都问一次服务器。 */
private const val SEARCH_DEBOUNCE_MS = 300L
private val POSTER_MIN = 104.dp

/** 海报墙已经取到的：条目和下一页的游标。 */
private data class WallPage(val entries: List<VideoEntry>, val cursor: String?)

/**
 * 海报墙：一个库、一个类型、文件夹或搜索结果。右上角是排序、只看没看过和类型；滑到底接着取下一页。
 * 搜索页（`query.search` 不为空）顶上是搜索框，一进来就聚焦。
 */
@Composable
fun VideoWallScreen(
    model: VideoModel,
    initial: VideoWallQuery,
    title: String,
    push: (VideoRoute) -> Unit,
    onBack: () -> Unit,
) {
    val searching = initial.search != null
    var query by remember { mutableStateOf(initial.copy(cursor = null)) }
    var load by remember { mutableStateOf<VideoLoad<WallPage>>(VideoLoad.Loading) }
    var filtering by remember { mutableStateOf(false) }
    var reloads by remember { mutableIntStateOf(0) }
    val paging = remember { WallPaging(model) }
    LaunchedEffect(query, reloads) {
        if (searching && query.search.isNullOrEmpty()) {
            load = VideoLoad.Loading
            return@LaunchedEffect
        }
        load = VideoLoad.Loading
        load = paging.first(query)
    }
    WontonScreen(bar = {
        WontonPushedBar(title, onBack = onBack) {
            if (!searching) {
                WontonIconButton(WontonIcon.List, stringResource(R.string.video_filter), { filtering = true })
            }
        }
    }) {
        // 搜索框在栏下面不跟着滚：它让开栏高，下面的网格就不用再让。
        val top = if (searching) LocalWontonTopInset.current else 0.dp
        Column(Modifier.fillMaxSize().padding(top = top)) {
            if (searching) SearchField { text -> query = query.copy(search = text) }
            CompositionLocalProvider(LocalWontonTopInset provides LocalWontonTopInset.current - top) {
                WallContent(load, searching, paging, query, push, { load = it }) { reloads += 1 }
            }
        }
    }
    if (filtering) VideoFilterSheet(model, query, { query = it }) { filtering = false }
}

@Composable
private fun SearchField(onSearch: (String) -> Unit) {
    var text by remember { mutableStateOf("") }
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    LaunchedEffect(text) {
        delay(SEARCH_DEBOUNCE_MS)
        onSearch(text)
    }
    WontonTextField(
        text,
        { text = it },
        Modifier.fillMaxWidth().padding(horizontal = WontonTheme.spacing.pageInset)
            .padding(bottom = WontonTheme.spacing.s8).focusRequester(focus),
        placeholder = stringResource(R.string.video_search_prompt),
        label = stringResource(R.string.video_search),
    )
}

@Suppress("LongParameterList") // 墙的状态在上一层（查询变了要重取），这里只画和翻页。
@Composable
private fun WallContent(
    load: VideoLoad<WallPage>,
    searching: Boolean,
    paging: WallPaging,
    query: VideoWallQuery,
    push: (VideoRoute) -> Unit,
    update: (VideoLoad<WallPage>) -> Unit,
    reload: () -> Unit,
) {
    val colors = WontonTheme.colors
    when (load) {
        VideoLoad.Loading -> if (!(searching && query.search.isNullOrEmpty())) LoadingView()
        is VideoLoad.Failed -> ProblemView(load.problem, reload)
        is VideoLoad.Loaded -> if (load.value.entries.isEmpty()) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Text(
                    stringResource(if (searching) R.string.video_no_results else R.string.video_empty),
                    style = WontonTheme.typography.supporting,
                    color = colors.textSecondary.toColor(),
                )
            }
        } else {
            WallGrid(load.value, paging, query, push, update)
        }
    }
}

@Composable
private fun WallGrid(
    page: WallPage,
    paging: WallPaging,
    query: VideoWallQuery,
    push: (VideoRoute) -> Unit,
    update: (VideoLoad<WallPage>) -> Unit,
) {
    val scope = rememberCoroutineScope()
    val more: () -> Unit = { scope.launch { paging.more(query, page)?.let { update(VideoLoad.Loaded(it)) } } }
    LazyVerticalGrid(
        GridCells.Adaptive(POSTER_MIN),
        Modifier.fillMaxSize(),
        contentPadding = wontonContentPaddingValues(
            horizontal = WontonTheme.spacing.pageInset,
            top = WontonTheme.spacing.pageInset,
            bottom = WontonTheme.spacing.pageInset,
        ),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16),
    ) {
        items(page.entries, key = { it.id }) { entry ->
            VideoPosterCard(entry, { push(entry.route) })
            if (entry.id == page.entries.last().id) LaunchedEffect(entry.id) { more() }
        }
        item(span = { GridItemSpan(maxLineSpan) }) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center) {
                val problem = paging.problem
                if (paging.loading) {
                    CircularProgressIndicator(color = WontonTheme.colors.textPrimary.toColor())
                } else if (problem != null) {
                    ProblemRow(problem, more)
                }
            }
        }
    }
}

/** 翻页：只有一页在取；取回来时查询已经换了（排序、筛选、搜索词）就丢掉那一页。 */
private class WallPaging(private val model: VideoModel) {
    var loading by mutableStateOf(false)
        private set
    var problem by mutableStateOf<Problem?>(null)
        private set
    private var current: VideoWallQuery? = null

    suspend fun first(query: VideoWallQuery): VideoLoad<WallPage> {
        current = query
        problem = null
        return model.wall(query).fold(
            { VideoLoad.Loaded(WallPage(it.entries, it.cursor)) },
            { VideoLoad.Failed(it.videoProblem) },
        )
    }

    suspend fun more(query: VideoWallQuery, page: WallPage): WallPage? {
        val cursor = page.cursor
        if (cursor == null || loading) return null
        loading = true
        problem = null
        val result = model.wall(query.copy(cursor = cursor))
        loading = false
        return if (current != query) null else result.fold(
            { wall ->
                val seen = page.entries.mapTo(HashSet()) { it.id }
                WallPage(page.entries + wall.entries.filterNot { it.id in seen }, wall.cursor)
            },
            {
                problem = it.videoProblem
                null
            },
        )
    }
}
