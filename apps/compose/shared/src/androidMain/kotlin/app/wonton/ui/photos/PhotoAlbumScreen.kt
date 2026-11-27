package app.wonton.ui.photos

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.PhotoAlbumOwner
import app.wonton.protocol.PhotoAlbumView
import app.wonton.protocol.PhotoEditKind
import app.wonton.protocol.PhotosAction
import app.wonton.ui.ListSkeleton
import app.wonton.ui.R
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.icons.Share
import app.wonton.ui.icons.WontonIcon

/** 打开着的相册：顶栏是相册名，自己的相册能共享给家人；下面是它的照片网格。 */
@Composable
internal fun PhotoAlbumScreen(state: PhotosState, open: PhotoAlbumView) {
    val album = open.album
    val page = open.page
    val selecting = state.view.selected.isNotEmpty()
    var sharing by remember { mutableStateOf(false) }
    val reload = { state.send(PhotosAction.OpenAlbum(open.id)) }
    Column(Modifier.fillMaxSize()) {
        if (selecting) {
            PhotoSelectionBar(state)
        } else {
            WontonPushedBar(album?.title.orEmpty(), onBack = { state.send(PhotosAction.CloseAlbum) }) {
                if (album != null && album.owner == PhotoAlbumOwner.Me && state.supports(PhotoEditKind.SHARE_ALBUM)) {
                    WontonIconButton(WontonIcon.Share, stringResource(R.string.photo_library_share), { sharing = true })
                }
            }
        }
        Box(Modifier.weight(1f)) {
            val failure = page.problem
            when {
                page.tiles.isEmpty() && failure != null -> PhotoProblemPage(failure.problem, reload)
                page.tiles.isEmpty() && page.loading -> ListSkeleton(WontonTheme.size.storeIcon)
                page.tiles.isEmpty() -> PhotoEmptyPage(stringResource(R.string.photo_library_album_empty), null)
                else -> PhotoGrid(
                    remember(page) { page.entries() },
                    state,
                    if (selecting) WindowInsets(0).asPaddingValues() else WindowInsets.navigationBars.asPaddingValues(),
                )
            }
        }
        if (selecting) PhotoSelectionActions(state, album)
    }
    if (sharing && album != null) PhotoShareSheet(state, ShareTarget.Album(album)) { sharing = false }
}
