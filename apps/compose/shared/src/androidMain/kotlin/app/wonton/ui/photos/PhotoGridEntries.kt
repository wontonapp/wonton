package app.wonton.ui.photos

import app.wonton.protocol.MediaFailure
import app.wonton.protocol.PhotoMonth
import app.wonton.protocol.PhotoPage
import app.wonton.protocol.PhotoTile
import app.wonton.protocol.PhotoTimeline

/** 还没取条目的月先占这么多格，滚到了才向核心要。 */
private const val PENDING_CELLS = 12

/** 网格里的一项。[key] 在一个网格里唯一：懒加载网格拿它认项，连选拿它认手指底下是哪一格。 */
internal sealed interface GridEntry {
    val key: String

    /** 一格照片。 */
    data class Tile(val tile: PhotoTile) : GridEntry {
        override val key get() = TILE_KEY + tile.id
    }

    /** 一天的标题，占一整行。 */
    data class Day(val date: String, val ids: List<String>) : GridEntry {
        override val key get() = "day:$date"
    }

    /** 还没取的月：标题占一整行，出现在屏上就向核心要这个月。 */
    data class PendingMonth(val month: String, val count: UInt) : GridEntry {
        override val key get() = "month:$month"
    }

    /** 还没取的月底下占位的格子。 */
    data class Placeholder(val month: String, val index: Int) : GridEntry {
        override val key get() = "pending:$month:$index"
    }

    /** 这个月的第一页没取成：标题、原因和重试。 */
    data class FailedMonth(val month: String, val failure: MediaFailure) : GridEntry {
        override val key get() = "failed:$month"
    }

    /**
     * 一段的末尾还有下一页：出现在屏上就接着取；[failure] 是翻页没成的原因。[month] 为空是相册或搜索结果。
     * [have] 是已经取到几格：取完一页它还在屏上时，靠这个数变了再取下一页。
     */
    data class More(val month: String?, val have: Int, val loading: Boolean, val failure: MediaFailure?) : GridEntry {
        override val key get() = "more:${month.orEmpty()}"
    }

    companion object {
        const val TILE_KEY = "tile:"
    }
}

/** 时间线摊平成网格的项：取过的月按日分组，没取的月出标题和占位。同一个 id 只出一格。 */
internal fun PhotoTimeline.entries(): List<GridEntry> {
    val seen = HashSet<String>()
    return months.flatMap { it.entries(seen) }
}

private fun PhotoMonth.entries(seen: MutableSet<String>): List<GridEntry> {
    val failure = problem
    val entries = ArrayList<GridEntry>()
    when {
        failure != null && days.isEmpty() -> entries += GridEntry.FailedMonth(month, failure)
        !opened || days.isEmpty() && loading -> {
            entries += GridEntry.PendingMonth(month, count)
            repeat(minOf(count.toInt(), PENDING_CELLS)) { entries += GridEntry.Placeholder(month, it) }
        }
        else -> {
            for (day in days) {
                val tiles = day.tiles.filter { seen.add(it.id) }
                if (tiles.isNotEmpty()) {
                    entries += GridEntry.Day(day.date, tiles.map { it.id })
                    tiles.mapTo(entries) { GridEntry.Tile(it) }
                }
            }
            if (hasMore || loadingMore) {
                entries += GridEntry.More(month, days.sumOf { it.tiles.size }, loadingMore, failure)
            }
        }
    }
    return entries
}

/** 相册、搜索结果的一页页条目。 */
internal fun PhotoPage.entries(): List<GridEntry> {
    val seen = HashSet<String>()
    val entries = ArrayList<GridEntry>()
    tiles.filter { seen.add(it.id) }.mapTo(entries) { GridEntry.Tile(it) }
    if (hasMore || loadingMore) {
        entries += GridEntry.More(null, tiles.size, loadingMore, problem.takeIf { tiles.isNotEmpty() })
    }
    return entries
}

/** 网格里的格子 id，按显示的先后：拖动连选按它取一段。 */
internal fun List<GridEntry>.tileIds(): List<String> = mapNotNull { (it as? GridEntry.Tile)?.tile?.id }
