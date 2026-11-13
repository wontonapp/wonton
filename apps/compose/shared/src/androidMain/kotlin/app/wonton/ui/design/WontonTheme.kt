package app.wonton.ui.design

import android.graphics.Color as AndroidColor
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.LocalActivity
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.Density
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.FontStyle
import app.wonton.protocol.Rgb
import app.wonton.protocol.TextSize
import app.wonton.protocol.ThemeColors
import app.wonton.protocol.ThemeMode
import app.wonton.ui.design.generated.WontonGesture
import app.wonton.ui.design.generated.WontonMotion
import app.wonton.ui.design.generated.WontonRadii
import app.wonton.ui.design.generated.WontonRatio
import app.wonton.ui.design.generated.WontonSize
import app.wonton.ui.design.generated.WontonSpacing
import app.wonton.ui.design.generated.WontonTypography

fun Rgb.toColor(): Color = Color(red.toInt(), green.toInt(), blue.toInt())

private val LocalWontonColors = compositionLocalOf<ThemeColors> {
    error("No color tokens: WontonTheme provider is missing")
}
private val LocalWontonTypography = compositionLocalOf<WontonTypography> {
    error("No typography tokens: WontonTheme provider is missing")
}
private val LocalWontonFontScale = compositionLocalOf<Float?> { null }
private val LocalWontonDark = compositionLocalOf { false }

/** ADR-0026 的令牌，和 SwiftUI 的 `WontonTheme` 同名。 */
object WontonTheme {
    val spacing = WontonSpacing
    val radii = WontonRadii
    val size = WontonSize
    val ratio = WontonRatio
    val motion = WontonMotion
    val gesture = WontonGesture

    val isDark: Boolean
        @Composable @ReadOnlyComposable
        get() = LocalWontonDark.current

    /** 带着用户选的字体。字号在 [WontonTheme] 里经 `fontScale` 统一放大，这里还是令牌的值。 */
    val typography: WontonTypography
        @Composable @ReadOnlyComposable
        get() = LocalWontonTypography.current

    /** 核心推导好的颜色令牌，按用户选的明暗选好了一套（ADR-0026）。 */
    val colors: ThemeColors
        @Composable @ReadOnlyComposable
        get() = LocalWontonColors.current
}

/** 把外观交给下面的所有界面。还没自绘的 Material 控件经 [materialColors] 取同一组颜色。 */
@Composable
fun WontonTheme(appearance: AppearanceViewModel, content: @Composable () -> Unit) {
    val dark = when (appearance.themeMode) {
        ThemeMode.SYSTEM -> isSystemInDarkTheme()
        ThemeMode.LIGHT -> false
        ThemeMode.DARK -> true
    }
    SystemBarsFollow(dark)
    val colors = if (dark) appearance.dark else appearance.light
    val typography = remember(appearance.font) { WontonTypography(appearance.font.family) }
    CompositionLocalProvider(
        LocalWontonColors provides colors,
        LocalWontonTypography provides typography,
        LocalWontonFontScale provides appearance.textSize.fontScale,
        LocalWontonDark provides dark,
    ) {
        WontonFontScale {
            MaterialTheme(colorScheme = materialColors(colors)) {
                CompositionLocalProvider(LocalTextStyle provides typography.body, content = content)
            }
        }
    }
}

/**
 * 盖上用户选的字号：所有 sp 都跟着放大，dp 的布局不变。
 *
 * `Dialog` 和 `ModalBottomSheet` 是单独的窗口，Compose 在里面按窗口重给 `LocalDensity`，根上盖的传不进去，
 * 窗口的内容要再套一层（别的 CompositionLocal 传得进去）。
 */
@Composable
fun WontonFontScale(content: @Composable () -> Unit) {
    val density = LocalDensity.current
    val scale = LocalWontonFontScale.current
    val scaled = if (scale != null) Density(density.density, scale) else density
    CompositionLocalProvider(LocalDensity provides scaled, content = content)
}

/** 状态栏和导航栏的图标跟着用户选的明暗，不是系统的。 */
@Composable
private fun SystemBarsFollow(dark: Boolean) {
    val activity = LocalActivity.current as? ComponentActivity
    DisposableEffect(activity, dark) {
        activity?.enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.auto(AndroidColor.TRANSPARENT, AndroidColor.TRANSPARENT) { dark },
            navigationBarStyle = SystemBarStyle.auto(AndroidColor.TRANSPARENT, AndroidColor.TRANSPARENT) { dark },
        )
        onDispose {}
    }
}
/** 除了跟随系统，每一档都盖过系统的字号。取值是 iOS 对应档位的正文放大比例（ADR-0028）。 */
private val TextSize.fontScale: Float?
    get() = when (this) {
        TextSize.SYSTEM -> null
        TextSize.SMALL -> 0.88f
        TextSize.STANDARD -> 1f
        TextSize.LARGE -> 1.12f
        TextSize.EXTRA_LARGE -> 1.24f
    }

/** 核心不会把圆体列给 Android：Android 的系统字体里没有。 */
val FontStyle.family: FontFamily?
    get() = when (this) {
        FontStyle.DEFAULT, FontStyle.ROUNDED -> null
        FontStyle.SERIF -> FontFamily.Serif
        FontStyle.MONOSPACED -> FontFamily.Monospace
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
