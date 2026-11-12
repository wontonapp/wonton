package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.FriendFailure
import app.wonton.protocol.FriendProblem
import app.wonton.protocol.ShareFailure
import app.wonton.protocol.ShareIdentityFailure
import app.wonton.protocol.ShareProblem

@Composable
internal fun ShareProblem.asProblem(): Problem {
    val title = when (reason) {
        ShareFailure.UNREACHABLE -> R.string.share_problem_unreachable
        ShareFailure.IDENTITY -> R.string.share_problem_identity
        ShareFailure.REQUIRED -> R.string.share_problem_required
        ShareFailure.REVOKED -> R.string.share_problem_revoked
        ShareFailure.EXPIRED -> R.string.share_problem_expired
        ShareFailure.PLUGIN_NOT_RUNNING -> R.string.share_problem_plugin
        ShareFailure.OTHER -> R.string.share_problem_other
    }
    val identityText = identity?.let {
        stringResource(when (it) {
            ShareIdentityFailure.INVALID_TOKEN -> R.string.share_identity_invalid
            ShareIdentityFailure.DISCOVERY_FAILED -> R.string.share_identity_discovery
            ShareIdentityFailure.USER_INFO_FAILED -> R.string.share_identity_userinfo
            ShareIdentityFailure.SERVER_NAME_MISMATCH -> R.string.share_identity_mismatch
            ShareIdentityFailure.OTHER -> R.string.share_identity_other
        })
    }
    val detail = listOfNotNull(identityText, httpStatus?.let { "HTTP $it" }).joinToString("\n").ifEmpty { null }
    return Problem(titleRes = title, detail = detail)
}

internal fun friendShareProblem(problem: FriendProblem): Problem = Problem(
    titleRes = when (problem.reason) {
        FriendFailure.TRANSPORT_UNAVAILABLE -> R.string.problem_transport_unavailable_title
        FriendFailure.UNAUTHORIZED -> R.string.problem_unauthorized_title
        FriendFailure.INVALID_USER_ID -> R.string.problem_invalid_input_title
        else -> R.string.share_problem_other
    },
    detail = listOfNotNull(problem.httpStatus?.let { "HTTP $it" }, problem.matrixErrcode,
        problem.retryAfterMs?.toString()).joinToString("\n").ifEmpty { null },
)
