package app.wonton.ui.photos

import android.text.format.DateUtils
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.DateRangePicker
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberDateRangePickerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.PhotoSearch
import app.wonton.ui.R
import app.wonton.ui.design.WontonRadioRow
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTheme
import java.time.LocalDate
import java.time.ZoneId

private const val RECENT_DAYS = 30L
private const val DAY_MS = 24L * 60 * 60 * 1000

/** 搜索页按时间筛的几种取法。 */
internal enum class TimeFilter(val title: Int) {
    ANY(R.string.photo_library_time_any),
    RECENT(R.string.photo_library_time_recent),
    THIS_YEAR(R.string.photo_library_time_this_year),
    LAST_YEAR(R.string.photo_library_time_last_year),
    CUSTOM(R.string.photo_library_time_custom),
    ;

    /** 这种取法对应的起止时刻（Unix 毫秒，按手机时区的自然日）；自选的由日期选择器给。 */
    fun range(): Pair<ULong?, ULong?> {
        val zone = ZoneId.systemDefault()
        val today = LocalDate.now(zone)
        fun start(day: LocalDate) = day.atStartOfDay(zone).toInstant().toEpochMilli().toULong()
        return when (this) {
            ANY, CUSTOM -> null to null
            RECENT -> start(today.minusDays(RECENT_DAYS)) to null
            THIS_YEAR -> start(today.withDayOfYear(1)) to null
            LAST_YEAR -> start(today.withDayOfYear(1).minusYears(1)) to start(today.withDayOfYear(1)) - 1u
        }
    }
}

/** 时间那颗胶囊上的字：没筛写“时间”，自选的写起止日期，其余写取法的名字。 */
@Composable
internal fun timeLabel(time: TimeFilter, query: PhotoSearch): String {
    val from = query.from
    val to = query.to
    return when {
        time == TimeFilter.ANY -> stringResource(R.string.photo_library_filter_time)
        time == TimeFilter.CUSTOM && from != null && to != null -> DateUtils.formatDateRange(
            LocalContext.current, from.toLong(), to.toLong(), DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_ABBREV_ALL,
        )
        else -> stringResource(time.title)
    }
}

/** 选时间范围的面板；选“自选日期”接着出系统样式的日期范围选择器。 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun PhotoTimeSheet(
    current: TimeFilter,
    onDismiss: () -> Unit,
    onPick: (TimeFilter, from: ULong?, to: ULong?) -> Unit,
) {
    var custom by remember { mutableStateOf(false) }
    if (custom) {
        val picker = rememberDateRangePickerState()
        val start = picker.selectedStartDateMillis
        val end = picker.selectedEndDateMillis
        DatePickerDialog(
            onDismissRequest = onDismiss,
            confirmButton = {
                TextButton(
                    onClick = {
                        if (start != null && end != null) {
                            // 选择器给的是 UTC 零点：换成手机时区那两天的起和止。
                            onPick(TimeFilter.CUSTOM, localDayStart(start), localDayStart(end) + DAY_MS.toULong() - 1u)
                        }
                        onDismiss()
                    },
                    enabled = start != null && end != null,
                ) { Text(stringResource(R.string.common_done)) }
            },
            dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.common_cancel)) } },
        ) { DateRangePicker(picker) }
        return
    }
    WontonSheet(onDismiss, stringResource(R.string.photo_library_filter_time)) { close ->
        Column(
            Modifier.padding(horizontal = WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        ) {
            TimeFilter.entries.forEach { option ->
                WontonRadioRow(stringResource(option.title), option == current, enabled = true, onSelect = {
                    if (option == TimeFilter.CUSTOM) {
                        custom = true
                    } else {
                        val (from, to) = option.range()
                        close { onPick(option, from, to) }
                    }
                })
            }
        }
    }
}

/** 日期选择器给的 UTC 零点换成手机时区同一天的零点。 */
private fun localDayStart(utcMidnight: Long): ULong {
    val day = LocalDate.ofEpochDay(utcMidnight / DAY_MS)
    return day.atStartOfDay(ZoneId.systemDefault()).toInstant().toEpochMilli().toULong()
}
