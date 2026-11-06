package app.wonton.ui

import android.os.Build
import android.view.WindowManager
import androidx.activity.compose.LocalActivity
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.rememberUpdatedState
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.LIGHT_NAVIGATION_SCRIM
import app.wonton.ui.design.DARK_NAVIGATION_SCRIM


@Suppress("DEPRECATION")
@Composable
internal fun FullscreenSystemBars(fullscreen: Boolean, web: Boolean) {
    val window = LocalActivity.current?.window ?: return
    val dark = WontonTheme.isDark
    DisposableEffect(window, fullscreen, dark) {
        val controller = WindowCompat.getInsetsController(window, window.decorView)
        val previousBehavior = controller.systemBarsBehavior
        val previousCutout = window.attributes.layoutInDisplayCutoutMode
        if (fullscreen) {
            window.attributes = window.attributes.apply {
                layoutInDisplayCutoutMode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                    WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
                } else {
                    WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
                }
            }
            controller.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            controller.hide(WindowInsetsCompat.Type.statusBars())
        } else {
            controller.show(WindowInsetsCompat.Type.statusBars())
        }
        controller.show(WindowInsetsCompat.Type.navigationBars())
        onDispose {
            controller.systemBarsBehavior = previousBehavior
            window.attributes = window.attributes.apply { layoutInDisplayCutoutMode = previousCutout }
            controller.show(WindowInsetsCompat.Type.statusBars())
        }
    }
    SideEffect {
        window.statusBarColor = android.graphics.Color.TRANSPARENT
        window.navigationBarColor = if (web) android.graphics.Color.TRANSPARENT
            else if (dark) DARK_NAVIGATION_SCRIM else LIGHT_NAVIGATION_SCRIM
        window.isNavigationBarContrastEnforced = !web
        if (!web) {
            val controller = WindowCompat.getInsetsController(window, window.decorView)
            controller.isAppearanceLightStatusBars = !dark
            controller.isAppearanceLightNavigationBars = !dark
        }
    }
}

@Composable
internal fun PageSystemBarIcons(edges: TabPageEdges, fullscreen: Boolean, webBottom: Boolean) {
    val window = LocalActivity.current?.window ?: return
    val dark = WontonTheme.isDark
    val currentDark = rememberUpdatedState(dark)
    DisposableEffect(window) {
        onDispose {
            val controller = WindowCompat.getInsetsController(window, window.decorView)
            controller.isAppearanceLightStatusBars = !currentDark.value
            controller.isAppearanceLightNavigationBars = !currentDark.value
        }
    }
    val top = edges.pixels[TabEdge.TOP]?.lightBackground ?: !dark
    val navigation = edges.pixels[tabNavigationEdge()]?.lightBackground ?: !dark
    SideEffect {
        val controller = WindowCompat.getInsetsController(window, window.decorView)
        controller.isAppearanceLightStatusBars = if (fullscreen) top else !dark
        controller.isAppearanceLightNavigationBars = if (webBottom) navigation else !dark
    }
}
