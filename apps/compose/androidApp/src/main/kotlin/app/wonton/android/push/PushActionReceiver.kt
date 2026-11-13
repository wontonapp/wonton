package app.wonton.android.push

import android.app.RemoteInput
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import app.wonton.android.R
import androidx.work.Data
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/** The broadcast only persists work; WorkManager owns execution and its wake lock after it returns. */
class PushActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val app = context.applicationContext
        val data = Data.Builder().apply {
            intent.extras?.keySet()?.forEach { key ->
                when (val value = intent.extras?.get(key)) {
                    is String -> putString(key, value)
                    is Boolean -> putBoolean(key, value)
                }
            }
            putString(EXTRA_ACTION, intent.action)
            if (intent.action == ACTION_REPLY) {
                putString(EXTRA_REPLY, RemoteInput.getResultsFromIntent(intent)
                    ?.getCharSequence(EXTRA_REPLY)?.toString())
            }
        }
        val pending = goAsync()
        scope.launch {
            try {
                val input = data.build()
                val request = OneTimeWorkRequestBuilder<PushActionWorker>().setInputData(input).build()
                // KEEP suppresses concurrent taps, not a later explicit attempt after a visible failure.
                WorkManager.getInstance(app).enqueueUniqueWork(
                    "push-action:${intent.getStringExtra(EXTRA_TAG)}",
                    ExistingWorkPolicy.KEEP, request,
                ).result.get(ENQUEUE_TIMEOUT_SECONDS, TimeUnit.SECONDS)
            } catch (_: Exception) {
                NotificationActions.failure(app, intent, app.getString(R.string.push_action_unavailable))
            } finally {
                pending.finish()
            }
        }
    }

    companion object {
        const val ACTION_REPLY = "app.wonton.android.push.REPLY"
        const val ACTION_ANSWER = "app.wonton.android.push.ANSWER"
        const val EXTRA_ACTION = "wonton_action"
        const val EXTRA_REPLY = "wonton_reply"
        const val EXTRA_ANSWER = "wonton_answer"
        const val EXTRA_CHOICE_ID = "wonton_choice"
        const val EXTRA_THREAD = "wonton_thread"
        const val EXTRA_EVENT = "wonton_event"
        const val EXTRA_TAG = "wonton_tag"
        const val EXTRA_CHANNEL = "wonton_channel"
        const val EXTRA_TITLE = "wonton_title"
        const val EXTRA_PRIVATE = "wonton_private"

        private const val ENQUEUE_TIMEOUT_SECONDS = 5L
        private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    }
}
