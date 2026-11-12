package app.wonton.ui

import android.app.DatePickerDialog
import android.app.TimePickerDialog
import android.text.format.DateFormat
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import app.wonton.ui.design.WontonRadioRow
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import java.util.Calendar
import java.util.concurrent.TimeUnit

private enum class ShareExpiry { INDEFINITE, HOUR, TODAY, CUSTOM }

@Composable
internal fun ShareExpiryPicker(expires: ULong?, enabled: Boolean, onChange: (ULong?) -> Unit) {
    val context = LocalContext.current
    var selected by remember { mutableStateOf(if (expires == null) ShareExpiry.INDEFINITE else ShareExpiry.CUSTOM) }
    Text(stringResource(R.string.share_expires), style = WontonTheme.typography.sectionTitle,
        color = WontonTheme.colors.textPrimary.toColor())
    WontonRadioRow(stringResource(R.string.share_indefinite), selected == ShareExpiry.INDEFINITE, enabled, {
        selected = ShareExpiry.INDEFINITE
        onChange(null)
    })
    WontonRadioRow(stringResource(R.string.share_hour), selected == ShareExpiry.HOUR, enabled, {
        selected = ShareExpiry.HOUR
        onChange((System.currentTimeMillis() + TimeUnit.HOURS.toMillis(1)).toULong())
    })
    WontonRadioRow(stringResource(R.string.share_today), selected == ShareExpiry.TODAY, enabled, {
        selected = ShareExpiry.TODAY
        val end = Calendar.getInstance().apply {
            add(Calendar.DAY_OF_MONTH, 1)
            set(Calendar.HOUR_OF_DAY, 0)
            set(Calendar.MINUTE, 0)
            set(Calendar.SECOND, 0)
            set(Calendar.MILLISECOND, 0)
        }
        onChange(end.timeInMillis.toULong())
    })
    WontonRadioRow(stringResource(R.string.share_custom), selected == ShareExpiry.CUSTOM, enabled, {
        val date = Calendar.getInstance().apply { expires?.let { timeInMillis = it.toLong() } }
        val picker = DatePickerDialog(context, { _, year, month, day ->
            date.set(Calendar.YEAR, year)
            date.set(Calendar.MONTH, month)
            date.set(Calendar.DAY_OF_MONTH, day)
            TimePickerDialog(context, { _, hour, minute ->
                date.set(Calendar.HOUR_OF_DAY, hour)
                date.set(Calendar.MINUTE, minute)
                date.set(Calendar.SECOND, 0)
                date.set(Calendar.MILLISECOND, 0)
                selected = ShareExpiry.CUSTOM
                onChange(date.timeInMillis.toULong())
            }, date.get(Calendar.HOUR_OF_DAY), date.get(Calendar.MINUTE), DateFormat.is24HourFormat(context)).show()
        }, date.get(Calendar.YEAR), date.get(Calendar.MONTH), date.get(Calendar.DAY_OF_MONTH))
        picker.show()
    }, value = if (selected == ShareExpiry.CUSTOM) shareExpiryText(expires) else null)
}
