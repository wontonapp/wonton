package app.wonton.ui.design

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.ui.graphics.Color
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.Rgb
import app.wonton.protocol.ThemeColors
import app.wonton.ui.design.generated.WontonMotion
import app.wonton.ui.design.generated.WontonRadii
import app.wonton.ui.design.generated.WontonRatio
import app.wonton.ui.design.generated.WontonSize
import app.wonton.ui.design.generated.WontonSpacing
import app.wonton.ui.design.generated.WontonTypography

fun Rgb.toColor(): Color = Color(red.toInt(), green.toInt(), blue.toInt())

private val LocalWontonColors = compositionLocalOf<ThemeColors> { error("WontonTheme 之外没有颜色令牌") }

/** ADR-0026 的令牌，和 SwiftUI 的 `WontonTheme` 同名。 */
object WontonTheme {
    val spacing = WontonSpacing
    val radii = WontonRadii
    val size = WontonSize
    val ratio = WontonRatio
    val motion = WontonMotion
    val typography = WontonTypography

    /** 核心推导好的颜色令牌，按系统的明暗选好了一套（ADR-0026）。 */
    val colors: ThemeColors
        @Composable @ReadOnlyComposable
        get() = LocalWontonColors.current
}

/** 把外观交给下面的所有界面。还没自绘的 Material 控件经 [materialColors] 取同一组颜色。 */
@Composable
fun WontonTheme(appearance: AppearanceViewModel, content: @Composable () -> Unit) {
    val colors = if (isSystemInDarkTheme()) appearance.dark else appearance.light
    CompositionLocalProvider(LocalWontonColors provides colors) {
        MaterialTheme(colorScheme = materialColors(colors), content = content)
    }
}

/**
 * 主按钮和链接用 `primary`：重点色只留给选中、自己的气泡、占位图和角标。不写全的话，没写的那些角色
 * （比如各级 surfaceContainer）会落回 Material 默认的紫色系。
 */
private fun materialColors(colors: ThemeColors) = lightColorScheme(
    primary = colors.primary.toColor(),
    onPrimary = colors.onPrimary.toColor(),
    primaryContainer = colors.accentFill.toColor(),
    onPrimaryContainer = colors.textOnAccentFill.toColor(),
    inversePrimary = colors.onPrimary.toColor(),
    secondary = colors.textSecondary.toColor(),
    onSecondary = colors.canvas.toColor(),
    secondaryContainer = colors.accentFill.toColor(),
    onSecondaryContainer = colors.textOnAccentFill.toColor(),
    tertiary = colors.textSecondary.toColor(),
    onTertiary = colors.canvas.toColor(),
    tertiaryContainer = colors.surfaceSubtle.toColor(),
    onTertiaryContainer = colors.textPrimary.toColor(),
    background = colors.canvas.toColor(),
    onBackground = colors.textPrimary.toColor(),
    surface = colors.surface.toColor(),
    onSurface = colors.textPrimary.toColor(),
    surfaceVariant = colors.surfaceSubtle.toColor(),
    onSurfaceVariant = colors.textSecondary.toColor(),
    surfaceTint = Color.Transparent,
    inverseSurface = colors.textPrimary.toColor(),
    inverseOnSurface = colors.canvas.toColor(),
    error = colors.danger.toColor(),
    onError = colors.canvas.toColor(),
    errorContainer = colors.surfaceSubtle.toColor(),
    onErrorContainer = colors.danger.toColor(),
    outline = colors.separator.toColor(),
    outlineVariant = colors.separator.toColor(),
    scrim = Color.Black,
    surfaceBright = colors.surface.toColor(),
    surfaceDim = colors.surfaceSubtle.toColor(),
    surfaceContainer = colors.surface.toColor(),
    surfaceContainerHigh = colors.surfaceSubtle.toColor(),
    surfaceContainerHighest = colors.surfaceSubtle.toColor(),
    surfaceContainerLow = colors.surface.toColor(),
    surfaceContainerLowest = colors.canvas.toColor(),
)
