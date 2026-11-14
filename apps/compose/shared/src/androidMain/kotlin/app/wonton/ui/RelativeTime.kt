package app.wonton.ui

import android.icu.text.DateFormat
import android.icu.text.RelativeDateTimeFormatter
import android.icu.text.RelativeDateTimeFormatter.RelativeUnit
import android.icu.util.ULocale
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import java.util.Date
import kotlin.math.roundToInt

private const val SECOND_MS = 1000L
private const val SECONDS_PER_MINUTE = 60
private const val SECONDS_PER_HOUR = 60.0 * SECONDS_PER_MINUTE
private const val SECONDS_PER_DAY = SECONDS_PER_HOUR * 24
private const val SECONDS_PER_MONTH = SECONDS_PER_DAY * 30

/** 相对时间（“3 小时前”），按 App 当前的语言格式化（ADR-0043）；一分钟内是“刚刚”，超过一个月用日期。 */
@Composable
internal fun RelativeTime(ms: Long): String {
    val locale = LocalConfiguration.current.locales[0]
    val formatter = remember(locale) { RelativeDateTimeFormatter.getInstance(ULocale.forLocale(locale)) }
    val dateFormatter = remember(locale) { DateFormat.getDateInstance(DateFormat.SHORT, locale) }
    val seconds = ((System.currentTimeMillis() - ms) / SECOND_MS.toDouble()).coerceAtLeast(0.0)
    return when {
        seconds < SECONDS_PER_MINUTE -> stringResource(R.string.peers_just_now)
        seconds < SECONDS_PER_MONTH -> {
            // Android 的 RelativeDateTimeFormatter 没有按数量自动挑单位的两参 format，这里按数量级挑。
            val (value, unit) = when {
                seconds < SECONDS_PER_HOUR -> rounded(seconds, SECONDS_PER_MINUTE.toDouble()) to RelativeUnit.MINUTES
                seconds < SECONDS_PER_DAY -> rounded(seconds, SECONDS_PER_HOUR) to RelativeUnit.HOURS
                else -> rounded(seconds, SECONDS_PER_DAY) to RelativeUnit.DAYS
            }
            formatter.format(value, RelativeDateTimeFormatter.Direction.LAST, unit)
        }
        else -> dateFormatter.format(Date(ms))
    }
}

/** 按单位折算成整数（至少 1），ICU 的相对格式带小数不好看。 */
internal fun rounded(seconds: Double, divisor: Double) =
    (seconds / divisor).roundToInt().coerceAtLeast(1).toDouble()
