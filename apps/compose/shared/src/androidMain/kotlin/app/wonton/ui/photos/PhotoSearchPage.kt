package app.wonton.ui.photos

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyGridScope
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.wonton.protocol.PhotoPage
import app.wonton.protocol.PhotoSearch
import app.wonton.protocol.PhotosAction
import app.wonton.ui.R
import app.wonton.ui.design.WontonSearchField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import kotlinx.coroutines.delay

/** 停手这么久才搜，打字途中不每个字发一次。 */
private const val SEARCH_DEBOUNCE_MS = 400L
private val STATUS_HEIGHT = 240.dp

/**
 * 搜索：顶上写画面描述，下面按人物、时间、地点、种类、收藏筛；条件一变（停手片刻）就搜，条件清空就收起结果。
 * 筛选条件排在结果网格的最前面，跟着一起滚。
 */
@Composable
internal fun PhotoSearchPage(state: PhotosState, padding: PaddingValues) {
    val search = state.view.search
    var draft by remember { mutableStateOf(search?.query ?: PhotoSearch()) }
    var time by remember { mutableStateOf(TimeFilter.ANY) }
    LaunchedEffect(draft) {
        if (draft == state.view.search?.query) return@LaunchedEffect
        delay(SEARCH_DEBOUNCE_MS)
        when {
            !draft.isBlank() -> state.send(PhotosAction.Search(draft))
            state.view.search != null -> state.send(PhotosAction.ClearSearch)
        }
    }
    Column(Modifier.fillMaxSize()) {
        WontonSearchField(
            draft.text.orEmpty(),
            { draft = draft.copy(text = it.ifEmpty { null }) },
            stringResource(R.string.photo_library_search_prompt),
            Modifier.padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s8),
        )
        val page = search?.page
        val entries = remember(page) { page?.entries().orEmpty() }
        PhotoGrid(entries, state, padding, header = {
            item(key = "filters", span = { GridItemSpan(maxLineSpan) }) {
                PhotoSearchFilters(state, draft, time) { query, picked ->
                    draft = query
                    time = picked
                }
            }
            if (page != null) searchStatus(page) { state.send(PhotosAction.Search(draft)) }
        })
    }
}

/** 一个条件都没写（和核心的 `PhotoSearch::is_empty` 同一个判断：空的核心不收）。 */
private fun PhotoSearch.isBlank(): Boolean = text.isNullOrBlank() && people.isEmpty() && place.isNullOrBlank() &&
    from == null && to == null && kind == null && favorite == null

/** 结果还没有格子时的那一块：在搜、没搜成、没搜到。 */
private fun LazyGridScope.searchStatus(page: PhotoPage, onRetry: () -> Unit) {
    if (page.tiles.isNotEmpty()) return
    val failure = page.problem
    item(key = "status", span = { GridItemSpan(maxLineSpan) }) {
        Box(Modifier.fillMaxWidth().padding(top = WontonTheme.spacing.s24), contentAlignment = Alignment.TopCenter) {
            when {
                failure != null -> PhotoProblemPage(failure.problem, onRetry, Modifier.padding(0.dp))
                page.loading -> CircularProgressIndicator(color = WontonTheme.colors.textSecondary.toColor())
                else -> PhotoEmptyPage(
                    stringResource(R.string.photo_library_search_none),
                    stringResource(R.string.photo_library_search_none_hint),
                )
            }
        }
    }
}
