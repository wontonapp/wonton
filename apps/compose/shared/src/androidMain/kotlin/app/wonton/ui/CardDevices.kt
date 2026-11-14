package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.wonton.client.DeviceChoices
import app.wonton.client.chosenDeviceIds
import app.wonton.client.deviceChoices
import app.wonton.client.toggleDevice
import app.wonton.protocol.ConfirmationCard
import app.wonton.protocol.ConfirmationState
import app.wonton.protocol.ConfirmationView
import app.wonton.protocol.PluginDevice

/**
 * 卡片上主人点选的设备（ADR-0036）：没点过时是卡片给的。卡片没带设备项时没有“设备”一节，也不调核心：
 * 最高风险的第二张卡片只带权限项，沿用第一张的选择。
 */
internal class CardDevices(private val card: ConfirmationCard) {
    private var picked by mutableStateOf<List<PluginDevice>?>(null)
    private val devices: List<PluginDevice> get() = picked ?: card.devices

    val section: DeviceChoices? get() = if (devices.isEmpty()) null else deviceChoices(devices, card.permissions)

    /** 随回答发回去的设备 ID。 */
    val chosenIds: List<String>
        get() = if (devices.isEmpty()) emptyList() else chosenDeviceIds(devices, card.permissions)

    fun toggle(id: String) {
        picked = toggleDevice(devices, id)
    }
}

/** 等回答的卡片上能点选的“设备”一节；正在发回答时点不动。 */
@Composable
internal fun CardDeviceSection(devices: CardDevices, view: ConfirmationView) {
    val section = devices.section ?: return
    if (view.card.state != ConfirmationState.Pending) return
    DeviceChoicesSection(section, enabled = view.sending == null, onToggle = devices::toggle)
}
