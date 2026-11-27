package app.wonton.ui.photos

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyGridItemScope
import androidx.compose.foundation.lazy.grid.LazyGridScope
import androidx.compose.foundation.lazy.grid.LazyGridState
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.onLongClick
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.wonton.protocol.PhotoRendition
import app.wonton.protocol.PhotosAction
import app.wonton.ui.ProblemRow
import app.wonton.ui.R
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import kotlin.math.roundToInt

/** 一格的目标边长，从大到小四档；列数按屏宽除出来，宽屏自然多几列。 */
private val CELL_SIZES = listOf(180.dp, 135.dp, 100.dp, 68.dp)
private const val DEFAULT_LEVEL = 2
private val CELL_GAP = 2.dp

/**
 * 照片网格：时间线、相册和搜索结果共用。捏合换列数，点一格看大图，长按进入选择、接着拖动连选；选择中点一格是
 * 选上或取消。[header] 是排在格子前面的整行内容（搜索页的筛选条件）。
 */
@Composable
internal fun PhotoGrid(
    entries: List<GridEntry>,
    state: PhotosState,
    padding: PaddingValues,
    modifier: Modifier = Modifier,
    grid: LazyGridState = rememberLazyGridState(),
    header: (LazyGridScope.() -> Unit)? = null,
) {
    var level by rememberSaveable { mutableIntStateOf(DEFAULT_LEVEL) }
    val selected = state.view.selected
    val ids = remember(entries) { entries.tileIds() }
    val selection = GridSelection(ids, selected) { changed, on ->
        state.send(PhotosAction.Select(changed, on))
    }
    BoxWithConstraints(modifier.fillMaxSize()) {
        val columns = (maxWidth / CELL_SIZES[level]).roundToInt().coerceAtLeast(1)
        LazyVerticalGrid(
            GridCells.Fixed(columns),
            Modifier.fillMaxSize()
                .pinchLevels { step -> level = (level + step).coerceIn(0, CELL_SIZES.lastIndex) }
                .dragSelect(grid, selection),
            state = grid,
            contentPadding = padding,
            horizontalArrangement = Arrangement.spacedBy(CELL_GAP),
            verticalArrangement = Arrangement.spacedBy(CELL_GAP),
        ) {
            header?.invoke(this)
            items(
                entries,
                key = { it.key },
                span = { if (it.isCell) GridItemSpan(1) else GridItemSpan(maxLineSpan) },
                contentType = { it::class },
            ) { entry -> GridEntryView(entry, state, selected) }
        }
    }
}

private val GridEntry.isCell: Boolean get() = this is GridEntry.Tile || this is GridEntry.Placeholder

@Composable
private fun LazyGridItemScope.GridEntryView(entry: GridEntry, state: PhotosState, selected: List<String>) {
    when (entry) {
        is GridEntry.Tile -> TileCell(entry, state, selected)
        is GridEntry.Day -> DayHeader(entry, state, selected)
        is GridEntry.PendingMonth -> {
            LaunchedEffect(entry.month) { state.send(PhotosAction.OpenMonth(entry.month)) }
            SectionTitle(monthTitle(LocalContext.current, entry.month))
        }
        is GridEntry.Placeholder -> Box(
            Modifier.aspectRatio(1f).background(WontonTheme.colors.surfaceSubtle.toColor()),
        )
        is GridEntry.FailedMonth -> Column {
            SectionTitle(monthTitle(LocalContext.current, entry.month))
            Box(Modifier.padding(horizontal = WontonTheme.spacing.pageInset)) {
                ProblemRow(entry.failure.problem) { state.send(PhotosAction.OpenMonth(entry.month)) }
            }
        }
        is GridEntry.More -> MoreRow(entry, state)
    }
}

@Composable
private fun LazyGridItemScope.TileCell(entry: GridEntry.Tile, state: PhotosState, selected: List<String>) {
    val tile = entry.tile
    val selecting = selected.isNotEmpty()
    val isSelected = tile.id in selected
    val toggle = { state.send(PhotosAction.Select(listOf(tile.id), !isSelected)) }
    val selectLabel = stringResource(R.string.photo_library_select)
    PhotoTileView(
        tile,
        state.ref(tile.id, PhotoRendition.THUMBNAIL),
        selecting,
        isSelected,
        Modifier.animateItem()
            .clickable(role = Role.Button) {
                if (selecting) toggle() else state.send(PhotosAction.OpenDetail(tile.id))
            }
            .semantics {
                if (selecting) this.selected = isSelected
                // 长按选中由网格的手势认（要接着拖动连选）；读屏走这个动作。
                onLongClick(selectLabel) {
                    toggle()
                    true
                }
            },
    )
}

/** 一天的标题；选择中标题前多一个圈，点它整天选上或取消。 */
@Composable
private fun DayHeader(entry: GridEntry.Day, state: PhotosState, selected: List<String>) {
    val all = entry.ids.all { it in selected }
    Row(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .then(
                if (selected.isEmpty()) Modifier else Modifier.clickable(role = Role.Checkbox) {
                    state.send(PhotosAction.Select(entry.ids, !all))
                },
            )
            .padding(horizontal = WontonTheme.spacing.pageInset),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (selected.isNotEmpty()) SelectionMark(all)
        Text(
            dayTitle(LocalContext.current, entry.date),
            Modifier.semantics { heading() },
            style = WontonTheme.typography.itemTitle,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
    }
}

@Composable
internal fun SectionTitle(title: String) {
    Box(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .padding(horizontal = WontonTheme.spacing.pageInset),
        contentAlignment = Alignment.CenterStart,
    ) {
        Text(
            title,
            Modifier.semantics { heading() },
            style = WontonTheme.typography.itemTitle,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
    }
}

/** 一段的末尾：出现在屏上就接着取下一页；翻页没成时留一条轻提示和重试，已取到的照留。 */
@Composable
private fun MoreRow(entry: GridEntry.More, state: PhotosState) {
    val more = entry.month?.let { PhotosAction.LoadMoreMonth(it) }
        ?: if (state.view.album != null) PhotosAction.LoadMoreAlbum else PhotosAction.LoadMoreSearch
    val failure = entry.failure
    if (failure == null) LaunchedEffect(entry.key, entry.have) { state.send(more) }
    Box(
        Modifier.fillMaxWidth().padding(WontonTheme.spacing.pageInset),
        contentAlignment = Alignment.Center,
    ) {
        if (failure != null && !entry.loading) {
            ProblemRow(failure.problem) { state.send(more) }
        } else {
            CircularProgressIndicator(color = WontonTheme.colors.textSecondary.toColor())
        }
    }
}
