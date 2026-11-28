package app.wonton.ui.photos

import android.content.Context
import android.text.format.DateUtils
import android.text.format.Formatter
import java.time.LocalDate
import java.time.YearMonth
import java.time.ZoneId
import java.time.format.DateTimeParseException

private const val SECOND_MS = 1000L

/** 核心给的拍摄日（`YYYY-MM-DD`，已按手机时差算好）写成系统语言的日期，带星期；今年的不带年份。 */
internal fun dayTitle(context: Context, date: String): String = try {
    val day = LocalDate.parse(date)
    val flags = DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_SHOW_WEEKDAY or
        if (day.year == LocalDate.now().year) DateUtils.FORMAT_NO_YEAR else DateUtils.FORMAT_SHOW_YEAR
    DateUtils.formatDateTime(context, day.startMillis(), flags)
} catch (_: DateTimeParseException) {
    date
}

/** 时间线的月（`YYYY-MM`）。 */
internal fun monthTitle(context: Context, month: String): String = try {
    val flags = DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_NO_MONTH_DAY or DateUtils.FORMAT_SHOW_YEAR
    DateUtils.formatDateTime(context, YearMonth.parse(month).atDay(1).startMillis(), flags)
} catch (_: DateTimeParseException) {
    month
}

private fun LocalDate.startMillis(): Long = atStartOfDay(ZoneId.systemDefault()).toInstant().toEpochMilli()

/** 拍摄时刻：日期、星期和时间。 */
internal fun takenAtText(context: Context, takenAt: ULong): String = DateUtils.formatDateTime(
    context,
    takenAt.toLong(),
    DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_SHOW_YEAR or DateUtils.FORMAT_SHOW_WEEKDAY or
        DateUtils.FORMAT_SHOW_TIME,
)

/** 大图页顶栏的日期。 */
internal fun takenDayText(context: Context, takenAt: ULong): String = DateUtils.formatDateTime(
    context,
    takenAt.toLong(),
    DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_SHOW_YEAR or DateUtils.FORMAT_ABBREV_MONTH,
)

/** 视频角标上的时长，`m:ss`。 */
internal fun durationText(durationMs: ULong): String = DateUtils.formatElapsedTime(durationMs.toLong() / SECOND_MS)

internal fun sizeText(context: Context, bytes: ULong): String = Formatter.formatShortFileSize(context, bytes.toLong())
