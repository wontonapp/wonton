## Texts the bridge posts into rooms by itself (ADR-0043). One message is one full sentence;
## data goes in as parameters, untranslated. English is the base language, in sentence case.

login-reconnect = Signed in. Reconnecting.
input-unsupported = Unsupported input: only plain text is accepted.
input-too-long = Invalid input: the text is over the { $max }-byte limit
input-empty = Invalid input: the text can't be empty
control-without-text = Invalid input: the control command has no text
interrupt-pending = Busy: an interrupt is already waiting.
queue-full = Busy: the ACP queue is full; try again later.
no-active-session = The agent isn't ready: there's no active ACP session.
steer-rejected = The agent didn't take the steering request.
session-lost-new = The ACP session was lost and a new one was created; the earlier context is gone.
permission-failed = The agent's permission request failed: { $message }
permissions-cancelled = The permission request was cancelled; the agent didn't run that step.
output-limited = Upstream error: the agent's reply passed the output limit; the turn was cancelled.
no-text = The agent produced no text this turn (stopReason={ $stop_reason }).
transport-disconnected = Transport unavailable: the ACP agent connection dropped; it reconnects shortly.

catch-up-dropped =
    I was offline; the {$count} below were not processed:
    {$list}
catch-up-dropped-more =
    I was offline; the {$count} below were not processed:
    {$list}
    …and {$rest} more are not listed

# Only shown in logs (logs stay untranslated; these read with the detail prefix).
agent-connect-socket = Can't connect to the ACP socket { $path }: { $error }
agent-init-timeout = The ACP initialize timed out.
direct-task-failed = Couldn't run the direct chat task: { $error }
stream-closed = The Matrix room message stream closed.
