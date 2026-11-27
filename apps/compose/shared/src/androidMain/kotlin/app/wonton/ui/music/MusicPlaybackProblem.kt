package app.wonton.ui.music

import app.wonton.protocol.MediaFailure
import app.wonton.protocol.MediaFailureCode
import app.wonton.ui.Problem
import app.wonton.ui.R

/** 放歌没成：类别由核心给，文字按类别写在这里，原话收进详情。 */
internal val MediaFailure.problem: Problem
    get() {
        val detail = message.takeIf { it.isNotEmpty() }
        val (title, nextStep) = when (code) {
            MediaFailureCode.TransportUnavailable ->
                R.string.problem_transport_unavailable_title to R.string.problem_transport_unavailable_next_step
            MediaFailureCode.PluginUnavailable ->
                R.string.music_problem_not_ready_title to R.string.music_problem_not_ready_next_step
            MediaFailureCode.AuthenticationRequired ->
                R.string.music_problem_not_signed_in_title to R.string.music_problem_not_signed_in_next_step
            MediaFailureCode.PermissionDenied ->
                R.string.music_problem_forbidden_title to R.string.music_problem_forbidden_next_step
            MediaFailureCode.MediaGone ->
                R.string.music_problem_not_found_title to R.string.music_problem_not_found_next_step
            MediaFailureCode.UnsupportedByDevice ->
                R.string.music_playback_unsupported_title to R.string.music_playback_unsupported_next_step
            MediaFailureCode.UpstreamError, MediaFailureCode.RateLimited ->
                R.string.music_problem_upstream_title to R.string.problem_upstream_next_step
            MediaFailureCode.Protocol ->
                R.string.music_problem_protocol_title to R.string.problem_plugin_protocol_next_step
            else -> R.string.music_playback_failed_title to R.string.music_playback_failed_next_step
        }
        return Problem(titleRes = title, nextStepRes = nextStep, detail = detail)
    }
