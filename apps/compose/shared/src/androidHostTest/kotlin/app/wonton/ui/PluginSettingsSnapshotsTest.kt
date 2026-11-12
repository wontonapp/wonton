package app.wonton.ui

import app.cash.paparazzi.DeviceConfig
import app.cash.paparazzi.Paparazzi
import org.junit.Rule
import org.junit.Test

/**
 * 插件设置页的参考图（ADR-0077）：每种控件、有问题、重启提示和保存中，一张一份。
 * 设备、语言和主题与 PreviewSnapshotsTest 保持一致，参考图在 `src/androidHostTest/snapshots`，直接进 git。
 */
class PluginSettingsSnapshotsTest {
    @get:Rule
    val paparazzi = Paparazzi(
        deviceConfig = DeviceConfig.PIXEL_5.copy(locale = "en", softButtons = false),
        theme = "android:Theme.Material.Light.NoActionBar.Fullscreen",
    )

    @Test
    fun pluginSettingsFields() = paparazzi.snapshot { PluginSettingsFieldsPreview() }

    @Test
    fun pluginSettingsProblems() = paparazzi.snapshot { PluginSettingsProblemsPreview() }

    @Test
    fun pluginSettingsRestartHint() = paparazzi.snapshot { PluginSettingsRestartHintPreview() }

    @Test
    fun pluginSettingsSaving() = paparazzi.snapshot { PluginSettingsSavingPreview() }
}
