package app.wonton.ui

import app.wonton.protocol.PortMappingState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** 状态到原因文案键的映射：没映射成功的每个类别有自己的键，开着和关着不给原因。 */
class PortMappingReasonTest {
    @Test
    fun onAndOffNeedNoReason() {
        assertNull(portMappingReasonRes(null))
        assertNull(portMappingReasonRes(PortMappingState.ON))
        assertNull(portMappingReasonRes(PortMappingState.OFF))
    }

    @Test
    fun everyFailureStateHasItsOwnKey() {
        val keys = mapOf(
            PortMappingState.PORT_IN_USE to R.string.port_mapping_reason_port_in_use,
            PortMappingState.SERVER_TOO_OLD to R.string.port_mapping_reason_server_too_old,
            PortMappingState.TUNNEL_DISCONNECTED to R.string.port_mapping_reason_tunnel_disconnected,
            PortMappingState.PLUGIN_NOT_INSTALLED to R.string.port_mapping_reason_plugin_not_installed,
            PortMappingState.PORT_NOT_DECLARED to R.string.port_mapping_reason_port_not_declared,
            PortMappingState.PLUGIN_NOT_RUNNING to R.string.port_mapping_reason_plugin_not_running,
            PortMappingState.PLUGIN_UNREACHABLE to R.string.port_mapping_reason_plugin_unreachable,
            PortMappingState.OTHER to R.string.port_mapping_reason_other,
        )
        keys.forEach { (state, key) -> assertEquals(key, portMappingReasonRes(state)) }
        // 类别之间不共用一句话。
        assertEquals(keys.size, keys.values.distinct().size)
    }
}
