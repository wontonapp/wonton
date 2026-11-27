package app.wonton.ui

import app.wonton.protocol.ClientsFailure
import app.wonton.protocol.ClientsProblem

fun ClientsProblem.problem(): Problem = when (kind) {
    ClientsFailure.UNAUTHENTICATED -> Problem(
        titleRes = R.string.clients_domain_problem_unauthenticated,
        nextStepRes = R.string.clients_domain_problem_unauthenticated_next,
        detail = message,
    )
    ClientsFailure.INVALID_INPUT ->
        Problem(titleRes = R.string.clients_domain_problem_invalid, detail = message)
    ClientsFailure.PLUGIN_UNAVAILABLE,
    ClientsFailure.SETUP_PENDING,
    ClientsFailure.NOT_REGISTERED,
    ClientsFailure.CORE_NOT_IN_CONTAINER -> Problem(
        titleRes = R.string.clients_domain_problem_not_ready,
        nextStepRes = R.string.clients_domain_problem_not_ready_next,
        detail = message,
    )
    ClientsFailure.CREDENTIALS_FAILED,
    ClientsFailure.INGRESS_UNAVAILABLE -> Problem(
        titleRes = R.string.clients_domain_problem_upstream,
        nextStepRes = R.string.clients_domain_problem_upstream_next,
        detail = message,
    )
    ClientsFailure.LAN_UNAVAILABLE -> Problem(
        titleRes = R.string.clients_domain_problem_lan,
        nextStepRes = R.string.clients_domain_problem_lan_next,
        detail = message,
    )
    else -> Problem(titleRes = R.string.clients_domain_problem_failed, detail = message)
}
