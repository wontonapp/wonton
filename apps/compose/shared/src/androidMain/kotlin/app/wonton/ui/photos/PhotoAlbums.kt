package app.wonton.ui.photos

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyGridScope
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.wonton.protocol.PhotoAlbumOwner
import app.wonton.protocol.PhotoAlbumRow
import app.wonton.protocol.PhotoEdit
import app.wonton.protocol.PhotoEditKind
import app.wonton.protocol.PhotosAction
import app.wonton.ui.ListSkeleton
import app.wonton.ui.ProblemRow
import app.wonton.ui.R
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Add
import app.wonton.ui.icons.Share
import app.wonton.ui.icons.WontonIcon

private val ALBUM_CELL = 160.dp

/**
 * 相册：分“我的”“家庭”“共享给我”三组，空的组不出。能建相册的源在“我的”最前面多一格“新建相册”。
 * 取不成、一个相册都没有各有整页的说明；已经有内容时取不成只留一条轻提示。
 */
@Composable
internal fun PhotoAlbums(state: PhotosState, padding: PaddingValues) {
    val albums = state.view.albums
    val failure = albums.problem
    val canCreate = state.supports(PhotoEditKind.CREATE_ALBUM)
    var naming by remember { mutableStateOf(false) }
    val empty = albums.mine.isEmpty() && albums.household.isEmpty() && albums.sharedWithMe.isEmpty()
    val refresh = { state.send(PhotosAction.Refresh) }
    when {
        empty && failure != null -> PhotoProblemPage(failure.problem, refresh)
        empty && albums.loading -> ListSkeleton(WontonTheme.size.storeIcon)
        empty -> PhotoEmptyPage(stringResource(R.string.photo_library_no_albums), null) {
            if (canCreate) WontonButton(stringResource(R.string.photo_library_new_album), { naming = true })
        }
        else -> LazyVerticalGrid(
            GridCells.Adaptive(ALBUM_CELL),
            Modifier.fillMaxSize(),
            contentPadding = PaddingValues(
                start = WontonTheme.spacing.pageInset,
                end = WontonTheme.spacing.pageInset,
                top = padding.calculateTopPadding(),
                bottom = padding.calculateBottomPadding() + WontonTheme.spacing.s24,
            ),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            failure?.let { stale ->
                item(key = "stale", span = { GridItemSpan(maxLineSpan) }) { ProblemRow(stale.problem, refresh) }
            }
            albumGroup(R.string.photo_library_albums_mine, albums.mine, state, { naming = true }.takeIf { canCreate })
            albumGroup(R.string.photo_library_albums_household, albums.household, state)
            albumGroup(R.string.photo_library_albums_shared, albums.sharedWithMe, state)
        }
    }
    if (naming) {
        NewAlbumDialog({ naming = false }) { title ->
            naming = false
            state.edit(PhotoEdit.CreateAlbum(title, emptyList(), emptyList()))
        }
    }
}

private fun LazyGridScope.albumGroup(
    title: Int,
    albums: List<PhotoAlbumRow>,
    state: PhotosState,
    onCreate: (() -> Unit)? = null,
) {
    if (albums.isEmpty() && onCreate == null) return
    item(key = "group:$title", span = { GridItemSpan(maxLineSpan) }) { GroupTitle(stringResource(title)) }
    onCreate?.let { item(key = "new:$title") { NewAlbumCard(it) } }
    items(albums, key = { "album:${it.id}" }) { album ->
        AlbumCard(state, album) { state.send(PhotosAction.OpenAlbum(album.id)) }
    }
}

@Composable
private fun GroupTitle(title: String) {
    Text(
        title,
        Modifier.fillMaxWidth(),
        style = WontonTheme.typography.sectionTitle,
        color = WontonTheme.colors.textPrimary.toColor(),
    )
}

@Composable
private fun AlbumCard(state: PhotosState, album: PhotoAlbumRow, onOpen: () -> Unit) {
    val colors = WontonTheme.colors
    val count = album.count.toInt()
    val owner = (album.owner as? PhotoAlbumOwner.Person)?.user?.name
    Column(
        Modifier.clickable(role = Role.Button, onClick = onOpen),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        Box {
            AlbumCover(state, album, Modifier.fillMaxWidth().aspectRatio(1f))
            if (album.shared) {
                Box(
                    Modifier.align(Alignment.TopEnd).padding(WontonTheme.spacing.s4)
                        .size(WontonTheme.size.smallButton).background(PhotoScrim, CircleShape),
                    Alignment.Center,
                ) {
                    // 共享的记号；读屏的说明在相册名那一行。
                    Icon(
                        WontonIcon.Share,
                        contentDescription = stringResource(R.string.photo_library_shared_mark),
                        Modifier.size(WontonTheme.size.iconSmall),
                        tint = Color.White,
                    )
                }
            }
        }
        Text(
            album.title,
            style = WontonTheme.typography.body,
            color = colors.textPrimary.toColor(),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            listOfNotNull(pluralStringResource(R.plurals.photo_library_items, count, count), owner).joinToString(" · "),
            style = WontonTheme.typography.supporting,
            color = colors.textSecondary.toColor(),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun NewAlbumCard(onCreate: () -> Unit) {
    val colors = WontonTheme.colors
    Column(
        Modifier.clickable(role = Role.Button, onClick = onCreate),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        val shape = RoundedCornerShape(WontonTheme.radii.control)
        Box(
            Modifier.fillMaxWidth().aspectRatio(1f).background(colors.surfaceSubtle.toColor(), shape),
            contentAlignment = Alignment.Center,
        ) {
            // 装饰：底下就是“新建相册”。
            Icon(
                WontonIcon.Add,
                contentDescription = null,
                Modifier.size(WontonTheme.size.icon),
                tint = WontonTheme.colors.textPrimary.toColor(),
            )
        }
        Text(
            stringResource(R.string.photo_library_new_album),
            style = WontonTheme.typography.body,
            color = colors.textPrimary.toColor(),
        )
    }
}
