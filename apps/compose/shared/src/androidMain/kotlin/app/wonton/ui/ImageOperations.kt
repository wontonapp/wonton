package app.wonton.ui

import app.wonton.protocol.CoreException

internal fun imageProblem(failure: Exception, title: Int): Problem = when (failure) {
    is CoreException -> failure.problem
    is SecurityException -> Problem(
        titleRes = title,
        nextStepRes = R.string.message_image_permission_denied,
        detail = failure.message,
    )
    else -> Problem(titleRes = title, detail = failure.message)
}
