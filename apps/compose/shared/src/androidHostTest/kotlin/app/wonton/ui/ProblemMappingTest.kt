package app.wonton.ui

import app.wonton.protocol.ChatFailure
import app.wonton.protocol.CoreException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * 聊天失败的原文是技术细节，收进默认收起的详情；正文只留壳里写好的一句话（ADR-0109 §7）。
 * 核心异常沿用核心给的下一步，原文就放在下一步里。
 */
class ProblemMappingTest {
    @Test
    fun chatFailuresKeepTheCoreTextInDetails() {
        for (failure in listOf(ChatFailure.NOT_READY, ChatFailure.INVALID_INPUT)) {
            val problem = failure.problem(CORE_TEXT)
            assertEquals(CORE_TEXT, problem.detail)
            assertNotNull("没有下一步", problem.nextStepRes)
            assertNull(problem.nextStep)
        }
    }

    @Test
    fun coreExceptionsPutTheCoreTextInTheNextStep() {
        for (failure in listOf(CoreException.NotReady(CORE_TEXT), CoreException.InvalidInput(CORE_TEXT))) {
            val problem = failure.coreProblem(CORE_TEXT)
            assertNull(problem.detail)
            assertNull(problem.nextStepRes)
            assertEquals(CORE_TEXT, problem.nextStep)
        }
    }

    private companion object {
        const val CORE_TEXT = "reply could not be built: {error}"
    }
}
