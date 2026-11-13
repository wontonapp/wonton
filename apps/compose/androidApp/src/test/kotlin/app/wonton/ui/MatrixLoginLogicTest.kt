package app.wonton.ui

import app.wonton.protocol.MatrixAccount
import app.wonton.protocol.MatrixLoginFailure
import app.wonton.protocol.MatrixLoginState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** 渲染决策的纯逻辑：浏览器回调的识别、失败类别到标题的映射、哪些状态算结束。 */
class MatrixLoginLogicTest {
    @Test
    fun oidcCallbackUrlsMatchExactly() {
        // 核心给的回调地址，浏览器会把它归一成 host 为空的写法。
        val redirect = "io.github.wontonapp:/matrix-login"
        assertTrue(isMatrixLoginCallback("io.github.wontonapp:/matrix-login", redirect))
        assertTrue(
            isMatrixLoginCallback("io.github.wontonapp:/matrix-login?code=private-code&state=private-state", redirect),
        )
        // 浏览器归一出来的 host 为空写法，算同一个地址。
        assertTrue(isMatrixLoginCallback("io.github.wontonapp:///matrix-login", redirect))
        assertTrue(
            isMatrixLoginCallback("io.github.wontonapp:///matrix-login?code=private-code&state=2", redirect),
        )
        // 前缀撞上的地址不是这次登录的回调，别把别的链接吞掉。
        assertFalse(isMatrixLoginCallback("io.github.wontonapp:/matrix-loginx", redirect))
        assertFalse(isMatrixLoginCallback("io.github.wontonapp:/matrix-login/extra", redirect))
        assertFalse(isMatrixLoginCallback("io.github.wontonapp:///matrix-loginx", redirect))
        assertFalse(isMatrixLoginCallback("io.github.wontonapp:/matrix-other", redirect))
        assertFalse(isMatrixLoginCallback("wonton://settings/peers", redirect))
        assertFalse(isMatrixLoginCallback("https://example.org/io.github.wontonapp:/matrix-login", redirect))
        // host 不为空的地址是别的 App 的回调，不算。
        assertFalse(isMatrixLoginCallback("io.github.wontonapp://evil.example/matrix-login", redirect))
    }

    @Test
    fun everyFailureCategoryHasItsOwnTitle() {
        val titles = listOf(
            MatrixLoginFailure.INVALID_HOMESERVER,
            MatrixLoginFailure.DISCOVERY,
            MatrixLoginFailure.UNSUPPORTED_METHOD,
            MatrixLoginFailure.TRANSPORT,
            MatrixLoginFailure.INVALID_CREDENTIALS,
            MatrixLoginFailure.OIDC_REJECTED,
            MatrixLoginFailure.INVALID_CALLBACK,
            MatrixLoginFailure.EXPIRED,
            MatrixLoginFailure.STORAGE,
            MatrixLoginFailure.UPSTREAM,
            MatrixLoginFailure.OTHER,
        ).map { failureTitle(it) }
        // 每个类别都有自己的标题，通用文案只留给不认识的类别。
        assertEquals(titles.size, titles.toSet().size)
    }

    @Test
    fun onlyFinishedStatesAreTerminal() {
        assertTrue(
            MatrixLoginState.Succeeded(
                "slot-1",
                MatrixAccount("@alice:matrix.org", "https://matrix-client.matrix.org/"),
            ).isTerminal,
        )
        assertTrue(MatrixLoginState.Cancelled.isTerminal)
        assertFalse(MatrixLoginState.Discovering.isTerminal)
        assertFalse(MatrixLoginState.Ready.isTerminal)
        assertFalse(MatrixLoginState.Authenticating.isTerminal)
        assertFalse(MatrixLoginState.Failed(MatrixLoginFailure.INVALID_CREDENTIALS).isTerminal)
        assertFalse(MatrixLoginState.Other.isTerminal)
    }
}
