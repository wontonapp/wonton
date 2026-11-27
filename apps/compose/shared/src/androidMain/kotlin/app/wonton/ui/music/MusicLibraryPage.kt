package app.wonton.ui.music

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.pulltorefresh.PullToRefreshDefaults
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import app.wonton.protocol.Action
import app.wonton.protocol.MusicPage
import app.wonton.protocol.MusicPageAction
import app.wonton.protocol.MusicPageState
import app.wonton.protocol.MyAppRef
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef
import app.wonton.ui.PagesState
import app.wonton.ui.ProblemDialog
import app.wonton.ui.ProblemView
import app.wonton.ui.R
import app.wonton.ui.design.LocalWontonTopInset
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSearchField
import app.wonton.ui.design.WontonTheme
import kotlinx.coroutines.delay

/** 下拉刷新的圈最多转这么久：核心取回来的内容和原来一样时视图不变，没有别的信号说“刷完了”。 */
private const val REFRESH_HOLD_MS = 1_500L

/** 换页时旧页往左让出的比例（新页从右边整页推进来）。 */
private const val PARALLAX = 3

/** 音乐库入口页右上角菜单里的几项：和网页的胶囊菜单一样，网页版也从这里进。 */
internal class MusicRootActions(
    val title: String,
    val close: () -> Unit,
    val openWeb: () -> Unit,
    val addToMyApps: () -> Unit,
    val details: () -> Unit,
)

/**
 * 点开音乐库插件的那一页（`TabPage.native` 是音乐）：原生的首页，往里点是壳里自己的一叠页，系统返回先退这一叠，
 * 退到首页再关掉插件页。右上角菜单里有网页版、加到我的应用和详情。
 */
@Composable
internal fun MusicLibraryPage(
    tab: TabRef.Plugin, title: String, pages: PagesState, active: Boolean, onDetails: (String) -> Unit,
) {
    val scope = rememberCoroutineScope()
    val navigator = remember(tab.pluginId) { MusicNavigator(pages.core, tab.pluginId, scope) }
    val saved = rememberSaveableStateHolder()
    DisposableEffect(navigator) {
        navigator.forget = saved::removeState
        onDispose { navigator.close() }
    }
    val close = { pages.send(Action.Pages(PageAction.ClosePage)) }
    BackHandler(active) { if (navigator.canGoBack) navigator.back() else close() }
    val root = MusicRootActions(
        title, close,
        openWeb = { pages.send(Action.Pages(PageAction.OpenPage(TabRef.PluginPath(tab.pluginId, "/")))) },
        addToMyApps = { pages.addMyApp(MyAppRef.Page(tab)) },
        details = { onDetails(tab.pluginId) },
    )
    CompositionLocalProvider(LocalMusic provides navigator) {
        Box(Modifier.zIndex(if (active) 1f else 0f).fillMaxSize()) {
            AnimatedContent(
                navigator.top,
                transitionSpec = {
                    when {
                        targetState.depth == initialState.depth -> fadeIn() togetherWith fadeOut()
                        targetState.depth > initialState.depth ->
                            slideInHorizontally { it } togetherWith slideOutHorizontally { -it / PARALLAX }
                        else -> slideInHorizontally { -it / PARALLAX } togetherWith slideOutHorizontally { it }
                    }
                },
                label = "music",
            ) { entry ->
                saved.SaveableStateProvider(entry.key) { MusicScreen(entry, root.takeIf { entry.depth == 0 }) }
            }
        }
        navigator.actionProblem?.let { ProblemDialog(it) { navigator.actionProblem = null } }
    }
}

@Composable
private fun MusicScreen(entry: MusicEntry, root: MusicRootActions?) {
    WontonScreen(bar = { MusicBar(entry, root) }) {
        if (entry.page is MusicPage.Search) MusicSearchBody(entry.model) else MusicStateBody(entry.model)
    }
}

/** 加载、整页出错和部分失败都在这里，内容交给各页。 */
@Composable
private fun MusicStateBody(model: MusicPageModel) {
    when (val state = model.view.state) {
        MusicPageState.Loading -> MusicLoading()
        is MusicPageState.Failed -> Box(Modifier.padding(top = LocalWontonTopInset.current)) {
            ProblemView(state.problem.problem) { model.act(MusicPageAction.Reload) }
        }
        is MusicPageState.Ready -> MusicRefresh(model) {
            MusicContentView(
                state.content,
                MusicPaging(state.hasMore, state.loadingMore) { model.act(MusicPageAction.LoadMore) },
                state.problem?.problem,
            ) { model.act(MusicPageAction.Reload) }
        }
    }
}

/** 下拉刷新：已有内容留着，取到新的再换。转圈让开顶栏。 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MusicRefresh(model: MusicPageModel, content: @Composable () -> Unit) {
    var refreshing by remember { mutableStateOf(false) }
    LaunchedEffect(model.view) { refreshing = false }
    LaunchedEffect(refreshing) {
        if (refreshing) {
            delay(REFRESH_HOLD_MS)
            refreshing = false
        }
    }
    val state = rememberPullToRefreshState()
    val top = LocalWontonTopInset.current
    PullToRefreshBox(
        refreshing,
        onRefresh = {
            refreshing = true
            model.act(MusicPageAction.Reload)
        },
        Modifier.fillMaxSize(),
        state = state,
        indicator = {
            PullToRefreshDefaults.Indicator(state, refreshing, Modifier.align(Alignment.TopCenter).padding(top = top))
        },
    ) { content() }
}

/** 搜索页：搜索框一直在最上面，下面是这批结果。 */
@Composable
private fun MusicSearchBody(model: MusicPageModel) {
    var query by rememberSaveable { mutableStateOf("") }
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    val top = LocalWontonTopInset.current
    Column(Modifier.fillMaxSize().padding(top = top)) {
        WontonSearchField(
            query,
            {
                query = it
                model.act(MusicPageAction.Query(it))
            },
            stringResource(R.string.music_search_prompt),
            Modifier.padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s8)
                .focusRequester(focus),
        )
        CompositionLocalProvider(LocalWontonTopInset provides 0.dp) {
            MusicStateBody(model)
        }
    }
}

/** 顶栏：入口页的返回关掉插件页，里面的页退一层；入口页有搜索和菜单，专辑页有排序。 */
@Composable
private fun MusicBar(entry: MusicEntry, root: MusicRootActions?) {
    val music = LocalMusic.current
    val page = entry.page
    WontonPushedBar(
        musicTitle(page, root),
        onBack = root?.close ?: music::back,
        actions = {
            if (root != null) MusicRootMenu(root)
            if (page is MusicPage.Albums) MusicSortMenu(page.sort)
        },
    )
}

@Composable
private fun musicTitle(page: MusicPage, root: MusicRootActions?): String = when (page) {
    MusicPage.Home -> root?.title.orEmpty()
    is MusicPage.Albums -> stringResource(R.string.music_albums)
    MusicPage.Artists -> stringResource(R.string.music_artists)
    MusicPage.Playlists -> stringResource(R.string.music_playlists)
    MusicPage.Songs -> stringResource(R.string.music_songs)
    MusicPage.Starred -> stringResource(R.string.music_starred)
    MusicPage.Genres -> stringResource(R.string.music_genres)
    is MusicPage.Genre -> page.name
    MusicPage.Search -> stringResource(R.string.music_search)
    // 页头里就是名字，顶栏不重复。
    is MusicPage.Album, is MusicPage.Artist, is MusicPage.Playlist -> ""
}
