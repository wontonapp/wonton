package app.wonton.android

import app.wonton.protocol.ConnectionNotice
import app.wonton.protocol.ConnectionQuality
import app.wonton.ui.R
import app.wonton.ui.lossRateText
import app.wonton.ui.poorNoticeText
import app.wonton.ui.qualityValueText
import app.wonton.ui.rttText
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** 哪种状态显示哪一条的映射（#847），不测文案本身。 */
class ConnectionQualityMappingTest {
    @Test
    fun poorShowsOnlyWhenConnectedWithoutOtherNotice() {
        assertEquals(
            R.string.connection_poor,
            poorNoticeText(connected = true, notice = null, quality = ConnectionQuality.POOR, relayPath = false),
        )
        assertEquals(
            R.string.connection_poor_relay,
            poorNoticeText(connected = true, notice = null, quality = ConnectionQuality.POOR, relayPath = true),
        )
        // 断开了不显示。
        assertNull(
            poorNoticeText(connected = false, notice = null, quality = ConnectionQuality.POOR, relayPath = false),
        )
        // 别的通知优先。
        assertNull(
            poorNoticeText(
                connected = true, notice = ConnectionNotice.Connecting,
                quality = ConnectionQuality.POOR, relayPath = false,
            ),
        )
        // Good、Unknown、Other 都不显示。
        assertNull(poorNoticeText(true, null, ConnectionQuality.GOOD, false))
        assertNull(poorNoticeText(true, null, ConnectionQuality.UNKNOWN, false))
        assertNull(poorNoticeText(true, null, ConnectionQuality.OTHER, false))
    }

    @Test
    fun qualityValuesMapUncertainForUnknownAndOther() {
        assertEquals(R.string.connection_quality_good, qualityValueText(ConnectionQuality.GOOD))
        assertEquals(R.string.connection_quality_poor, qualityValueText(ConnectionQuality.POOR))
        assertEquals(R.string.connection_unknown, qualityValueText(ConnectionQuality.UNKNOWN))
        assertEquals(R.string.connection_unknown, qualityValueText(ConnectionQuality.OTHER))
    }

    @Test
    fun lossRateFormatsOneDecimalPercentOrNull() {
        assertNull(lossRateText(null))
        assertEquals("0.0%", lossRateText(0u.toUShort()))
        assertEquals("7.3%", lossRateText(73u.toUShort()))
        assertEquals("100.0%", lossRateText(1000u.toUShort()))
    }

    @Test
    fun rttFormatsMsOrNull() {
        assertNull(rttText(null))
        assertEquals("1240 ms", rttText(1240u))
    }
}
