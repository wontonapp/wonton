package app.wonton.ui.photos

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextAlign
import app.wonton.protocol.PhotoAlbumOwner
import app.wonton.protocol.PhotoAlbumRow
import app.wonton.protocol.PhotoEdit
import app.wonton.protocol.PhotoEditKind
import app.wonton.protocol.PhotosAction
import app.wonton.protocol.PhotosViewModel
import app.wonton.ui.R
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Add
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Leave
import app.wonton.ui.icons.Share
import app.wonton.ui.icons.Star
import app.wonton.ui.icons.StarFill
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.WontonIcon

/** 选择中的顶栏：左边退出选择，标题是选了几项。 */
@Composable
internal fun PhotoSelectionBar(state: PhotosState) {
    val count = state.view.selected.size
    WontonPushedBar(
        pluralStringResource(R.plurals.photo_library_selected, count, count),
        onBack = { state.send(PhotosAction.ClearSelection) },
        icon = WontonIcon.Close,
        iconLabel = stringResource(R.string.photo_library_selection_cancel),
    )
}

private enum class SelectionSheet { ALBUM, SHARE }

/**
 * 选择中的底栏：只出这个相册应用接的写操作（`PhotosViewModel.actions`）。[album] 是正开着的相册，在它里面才有“移出相册”。
 * 写操作发出去还没回时顶上走一条进度。
 */
@Composable
internal fun PhotoSelectionActions(state: PhotosState, album: PhotoAlbumRow?) {
    val view = state.view
    val ids = view.selected
    var sheet by remember { mutableStateOf<SelectionSheet?>(null) }
    val colors = WontonTheme.colors
    Column(Modifier.fillMaxWidth().background(colors.surface.toColor()).navigationBarsPadding()) {
        if (view.editing > 0u) {
            LinearProgressIndicator(Modifier.fillMaxWidth(), color = colors.primary.toColor())
        }
        Row(Modifier.fillMaxWidth().padding(vertical = WontonTheme.spacing.s4)) {
            val allFavorite = view.allFavorite(ids)
            if (allFavorite && state.supports(PhotoEditKind.UNFAVORITE)) {
                SelectionAction(WontonIcon.StarFill, R.string.photo_library_unfavorite) {
                    state.edit(PhotoEdit.Favorite(ids, false))
                }
            } else if (state.supports(PhotoEditKind.FAVORITE)) {
                SelectionAction(WontonIcon.Star, R.string.photo_library_favorite) {
                    state.edit(PhotoEdit.Favorite(ids, true))
                }
            }
            if (state.supports(PhotoEditKind.ADD_TO_ALBUM) || state.supports(PhotoEditKind.CREATE_ALBUM)) {
                SelectionAction(WontonIcon.Add, R.string.photo_library_add_to_album) { sheet = SelectionSheet.ALBUM }
            }
            if (state.supports(PhotoEditKind.CREATE_ALBUM) && state.supports(PhotoEditKind.SHARE_ALBUM)) {
                SelectionAction(WontonIcon.Share, R.string.photo_library_share) { sheet = SelectionSheet.SHARE }
            }
            if (album != null && album.editable && state.supports(PhotoEditKind.REMOVE_FROM_ALBUM)) {
                SelectionAction(WontonIcon.Leave, R.string.photo_library_remove_from_album) {
                    state.edit(PhotoEdit.RemoveFromAlbum(album.id, ids))
                }
            }
            if (state.supports(PhotoEditKind.TRASH)) {
                SelectionAction(WontonIcon.Trash, R.string.photo_library_trash) { state.edit(PhotoEdit.Trash(ids)) }
            }
        }
    }
    when (sheet) {
        SelectionSheet.ALBUM -> PhotoAlbumPicker(state, ids) { sheet = null }
        SelectionSheet.SHARE -> PhotoShareSheet(state, ShareTarget.NewAlbum(ids)) { sheet = null }
        null -> Unit
    }
}

/** 自己的和家庭的相册能往里加、往外移；别人共享来的不能。 */
internal val PhotoAlbumRow.editable: Boolean
    get() = owner == PhotoAlbumOwner.Me || owner == PhotoAlbumOwner.Household

@Composable
private fun RowScope.SelectionAction(icon: ImageVector, label: Int, onClick: () -> Unit) {
    val tint = WontonTheme.colors.textPrimary.toColor()
    Column(
        Modifier.weight(1f).heightIn(min = WontonTheme.size.bottomBar)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = WontonTheme.spacing.s4, vertical = WontonTheme.spacing.s8),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        // 装饰：底下就是这个操作的名字。
        Icon(icon, contentDescription = null, Modifier.size(WontonTheme.size.icon), tint = tint)
        Text(
            stringResource(label),
            style = WontonTheme.typography.caption.copy(textAlign = TextAlign.Center),
            color = tint,
            maxLines = 2,
        )
    }
}

/** 选中的是不是都收藏了：都收藏了底栏出“取消收藏”，否则出“收藏”。 */
private fun PhotosViewModel.allFavorite(ids: List<String>): Boolean {
    val wanted = ids.toSet()
    val pages = listOfNotNull(album?.page, search?.page).flatMap { it.tiles }
    val known = (timelineTiles() + pages).filter { it.id in wanted }
    return known.isNotEmpty() && known.all { it.favorite }
}
