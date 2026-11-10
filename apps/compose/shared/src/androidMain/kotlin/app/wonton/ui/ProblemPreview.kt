package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview

/** 整页都显示不出来时的样子（错误页）；文案就是核心给的那三类：发生了什么、能做什么、详情。 */
@Preview(name = "错误页", locale = "en", widthDp = 390, heightDp = 420)
@Composable
internal fun ProblemViewPreview() {
    PreviewFrame {
        ProblemView(
            problem = Problem(
                title = "Can't reach your home server",
                nextStep = "Check the network and try again.",
                detail = "error sending request: connection reset by peer",
            ),
            onRetry = {},
        )
    }
}
