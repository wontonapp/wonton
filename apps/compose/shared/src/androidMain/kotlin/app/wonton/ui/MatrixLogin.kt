package app.wonton.ui

import androidx.annotation.StringRes
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.MatrixLoginFailure
import app.wonton.protocol.MatrixLoginState
import app.wonton.protocol.MatrixOidcCallback

/** 登录起手时填的服务器；核心的发现也是这个默认值（ADR-0051）。 */
internal const val DEFAULT_MATRIX_SERVER = "matrix.org"

/** Matrix 登录的 OIDC 回调地址（MSC2965/2966：scheme 是 client_uri `https://wontonapp.github.io` 的反写）。
 *  核心的 `REDIRECT_URIS` 必须收这个值，浏览器完成登录带授权码回到这里（MainActivity 的深链处理）。 */
const val MATRIX_LOGIN_REDIRECT_URI = "io.github.wontonapp:/matrix-login"

/** 浏览器归一出来的 host 为空的写法，和 [MATRIX_LOGIN_REDIRECT_URI] 是同一个地址。 */
const val MATRIX_LOGIN_REDIRECT_URI_HOST_EMPTY = "io.github.wontonapp:///matrix-login"

/** 浏览器回来的地址是不是这次登录的回调。浏览器会把 `io.github.wontonapp:/matrix-login` 归一成
 *  host 为空的 `io.github.wontonapp:///matrix-login`，两种算同一个地址；前缀撞上的（如 `…-loginx`）不算。 */
fun isMatrixLoginCallback(url: String): Boolean =
    url == MATRIX_LOGIN_REDIRECT_URI || url == MATRIX_LOGIN_REDIRECT_URI_HOST_EMPTY ||
        url.startsWith("$MATRIX_LOGIN_REDIRECT_URI?") ||
        url.startsWith("$MATRIX_LOGIN_REDIRECT_URI_HOST_EMPTY?")

/**
 * 浏览器回来的回调交给核心续上流程。没有进行中的登录（已结束或本来没有）就不是这次流程的回调，
 * 原样丢弃：迟到的回调不能恢复已取消的流程。返回是否按回调处理了。
 */
fun ClientCore.completeMatrixOidcCallback(callbackUrl: String): Boolean {
    if (!isMatrixLoginCallback(callbackUrl)) return false
    val loginId = servers().matrixLogin?.loginId
    if (loginId != null) {
        send(Action.CompleteMatrixOidc(MatrixOidcCallback(loginId, callbackUrl)))
    }
    return true
}

/** 登录已结束：成功切走槽位、取消不改动槽位，两种都回到原来的界面。 */
val MatrixLoginState.isTerminal: Boolean
    get() = this is MatrixLoginState.Succeeded || this is MatrixLoginState.Cancelled

/** 失败类别的一句话标题；不认识的类别落通用文案。 */
@StringRes
fun failureTitle(reason: MatrixLoginFailure): Int = when (reason) {
    MatrixLoginFailure.INVALID_HOMESERVER -> R.string.matrix_login_failed_invalid_homeserver
    MatrixLoginFailure.DISCOVERY -> R.string.matrix_login_failed_discovery
    MatrixLoginFailure.UNSUPPORTED_METHOD -> R.string.matrix_login_failed_unsupported_method
    MatrixLoginFailure.TRANSPORT -> R.string.matrix_login_failed_transport
    MatrixLoginFailure.INVALID_CREDENTIALS -> R.string.matrix_login_failed_invalid_credentials
    MatrixLoginFailure.OIDC_REJECTED -> R.string.matrix_login_failed_oidc_rejected
    MatrixLoginFailure.INVALID_CALLBACK -> R.string.matrix_login_failed_invalid_callback
    MatrixLoginFailure.EXPIRED -> R.string.matrix_login_failed_expired
    MatrixLoginFailure.STORAGE -> R.string.matrix_login_failed_storage
    MatrixLoginFailure.UPSTREAM -> R.string.matrix_login_failed_upstream
    MatrixLoginFailure.OTHER -> R.string.matrix_login_failed_other
}
