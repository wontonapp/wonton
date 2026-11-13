package app.wonton.ui

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class QrScanGateTest {
    @Test
    fun singleScanRejectsBothRepeatedAndDifferentCodes() {
        val session = QrScanGate(singleScan = true)
        assertTrue(session.accept("first"))
        assertTrue(session.finished)
        assertFalse(session.accept("first"))
        assertFalse(session.accept("second"))
    }

    @Test
    fun newScanSessionAcceptsThePreviouslyScannedCode() {
        val firstSession = QrScanGate(singleScan = true)
        assertTrue(firstSession.accept("code"))
        val nextSession = QrScanGate(singleScan = true)
        assertTrue(nextSession.accept("code"))
    }

    @Test
    fun continuousScanningStillAcceptsChangedCodes() {
        val session = QrScanGate(singleScan = false)
        assertTrue(session.accept("first"))
        assertFalse(session.accept("first"))
        assertTrue(session.accept("second"))
        assertTrue(session.accept("first"))
        assertFalse(session.finished)
    }
}
