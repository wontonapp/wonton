@file:Suppress("TooManyFunctions") // 一种内容一个小函数，按页面种类分派。
package app.wonton.ui.music

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyGridScope
import androidx.compose.foundation.lazy.grid.LazyGridState
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import app.wonton.protocol.AlbumSort
import app.wonton.protocol.AlbumTile
import app.wonton.protocol.ArtistRow
import app.wonton.protocol.MusicContent
import app.wonton.protocol.MusicHome
import app.wonton.protocol.MusicPage
import app.wonton.protocol.MusicSearch
import app.wonton.protocol.MusicStarred
import app.wonton.protocol.MusicTarget
import app.wonton.protocol.PlaylistRow
import app.wonton.protocol.TrackRow
import app.wonton.ui.Problem
import app.wonton.ui.ProblemRow
import app.wonton.ui.R
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.design.wontonContentPaddingValues
import app.wonton.ui.icons.Folder
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.ImageFill
import app.wonton.ui.icons.List
import app.wonton.ui.icons.Music
import app.wonton.ui.icons.Person
import app.wonton.ui.icons.Star
import app.wonton.ui.icons.WontonIcon

/** 网格里一张专辑的最小宽度；手机上两列，宽屏多列。 */
internal const val GRID_TILE = 150
private const val SHELF_TILE = 150
private const val SHELF_ARTIST = 110

/** 离最后一项还有几项时就要下一页，滚到底前就取回来。 */
private const val PAGING_AHEAD = 6

/** 按内容种类挑页面；部分失败（翻页、刷新没成）在最上面一条轻提示，内容留着。 */
@Composable
internal fun MusicContentView(content: MusicContent, paging: MusicPaging, notice: Problem?, reload: () -> Unit) {
    when (content) {
        is MusicContent.Home -> MusicHomeView(content.home, notice, reload)
        is MusicContent.Albums -> MusicAlbumGrid(content.albums, paging, notice, reload)
        is MusicContent.Artists -> MusicList(notice, reload) {
            items(content.artists, key = { it.id }) { MusicArtistRow(it) }
        }
        is MusicContent.Playlists -> MusicList(notice, reload) {
            items(content.playlists, key = { it.id }) { MusicPlaylistRow(it) }
        }
        is MusicContent.Songs -> MusicSongs(content.songs, paging, notice, reload)
        is MusicContent.Starred -> MusicStarredView(content.starred, notice, reload)
        is MusicContent.Genres -> MusicList(notice, reload) {
            items(content.genres, key = { it.name }) { MusicGenreRow(it) }
        }
        is MusicContent.Album -> MusicAlbumView(content.album, notice, reload)
        is MusicContent.Artist -> MusicArtistView(content.artist, notice, reload)
        is MusicContent.Playlist -> MusicPlaylistView(content.playlist, notice, reload)
        is MusicContent.Search -> MusicSearchResults(content.query, content.results, notice)
    }
}

/** 一列行的页：行在页边距里排。 */
@Composable
internal fun MusicList(
    notice: Problem?, reload: (() -> Unit)?, state: LazyListState = rememberLazyListState(),
    rows: LazyListScope.() -> Unit,
) {
    LazyColumn(
        Modifier.fillMaxSize(), state,
        contentPadding = wontonContentPaddingValues(
            horizontal = WontonTheme.spacing.pageInset, top = WontonTheme.spacing.s8, bottom = WontonTheme.spacing.s24,
        ),
    ) {
        notice?.let { item(key = "notice") { ProblemRow(it, reload) } }
        rows()
    }
}

@Composable
private fun MusicSongs(songs: List<TrackRow>, paging: MusicPaging, notice: Problem?, reload: () -> Unit) {
    val state = rememberLazyListState()
    LoadMoreNearEnd(paging, songs.size) { state.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 }
    MusicList(notice, reload, state) {
        items(songs, key = { it.id }) { MusicTrackRow(it) }
        if (paging.loadingMore) item(key = "more") { MusicLoadingRow() }
    }
}

/** 滚到离最后一项不远时要下一页。 */
@Composable
internal fun LoadMoreNearEnd(paging: MusicPaging, total: Int, lastVisible: () -> Int) {
    LaunchedEffect(paging.hasMore, paging.loadingMore, total) {
        snapshotFlow(lastVisible).collect { if (it >= total - PAGING_AHEAD) paging.reached() }
    }
}

@Composable
internal fun MusicLoadingRow() {
    Box(Modifier.fillMaxWidth().padding(WontonTheme.spacing.s16), contentAlignment = Alignment.Center) {
        CircularProgressIndicator(
            Modifier.size(WontonTheme.size.icon), color = WontonTheme.colors.textSecondary.toColor(),
        )
    }
}

/** 专辑封面墙：手机上两列，宽屏多列；滚到底要下一页。 */
@Composable
private fun MusicAlbumGrid(albums: List<AlbumTile>, paging: MusicPaging, notice: Problem?, reload: () -> Unit) {
    val state = rememberLazyGridState()
    LoadMoreNearEnd(paging, albums.size) { state.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 }
    MusicGrid(state = state) {
        notice?.let { fullLine("notice") { ProblemRow(it, reload) } }
        items(albums.size, key = { albums[it].id }) { MusicAlbumTile(albums[it]) }
        if (paging.loadingMore) fullLine("more") { MusicLoadingRow() }
    }
}

@Composable
internal fun MusicGrid(
    state: LazyGridState = rememberLazyGridState(),
    cells: LazyGridScope.() -> Unit,
) {
    LazyVerticalGrid(
        GridCells.Adaptive(GRID_TILE.dp), Modifier.fillMaxSize(), state,
        contentPadding = wontonContentPaddingValues(
            horizontal = WontonTheme.spacing.pageInset, top = WontonTheme.spacing.s8, bottom = WontonTheme.spacing.s24,
        ),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s24),
        content = cells,
    )
}

/** 网格里占满一行的一格。 */
internal fun LazyGridScope.fullLine(key: String, content: @Composable () -> Unit) {
    item(key = key, span = { GridItemSpan(maxLineSpan) }) { content() }
}

/** 音乐库的首页：上面是几类入口，下面是最近添加、最近播放、常听和歌单几排。空的排不显示。 */
@Composable
private fun MusicHomeView(home: MusicHome, notice: Problem?, reload: () -> Unit) {
    MusicShelves(notice, reload) {
        item(key = "categories") { MusicCategories() }
        albumShelf("added", R.string.music_recently_added, home.recentlyAdded, MusicPage.Albums(AlbumSort.NEWEST))
        albumShelf("played", R.string.music_recently_played, home.recentlyPlayed, MusicPage.Albums(AlbumSort.RECENT))
        albumShelf("most", R.string.music_most_played, home.mostPlayed, MusicPage.Albums(AlbumSort.FREQUENT))
        if (home.playlists.isNotEmpty()) item(key = "playlists") { MusicPlaylistShelf(home.playlists) }
    }
}

/** 一段段竖着排的页（首页、收藏、搜索）：段之间留段距，每段自己管左右边距（横排要能滑到屏幕边）。 */
@Composable
private fun MusicShelves(notice: Problem?, reload: (() -> Unit)?, sections: LazyListScope.() -> Unit) {
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = wontonContentPaddingValues(top = WontonTheme.spacing.s8, bottom = WontonTheme.spacing.s24),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        notice?.let {
            item(key = "notice") {
                Box(Modifier.padding(horizontal = WontonTheme.spacing.pageInset)) { ProblemRow(it, reload) }
            }
        }
        sections()
    }
}

private fun LazyListScope.albumShelf(key: String, title: Int, albums: List<AlbumTile>, all: MusicPage? = null) {
    if (albums.isNotEmpty()) item(key = key) { MusicAlbumShelf(stringResource(title), albums, all) }
}

/** 几类入口：一行一类。 */
@Composable
private fun MusicCategories() {
    val entries: List<Triple<Int, ImageVector, MusicPage>> = listOf(
        Triple(R.string.music_playlists, WontonIcon.List, MusicPage.Playlists),
        Triple(R.string.music_artists, WontonIcon.Person, MusicPage.Artists),
        Triple(R.string.music_albums, WontonIcon.ImageFill, MusicPage.Albums(AlbumSort.NAME)),
        Triple(R.string.music_songs, WontonIcon.Music, MusicPage.Songs),
        Triple(R.string.music_starred, WontonIcon.Star, MusicPage.Starred),
        Triple(R.string.music_genres, WontonIcon.Folder, MusicPage.Genres),
    )
    Column(Modifier.padding(horizontal = WontonTheme.spacing.pageInset)) {
        entries.forEachIndexed { index, (title, icon, page) ->
            MusicCategoryRow(stringResource(title), icon, page)
            if (index < entries.lastIndex) {
                HorizontalDivider(
                    Modifier.padding(start = WontonTheme.size.icon + WontonTheme.spacing.s16),
                    thickness = WontonTheme.size.hairline, color = WontonTheme.colors.separator.toColor(),
                )
            }
        }
    }
}

@Composable
private fun MusicCategoryRow(title: String, icon: ImageVector, page: MusicPage) {
    val music = LocalMusic.current
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .clickable(role = Role.Button) { music.open(page) },
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // 装饰：文字就在旁边。
        Icon(icon, contentDescription = null, Modifier.size(WontonTheme.size.icon), tint = colors.textPrimary.toColor())
        Text(title, Modifier.weight(1f), style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
        Icon(
            WontonIcon.Forward, contentDescription = null, Modifier.size(WontonTheme.size.iconSmall),
            tint = colors.textSecondary.toColor(),
        )
    }
}

/** 一排横着滑的专辑。 */
@Composable
private fun MusicAlbumShelf(title: String, albums: List<AlbumTile>, all: MusicPage? = null) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        MusicSectionHeader(title, all)
        MusicShelfRow { items(albums, key = { it.id }) { MusicAlbumTile(it, Modifier.width(SHELF_TILE.dp)) } }
    }
}

@Composable
private fun MusicShelfRow(cells: LazyListScope.() -> Unit) {
    LazyRow(
        contentPadding = PaddingValues(horizontal = WontonTheme.spacing.pageInset),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16),
        content = cells,
    )
}

/** 一排横着滑的歌单。 */
@Composable
private fun MusicPlaylistShelf(playlists: List<PlaylistRow>) {
    val music = LocalMusic.current
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        MusicSectionHeader(stringResource(R.string.music_playlists), MusicPage.Playlists)
        MusicShelfRow {
            items(playlists, key = { it.id }) { playlist ->
                Column(
                    Modifier.width(SHELF_TILE.dp).clickable(role = Role.Button) {
                        music.open(MusicPage.Playlist(playlist.id))
                    },
                    verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
                ) {
                    MusicCover(playlist.cover, Modifier.fillMaxWidth())
                    MusicLine(playlist.name, secondary = false)
                    MusicLine(MusicFormat.playlistLine(playlist), secondary = true)
                }
            }
        }
    }
}

/** 一排横着滑的歌手头像。 */
@Composable
private fun MusicArtistShelf(artists: List<ArtistRow>) {
    val music = LocalMusic.current
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        MusicSectionHeader(stringResource(R.string.music_artists))
        MusicShelfRow {
            items(artists, key = { it.id }) { artist ->
                MusicStarMenuBox(
                    artist.starred, MusicTarget.Artist(artist.id),
                    { music.open(MusicPage.Artist(artist.id)) }, Modifier.width(SHELF_ARTIST.dp),
                ) {
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
                    ) {
                        MusicCover(artist.cover, Modifier.fillMaxWidth(), round = true)
                        MusicLine(artist.name, secondary = false)
                    }
                }
            }
        }
    }
}

/** 页里的一段歌：标题加一列行。 */
private fun LazyListScope.trackStack(key: String, title: Int, tracks: List<TrackRow>) {
    if (tracks.isEmpty()) return
    item(key = "$key-title") { MusicSectionHeader(stringResource(title)) }
    item(key = key) {
        Column(Modifier.padding(horizontal = WontonTheme.spacing.pageInset)) {
            tracks.forEach { MusicTrackRow(it) }
        }
    }
}

/** 收藏：专辑、歌手、歌曲三段，空的段不显示。 */
@Composable
private fun MusicStarredView(starred: MusicStarred, notice: Problem?, reload: () -> Unit) {
    MusicShelves(notice, reload) {
        albumShelf("albums", R.string.music_albums, starred.albums)
        if (starred.artists.isNotEmpty()) item(key = "artists") { MusicArtistShelf(starred.artists) }
        trackStack("songs", R.string.music_songs, starred.songs)
    }
}

/** 搜索结果：歌手、专辑、歌曲；有词没结果时说一句。 */
@Composable
private fun MusicSearchResults(query: String, results: MusicSearch, notice: Problem?) {
    val empty = results.artists.isEmpty() && results.albums.isEmpty() && results.songs.isEmpty()
    MusicShelves(notice, reload = null) {
        if (query.isNotEmpty() && empty) {
            item(key = "empty") {
                Text(
                    stringResource(R.string.music_no_results, query),
                    Modifier.fillMaxWidth().padding(top = WontonTheme.spacing.s48),
                    style = WontonTheme.typography.body.copy(textAlign = TextAlign.Center),
                    color = WontonTheme.colors.textSecondary.toColor(),
                )
            }
        }
        if (results.artists.isNotEmpty()) item(key = "artists") { MusicArtistShelf(results.artists) }
        albumShelf("albums", R.string.music_albums, results.albums)
        trackStack("songs", R.string.music_songs, results.songs)
    }
}

/** 整页转圈：第一次取。 */
@Composable
internal fun MusicLoading() {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        CircularProgressIndicator(color = WontonTheme.colors.textSecondary.toColor())
    }
}
