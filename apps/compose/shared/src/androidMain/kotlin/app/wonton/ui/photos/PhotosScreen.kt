package app.wonton.ui.photos

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.zIndex
import app.wonton.protocol.Action
import app.wonton.protocol.PageAction
import app.wonton.protocol.PhotoSourceState
import app.wonton.protocol.PhotoTile
import app.wonton.protocol.TabRef
import app.wonton.protocol.PhotosAction
import app.wonton.ui.LocalPluginAccount
import app.wonton.ui.PagesState
import app.wonton.ui.ProblemDialog
import app.wonton.ui.R
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Folder
import app.wonton.ui.icons.FolderFill
import app.wonton.ui.icons.Image
import app.wonton.ui.icons.ImageFill
import app.wonton.ui.icons.Search
import app.wonton.ui.icons.WontonIcon

/**
 * 原生相册页（ADR-0113 第 9 节）：点开声明了相册库的插件（`TabPage.native` 是相册）就是它，代替插件的网页。
 * 时间线、相册、搜索三块，上面叠相册页和大图页。状态都在核心的 `PhotosViewModel`，这里订阅它、画出来、把操作发回去。
 * [active] 是不是栈顶的那一页：不是的时候不接系统返回。
 */
@Composable
internal fun PhotosRoute(tab: TabRef.Plugin, pages: PagesState, active: Boolean) {
    val core = pages.core
    val account = LocalPluginAccount.current
    val context = LocalContext.current
    val state = remember(core, account, tab.pluginId) { PhotosState(core, account, tab.pluginId) }
    val loader = remember(core) { photoLoader(context, core) }
    DisposableEffect(loader) { onDispose { loader.memoryCache?.clear() } }
    LaunchedEffect(state) { state.observe() }
    LaunchedEffect(state) { state.open() }
    val close = { pages.send(Action.Pages(PageAction.ClosePage)) }
    CompositionLocalProvider(LocalPhotoLoader provides loader) {
        Box(Modifier.zIndex(if (active) 1f else 0f).fillMaxSize()) { PhotosHost(state, active, close) }
    }
    state.rejection?.let { ProblemDialog(it) { state.rejection = null } }
}

/** 相册页底栏的三块。 */
internal enum class PhotosTab(val title: Int, val icon: ImageVector, val selectedIcon: ImageVector) {
    TIMELINE(R.string.photo_library_tab_photos, WontonIcon.Image, WontonIcon.ImageFill),
    ALBUMS(R.string.photo_library_tab_albums, WontonIcon.Folder, WontonIcon.FolderFill),
    SEARCH(R.string.photo_library_tab_search, WontonIcon.Search, WontonIcon.Search),
}

@Composable
private fun PhotosHost(state: PhotosState, active: Boolean, onClose: () -> Unit) {
    val view = state.view
    var tab by rememberSaveable { mutableStateOf(PhotosTab.TIMELINE) }
    val album = view.album
    // 大图页左右滑的范围：从哪个网格点进来的就是哪个网格。
    val tiles: List<PhotoTile> = when {
        album != null -> album.page.tiles
        tab == PhotosTab.SEARCH -> view.search?.page?.tiles.orEmpty()
        else -> remember(view.timeline) { view.timelineTiles() }
    }
    BackHandler(active) {
        when {
            view.selected.isNotEmpty() -> state.send(PhotosAction.ClearSelection)
            album != null -> state.send(PhotosAction.CloseAlbum)
            tab != PhotosTab.TIMELINE -> tab = PhotosTab.TIMELINE
            else -> onClose()
        }
    }
    Box(Modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor())) {
        when {
            // 用哪个相册应用还没定下来（在问、没装、要挑、没问成）：其余几块核心都不取，整页说这件事。
            view.sources.state != PhotoSourceState.READY -> PhotoSourcesPage(state, onClose)
            album != null -> PhotoAlbumScreen(state, album)
            else -> PhotosTabs(state, tab, { tab = it }, onClose)
        }
        view.detail?.let { PhotoViewer(state, it, tiles, active) }
        PhotoEditNotice(state, Modifier.align(Alignment.BottomCenter))
    }
}

@Composable
private fun PhotosTabs(
    state: PhotosState,
    tab: PhotosTab,
    onTab: (PhotosTab) -> Unit,
    onClose: () -> Unit,
) {
    val selecting = state.view.selected.isNotEmpty()
    Column(Modifier.fillMaxSize()) {
        if (selecting) {
            PhotoSelectionBar(state)
        } else {
            WontonPushedBar(stringResource(tab.title), onClose)
        }
        Box(Modifier.weight(1f)) {
            val padding = PaddingValues()
            when (tab) {
                PhotosTab.TIMELINE -> PhotosTimeline(state, padding)
                PhotosTab.ALBUMS -> PhotoAlbums(state, padding)
                PhotosTab.SEARCH -> PhotoSearchPage(state, padding)
            }
        }
        if (selecting) PhotoSelectionActions(state, album = null) else PhotosTabBar(tab, onTab)
    }
}

/** 底栏走 Material 3 的 `NavigationBar`，不透明、贴底，颜色取主题令牌（ADR-0029）。 */
@Composable
private fun PhotosTabBar(tab: PhotosTab, onTab: (PhotosTab) -> Unit) {
    val colors = WontonTheme.colors
    NavigationBar(containerColor = colors.surface.toColor(), contentColor = colors.textPrimary.toColor()) {
        PhotosTab.entries.forEach { item ->
            val selected = item == tab
            NavigationBarItem(
                selected = selected,
                onClick = { onTab(item) },
                // 装饰：底下就是这一项的名字。
                icon = {
                    Icon(
                        if (selected) item.selectedIcon else item.icon,
                        contentDescription = null,
                        Modifier.size(WontonTheme.size.icon),
                    )
                },
                label = { Text(stringResource(item.title), style = WontonTheme.typography.caption) },
                colors = NavigationBarItemDefaults.colors(
                    indicatorColor = colors.primary.toColor(),
                    selectedIconColor = colors.onPrimary.toColor(),
                    selectedTextColor = colors.textPrimary.toColor(),
                    unselectedIconColor = colors.textSecondary.toColor(),
                    unselectedTextColor = colors.textSecondary.toColor(),
                ),
            )
        }
    }
}
