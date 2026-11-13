package app.wonton.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PairingRefusalTest {
    @Test
    fun immediateRefusalKeepsCategoryAndMovesRawReasonIntoDetails() {
        val reason = "invalid code payload"
        val refusal = Problem(titleRes = R.string.problem_invalid_input_title, nextStep = reason)
        val displayed = refusal.pairingRefusal()
        assertEquals(refusal.titleRes, displayed.titleRes)
        assertNull(displayed.nextStep)
        assertEquals(reason, displayed.detail)
    }

    @Test
    fun existingDiagnosticsAndLocalizedRecoveryStepAreRetained() {
        val refusal = Problem(
            titleRes = R.string.pairing_failure_transport,
            nextStepRes = R.string.pairing_hint_transport,
            detail = "transport diagnostic",
        )
        assertEquals(refusal, refusal.pairingRefusal())
    }
}
