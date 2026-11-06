package app.wonton.ui.design

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.material3.Scaffold
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.graphics.Color

@Composable
fun WontonEdgeToEdgeScaffold(
    containerColor: Color,
    contentWindowInsets: WindowInsets,
    bottomBar: @Composable () -> Unit,
    content: @Composable (PaddingValues) -> Unit,
) {
    val navigation = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    val chrome: @Composable (PaddingValues) -> Unit = { insets ->
        CompositionLocalProvider(LocalWontonBottomInset provides maxOf(insets.calculateBottomPadding(), navigation)) {
            content(insets)
        }
    }
    Scaffold(
        containerColor = containerColor,
        contentWindowInsets = contentWindowInsets,
        bottomBar = bottomBar,
        content = chrome,
    )
}
