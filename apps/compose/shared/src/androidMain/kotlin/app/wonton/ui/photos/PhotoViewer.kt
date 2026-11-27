package app.wonton.ui.photos

import androidx.activity.compose.BackHandler
import androidx.activity.compose.LocalActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.core.view.WindowCompat
import app.wonton.protocol.PhotoDetailView
import app.wonton.protocol.PhotoEdit
import app.wonton.protocol.PhotoEditKind
import app.wonton.protocol.PhotoTile
import app.wonton.protocol.PhotosAction
import app.wonton.ui.R
import app.wonton.ui.design.WontonActionSheet
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonSheetAction
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.icons.Add
import app.wonton.ui.icons.Back
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.More
import app.wonton.ui.icons.Star
import app.wonton.ui.icons.StarFill
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.WontonIcon

/** 下滑到这么远，黑底淡到最浅。 */
private val FADE_DRAG = 320.dp
private const val FADE_FLOOR = 0.35f
private const val BAR_SCRIM = 0.55f

private enum class ViewerPanel { INFO, MORE, ALBUM }

/**
 * 大图页：盖在网格上面的一整屏。[tiles] 是点进来的那个网格的格子，左右滑在它们之间翻；翻到哪张就让核心取哪张的
 * 详情。点一下收放顶栏，下滑或返回键关掉。
 */
@Composable
internal fun PhotoViewer(state: PhotosState, detail: PhotoDetailView, tiles: List<PhotoTile>, active: Boolean) {
    val pages = remember(tiles, detail.id, detail.tile) {
        if (tiles.any { it.id == detail.id }) tiles else listOfNotNull(detail.tile)
    }
    val index = pages.indexOfFirst { it.id == detail.id }
    val pager = rememberPagerState(index.coerceAtLeast(0)) { pages.size }
    val shown by rememberUpdatedState(detail.id)
    LaunchedEffect(pager, pages) {
        snapshotFlow { pager.settledPage }.collect { page ->
            pages.getOrNull(page)?.id?.takeIf { it != shown }?.let { state.send(PhotosAction.OpenDetail(it)) }
        }
    }
    LaunchedEffect(index) {
        if (index >= 0 && index != pager.currentPage && !pager.isScrollInProgress) pager.scrollToPage(index)
    }
    var chrome by remember { mutableStateOf(true) }
    var zoomed by remember { mutableStateOf(false) }
    var drag by remember { mutableFloatStateOf(0f) }
    var panel by remember { mutableStateOf<ViewerPanel?>(null) }
    val close = { state.send(PhotosAction.CloseDetail) }
    val events = remember(state) {
        ViewerPageEvents(
            onTap = { chrome = !chrome },
            onClose = close,
            onZoomed = { zoomed = it },
            onDrag = { drag = it },
        )
    }
    BackHandler(active, close)
    DarkStatusBar()
    val fade = with(LocalDensity.current) { (drag / FADE_DRAG.toPx()).coerceIn(0f, 1f - FADE_FLOOR) }
    Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 1f - fade))) {
        HorizontalPager(
            pager,
            Modifier.fillMaxSize(),
            key = { pages.getOrNull(it)?.id ?: it },
            userScrollEnabled = !zoomed,
            beyondViewportPageCount = 1,
        ) { page ->
            pages.getOrNull(page)?.let { PhotoViewerPage(state, it, page == pager.currentPage, events) }
        }
        val tile = detail.tile ?: pages.getOrNull(index)
        if (chrome && drag == 0f) ViewerBar(state, detail.id, tile, close) { panel = it }
    }
    when (panel) {
        ViewerPanel.INFO -> PhotoInfoSheet(state, detail) { panel = null }
        ViewerPanel.MORE -> ViewerMore(state, detail.id, { panel = null }) { panel = ViewerPanel.ALBUM }
        ViewerPanel.ALBUM -> PhotoAlbumPicker(state, listOf(detail.id)) { panel = null }
        null -> Unit
    }
}

/** 大图页是黑底：开着的时候状态栏的图标用浅色，关掉还原。 */
@Composable
private fun DarkStatusBar() {
    val window = LocalActivity.current?.window ?: return
    DisposableEffect(window) {
        val bars = WindowCompat.getInsetsController(window, window.decorView)
        val light = bars.isAppearanceLightStatusBars
        bars.isAppearanceLightStatusBars = false
        onDispose { bars.isAppearanceLightStatusBars = light }
    }
}

/** 顶栏压在图上：返回、拍摄日期，右边是收藏、信息和更多。只出这个相册应用接的操作。 */
@Composable
private fun ViewerBar(
    state: PhotosState,
    id: String,
    tile: PhotoTile?,
    onClose: () -> Unit,
    onPanel: (ViewerPanel) -> Unit,
) {
    val white = Color.White
    Row(
        Modifier.fillMaxWidth()
            .background(Brush.verticalGradient(listOf(Color.Black.copy(alpha = BAR_SCRIM), Color.Transparent)))
            .statusBarsPadding().padding(horizontal = WontonTheme.spacing.s4),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        WontonIconButton(WontonIcon.Back, stringResource(R.string.design_back), onClose, white)
        Text(
            tile?.let { takenDayText(LocalContext.current, it.takenAt) }.orEmpty(),
            Modifier.weight(1f).padding(horizontal = WontonTheme.spacing.s8),
            style = WontonTheme.typography.itemTitle,
            color = white,
            maxLines = 1,
        )
        val favorite = tile?.favorite == true
        val flip = if (favorite) PhotoEditKind.UNFAVORITE else PhotoEditKind.FAVORITE
        if (tile != null && state.supports(flip)) {
            WontonIconButton(
                if (favorite) WontonIcon.StarFill else WontonIcon.Star,
                stringResource(if (favorite) R.string.photo_library_unfavorite else R.string.photo_library_favorite),
                { state.edit(PhotoEdit.Favorite(listOf(id), !favorite)) },
                white,
            )
        }
        val info = stringResource(R.string.photo_library_info)
        WontonIconButton(WontonIcon.Info, info, { onPanel(ViewerPanel.INFO) }, white)
        val more = listOf(PhotoEditKind.ADD_TO_ALBUM, PhotoEditKind.CREATE_ALBUM, PhotoEditKind.TRASH)
        if (more.any { state.supports(it) }) {
            val label = stringResource(R.string.photo_library_more)
            WontonIconButton(WontonIcon.More, label, { onPanel(ViewerPanel.MORE) }, white)
        }
    }
}

@Composable
private fun ViewerMore(state: PhotosState, id: String, onDismiss: () -> Unit, onAlbum: () -> Unit) {
    val actions = buildList {
        if (state.supports(PhotoEditKind.ADD_TO_ALBUM) || state.supports(PhotoEditKind.CREATE_ALBUM)) {
            val add = stringResource(R.string.photo_library_add_to_album)
            add(WontonSheetAction(add, WontonIcon.Add, onClick = onAlbum))
        }
        if (state.supports(PhotoEditKind.TRASH)) {
            add(
                WontonSheetAction(stringResource(R.string.photo_library_trash), WontonIcon.Trash, isDanger = true) {
                    state.edit(PhotoEdit.Trash(listOf(id)))
                },
            )
        }
    }
    WontonActionSheet(actions, onDismiss)
}
