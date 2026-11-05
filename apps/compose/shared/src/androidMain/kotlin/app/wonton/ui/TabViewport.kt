package app.wonton.ui

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.union
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.Dp
import app.wonton.client.TabEnvironment
import app.wonton.client.TabPlatform
import app.wonton.client.TabSafeArea
import app.wonton.ui.design.WontonTheme
import java.util.TimeZone

@Composable
internal fun tabSafeInsets(): PaddingValues =
    WindowInsets.systemBars.union(WindowInsets.displayCutout).asPaddingValues()

@Composable
internal fun tabEnvironment(top: Dp, bottom: Dp, edge: Boolean, safe: PaddingValues): TabEnvironment {
    val configuration = LocalConfiguration.current
    val direction = LocalLayoutDirection.current
    return TabEnvironment(
        configuration.locales[0].toLanguageTag(), TimeZone.getDefault().id, TabPlatform.ANDROID,
        WontonTheme.isDark, top.value.toDouble(), bottom.value.toDouble(),
        TabSafeArea(
            if (edge) safe.calculateTopPadding().value.toDouble() else 0.0,
            if (edge) safe.calculateRightPadding(direction).value.toDouble() else 0.0,
            if (edge) safe.calculateBottomPadding().value.toDouble() else 0.0,
            if (edge) safe.calculateLeftPadding(direction).value.toDouble() else 0.0,
        ),
    )
}
