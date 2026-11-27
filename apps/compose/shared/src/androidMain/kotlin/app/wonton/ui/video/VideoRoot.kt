package app.wonton.ui.video

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.client.VideoEntry
import app.wonton.client.VideoEntryKind
import app.wonton.client.VideoHome
import app.wonton.client.VideoLibrary
import app.wonton.client.VideoShelf
import app.wonton.client.VideoShelfKind
import app.wonton.client.VideoSort
import app.wonton.client.VideoWallQuery
import app.wonton.ui.LoadingView
import app.wonton.ui.Problem
import app.wonton.ui.ProblemDialog
import app.wonton.ui.ProblemRow
import app.wonton.ui.ProblemView
import app.wonton.ui.R
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.design.wontonContentPaddingValues
import app.wonton.ui.icons.More
import app.wonton.ui.icons.Search
import app.wonton.ui.icons.WontonIcon

private val LANDSCAPE_CARD = 280.dp
private val POSTER_CARD = 128.dp

/** 影视页里能去的地方：海报墙（一个库、一个类型、一部剧的季、搜索）和详情。 */
sealed interface VideoRoute {
    data class Wall(val query: VideoWallQuery, val title: String) : VideoRoute

    data class Detail(val id: String, val source: String?) : VideoRoute
}

/** 一页内容的三种样子。 */
sealed interface VideoLoad<out T> {
    data object Loading : VideoLoad<Nothing>

    data class Loaded<T>(val value: T) : VideoLoad<T>

    data class Failed(val problem: Problem) : VideoLoad<Nothing>
}

/** 点卡去哪：文件夹进海报墙，其余进详情（剧和季的详情里是季和集）。 */
val VideoEntry.route: VideoRoute
    get() = if (kind == VideoEntryKind.FOLDER) {
        VideoRoute.Wall(wallQuery(parent = id, source = source), title)
    } else {
        VideoRoute.Detail(id, source)
    }

fun wallQuery(
    library: String? = null,
    parent: String? = null,
    search: String? = null,
    source: String? = null,
): VideoWallQuery = VideoWallQuery(
    library = library, parent = parent, kind = null, genre = null, search = search,
    sort = VideoSort.TITLE, unplayedOnly = false, cursor = null, source = source,
)

/** 影视页入口页右上角菜单里的几项：和网页的胶囊菜单一样，网页版也从这里进。 */
class VideoRootActions(
    val title: String,
    val close: () -> Unit,
    val openWeb: () -> Unit,
    val addToMyApps: () -> Unit,
    val details: () -> Unit,
)

/**
 * 点开影视库插件的那一页（`TabPage.native` 是影视，[source] 是这个插件）：首页是库和继续看、下一集、最近添加几排；
 * 点进去是海报墙和详情。自己带一个页面栈，系统返回一层层退，退到首页再关掉插件页。
 */
@Composable
fun VideoRoot(core: ClientCore, source: String, root: VideoRootActions, active: Boolean) {
    val context = LocalContext.current
    val model = remember(core, source) { VideoModel(core, context.applicationContext, source) }
    val stack = remember { mutableStateListOf<VideoRoute>() }
    BackHandler(active) { if (stack.isEmpty()) root.close() else stack.removeAt(stack.lastIndex) }
    val push: (VideoRoute) -> Unit = { stack.add(it) }
    CompositionLocalProvider(LocalVideoCacheScope provides remember(core) { core.videoCacheScope() }) {
        when (val top = stack.lastOrNull()) {
            null -> VideoHomeScreen(model, push, root)
            is VideoRoute.Wall -> VideoWallScreen(model, top.query, top.title, push) { stack.removeAt(stack.lastIndex) }
            is VideoRoute.Detail -> VideoDetailScreen(model, top.id, top.source, push) {
                stack.removeAt(stack.lastIndex)
            }
        }
        model.actionProblem?.let { ProblemDialog(it) { model.actionProblem = null } }
    }
}

/** 首页：顶上一排库，下面继续看、下一集（横版剧照），最近添加（竖版海报）。某一排没取到，顶上一条轻提示。 */
@Composable
private fun VideoHomeScreen(model: VideoModel, push: (VideoRoute) -> Unit, root: VideoRootActions) {
    var load by remember { mutableStateOf<VideoLoad<VideoHome>>(VideoLoad.Loading) }
    var attempt by rememberSaveable { mutableIntStateOf(0) }
    val search = stringResource(R.string.video_search)
    LaunchedEffect(attempt) {
        load = model.home().fold({ VideoLoad.Loaded(it) }, { VideoLoad.Failed(it.videoProblem) })
    }
    WontonScreen(bar = {
        WontonPushedBar(root.title, onBack = root.close) {
            WontonIconButton(WontonIcon.Search, search, {
                push(VideoRoute.Wall(wallQuery(search = "", source = model.source), search))
            })
            VideoRootMenu(root)
        }
    }) {
        when (val current = load) {
            VideoLoad.Loading -> LoadingView()
            is VideoLoad.Failed -> ProblemView(current.problem) {
                load = VideoLoad.Loading
                attempt += 1
            }
            is VideoLoad.Loaded -> HomeContent(current.value, model.source, push) { attempt += 1 }
        }
    }
}

@Composable
private fun HomeContent(home: VideoHome, source: String?, push: (VideoRoute) -> Unit, retry: () -> Unit) {
    val colors = WontonTheme.colors
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = wontonContentPaddingValues(top = WontonTheme.spacing.s16, bottom = WontonTheme.spacing.s16),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        home.partialFailure?.let { failure ->
            item {
                Row(Modifier.padding(horizontal = WontonTheme.spacing.pageInset)) {
                    ProblemRow(failure.videoProblem, retry)
                }
            }
        }
        if (home.libraries.isNotEmpty()) item { Libraries(home.libraries, source, push) }
        items(home.shelves, key = { it.kind.name }) { Shelf(it, push) }
        if (home.shelves.isEmpty() && home.libraries.isEmpty()) {
            item {
                Text(
                    stringResource(R.string.video_empty),
                    Modifier.fillMaxWidth().padding(WontonTheme.spacing.s32),
                    style = WontonTheme.typography.supporting,
                    color = colors.textSecondary.toColor(),
                )
            }
        }
    }
}

@Composable
private fun Libraries(libraries: List<VideoLibrary>, source: String?, push: (VideoRoute) -> Unit) {
    val colors = WontonTheme.colors
    LazyRow(
        contentPadding = PaddingValues(horizontal = WontonTheme.spacing.pageInset),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        items(libraries, key = { it.id }) { library ->
            val wall = VideoRoute.Wall(wallQuery(library = library.id, source = source), library.title)
            Text(
                library.title,
                Modifier.background(colors.surfaceSubtle.toColor(), CircleShape)
                    .clickable { push(wall) }
                    .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s8),
                style = WontonTheme.typography.supporting.copy(fontWeight = FontWeight.Medium),
                color = colors.textPrimary.toColor(),
            )
        }
    }
}

@Composable
private fun Shelf(shelf: VideoShelf, push: (VideoRoute) -> Unit) {
    val title = when (shelf.kind) {
        VideoShelfKind.RESUME -> stringResource(R.string.video_shelf_resume)
        VideoShelfKind.NEXT_UP -> stringResource(R.string.video_shelf_next_up)
        VideoShelfKind.LATEST -> stringResource(R.string.video_shelf_latest)
    }
    VideoShelfRow(title, shelf.entries) { entry ->
        if (shelf.kind == VideoShelfKind.LATEST) {
            VideoPosterCard(entry, { push(entry.route) }, Modifier.width(POSTER_CARD))
        } else {
            VideoLandscapeCard(entry, LANDSCAPE_CARD) { push(entry.route) }
        }
    }
}

/** 入口页右上角的“更多”：网页版、加到我的应用、详情，和网页的胶囊菜单一样。 */
@Composable
private fun VideoRootMenu(root: VideoRootActions) {
    var open by remember { mutableStateOf(false) }
    Box {
        WontonIconButton(WontonIcon.More, stringResource(R.string.tab_page_menu), { open = true })
        DropdownMenu(open, { open = false }, containerColor = WontonTheme.colors.surface.toColor()) {
            RootMenuItem(R.string.video_web_version) { open = false; root.openWeb() }
            RootMenuItem(R.string.myapp_add) { open = false; root.addToMyApps() }
            RootMenuItem(R.string.tab_menu_details) { open = false; root.details() }
        }
    }
}

@Composable
private fun RootMenuItem(label: Int, onClick: () -> Unit) {
    DropdownMenuItem(text = { Text(stringResource(label)) }, onClick = onClick)
}
