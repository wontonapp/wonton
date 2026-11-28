package app.wonton.ui.photos

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyGridScope
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.MediaFailure
import app.wonton.protocol.PhotosAction
import app.wonton.ui.ListSkeleton
import app.wonton.ui.ProblemRow
import app.wonton.ui.R
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonTheme

/**
 * 时间线：按日分组的照片网格，月滚到了才取。整库取不成、库是空的各有整页的说明；已经有内容时取不成只在顶上
 * 留一条轻提示。下拉重取。
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun PhotosTimeline(state: PhotosState, padding: PaddingValues) {
    val timeline = state.view.timeline
    val failure = timeline.problem
    val refresh = { state.send(PhotosAction.Refresh) }
    when {
        timeline.months.isEmpty() && failure != null -> PhotoProblemPage(failure.problem, refresh)
        timeline.months.isEmpty() && timeline.loading -> ListSkeleton(WontonTheme.size.storeIcon)
        timeline.months.isEmpty() -> PhotoEmptyPage(
            stringResource(R.string.photo_library_empty_title),
            stringResource(R.string.photo_library_empty_hint),
        ) {
            WontonButton(stringResource(R.string.photo_library_refresh), refresh, kind = WontonButtonKind.SECONDARY)
        }
        else -> PullToRefreshBox(isRefreshing = timeline.loading, onRefresh = refresh, Modifier.fillMaxSize()) {
            val entries = remember(timeline) { timeline.entries() }
            // 已经有内容时取不成：顶上一条轻提示，格子照留。
            val stale: (LazyGridScope.() -> Unit)? = if (failure == null) null else {
                { staleRow(failure, refresh) }
            }
            PhotoGrid(entries, state, padding, header = stale)
        }
    }
}

private fun LazyGridScope.staleRow(failure: MediaFailure, onRetry: () -> Unit) {
    item(key = "stale", span = { GridItemSpan(maxLineSpan) }) {
        Box(Modifier.padding(WontonTheme.spacing.pageInset)) { ProblemRow(failure.problem, onRetry) }
    }
}
