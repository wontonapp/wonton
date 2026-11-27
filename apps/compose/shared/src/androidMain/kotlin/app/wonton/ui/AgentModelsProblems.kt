package app.wonton.ui

import app.wonton.client.ClientCore
import app.wonton.protocol.AgentModelsProblem
import app.wonton.protocol.AgentModelsReason
import app.wonton.protocol.AgentModelsViewModel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

/** 把模型页的订阅包装成 [Flow]，取消和释放方式同 [chatUpdates]。 */
fun ClientCore.agentModelsUpdates(): Flow<AgentModelsViewModel> = flow {
    subscribeAgentModels().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}

/** 输入框的名字，和核心给的 [AgentModelsProblem.field] 对应。 */
internal const val MODEL_FIELD_KEY = "key"
internal const val MODEL_FIELD_BASE_URL = "base_url"

/** 这次失败是不是标在某个输入框上的：是的话不再占一条提示。 */
internal fun AgentModelsProblem.isFieldProblem(): Boolean =
    reason == AgentModelsReason.INVALID_INPUT && (field == MODEL_FIELD_KEY || field == MODEL_FIELD_BASE_URL)

/** 一句人话加能做什么，类别由核心给；核心的原文收进默认收起的“详情”（ADR-0129）。 */
internal val AgentModelsProblem.problem: Problem
    get() {
        val (title, next) = when (reason) {
            AgentModelsReason.TRANSPORT ->
                R.string.agent_models_problem_transport to R.string.agent_models_problem_transport_next
            AgentModelsReason.FORBIDDEN ->
                R.string.agent_models_problem_forbidden to R.string.agent_models_problem_forbidden_next
            AgentModelsReason.NOT_INSTALLED ->
                R.string.agent_models_problem_not_installed to R.string.agent_models_problem_not_installed_next
            AgentModelsReason.NOT_READY ->
                R.string.agent_models_problem_not_ready to R.string.agent_models_problem_not_ready_next
            AgentModelsReason.UNSUPPORTED ->
                R.string.agent_models_problem_unsupported to R.string.agent_models_problem_unsupported_next
            AgentModelsReason.NOT_FOUND ->
                R.string.agent_models_problem_not_found to R.string.agent_models_problem_not_found_next
            AgentModelsReason.INVALID_INPUT ->
                R.string.agent_models_problem_invalid_input to R.string.agent_models_problem_invalid_input_next
            AgentModelsReason.UPSTREAM ->
                R.string.agent_models_problem_upstream to R.string.agent_models_problem_upstream_next
            AgentModelsReason.PROTOCOL ->
                R.string.agent_models_problem_protocol to R.string.agent_models_problem_protocol_next
            AgentModelsReason.OTHER ->
                R.string.agent_models_problem_other to R.string.agent_models_problem_other_next
        }
        val details = listOfNotNull(message.takeIf { it.isNotBlank() }, detail?.takeIf { it.isNotBlank() })
        return Problem(
            titleRes = title,
            nextStepRes = next,
            detail = details.joinToString("\n").takeIf { it.isNotEmpty() },
        )
    }
