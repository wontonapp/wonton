package app.wonton.android.push

import android.content.Context
import android.content.Intent
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import app.wonton.android.IncomingRequests
import app.wonton.android.R
import app.wonton.android.WontonApplication
import app.wonton.protocol.Answer
import app.wonton.protocol.CoreException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout

/** No retry: a restarted attempt reports an uncertain result rather than sending the user action twice. */
class PushActionWorker(context: Context, parameters: WorkerParameters) : CoroutineWorker(context, parameters) {
    override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
        val target = Intent().apply {
            inputData.keyValueMap.forEach { (key, value) ->
                when (value) {
                    is String -> putExtra(key, value)
                    is Boolean -> putExtra(key, value)
                }
            }
            action = inputData.getString(PushActionReceiver.EXTRA_ACTION)
        }
        val context = applicationContext
        if (runAttemptCount > 0) {
            NotificationActions.failure(context, target, context.getString(R.string.push_action_interrupted))
            return@withContext Result.failure()
        }
        if (!valid(target)) {
            NotificationActions.failure(context, target, context.getString(R.string.push_action_unavailable))
            return@withContext Result.failure()
        }
        try {
            NotificationActions.progress(context, target)
            withTimeout(ACTION_TIMEOUT_MS) {
                val core = (context as WontonApplication).core.getOrThrow()
                val server = target.getStringExtra(IncomingRequests.EXTRA_SERVER)
                when (target.action) {
                    PushActionReceiver.ACTION_REPLY -> core.replyToPush(
                        server, target.getStringExtra(IncomingRequests.EXTRA_ROOM).orEmpty(),
                        target.getStringExtra(PushActionReceiver.EXTRA_THREAD),
                        target.getStringExtra(PushActionReceiver.EXTRA_EVENT),
                        target.getStringExtra(PushActionReceiver.EXTRA_REPLY).orEmpty(),
                    )
                    PushActionReceiver.ACTION_ANSWER -> core.answerPushConfirmation(
                        server, target.getStringExtra(PushBuilder.EXTRA_CARD).orEmpty(),
                        Answer.entries.firstOrNull {
                            it.name == target.getStringExtra(PushActionReceiver.EXTRA_ANSWER)
                        } ?: Answer.UNKNOWN,
                        target.getStringExtra(PushActionReceiver.EXTRA_CHOICE_ID),
                    )
                    else -> error("Unsupported notification action")
                }
            }
            NotificationActions.accepted(context, target)
            Result.success()
        } catch (_: TimeoutCancellationException) {
            fail(target, R.string.push_action_timeout)
        } catch (failure: CoreException) {
            NotificationActions.failure(context, target,
                failure.message?.takeIf { it.isNotBlank() } ?: context.getString(R.string.push_action_unavailable))
            Result.failure()
        } catch (cancelled: CancellationException) {
            fail(target, R.string.push_action_interrupted)
            throw cancelled
        } catch (_: Exception) {
            fail(target, R.string.push_action_unavailable)
        }
    }

    private fun fail(target: Intent, reason: Int): Result {
        NotificationActions.failure(applicationContext, target, applicationContext.getString(reason))
        return Result.failure()
    }

    private fun valid(target: Intent): Boolean {
        if (target.getStringExtra(PushActionReceiver.EXTRA_TAG).isNullOrBlank()) return false
        return when (target.action) {
            PushActionReceiver.ACTION_REPLY ->
                !target.getStringExtra(IncomingRequests.EXTRA_ROOM).isNullOrBlank() &&
                    !target.getStringExtra(PushActionReceiver.EXTRA_EVENT).isNullOrBlank() &&
                    !target.getStringExtra(PushActionReceiver.EXTRA_REPLY).isNullOrBlank()
            PushActionReceiver.ACTION_ANSWER -> !target.getStringExtra(PushBuilder.EXTRA_CARD).isNullOrBlank()
            else -> false
        }
    }

    private companion object {
        const val ACTION_TIMEOUT_MS = 30_000L
    }
}
