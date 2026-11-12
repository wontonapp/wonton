package app.wonton.ui

import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext

@Composable
internal fun appVersion(): String {
    val context = LocalContext.current
    return remember(context) { context.installedVersion() }
}

@Suppress("DEPRECATION")
private fun Context.installedVersion(): String {
    val info = packageManager.getPackageInfo(packageName, 0)
    return "${checkNotNull(info.versionName)} (${info.longVersionCode})"
}
