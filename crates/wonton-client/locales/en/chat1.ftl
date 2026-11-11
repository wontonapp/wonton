## Fragments for crates/wonton-client/src/chat/{login,sign_in,recovery}.rs (chat-a).
## These are the collapsed technical details of a chat problem or error:
## the CoreError reason and the ChatProblem message shown under "Details".
# Sign-in details. $reason is the Matrix SDK's original error.
chat-sign-in-restore-failed = Can't restore your Matrix session: { $reason }

chat-sign-in-login-failed = Can't sign in to Matrix: { $reason }

# $reason is the HTTP client's original error.
chat-login-http-client = Can't create an HTTP client: { $reason }

# $seconds is the request timeout in seconds.
chat-login-timeout = { $seconds ->
    [one] No response within { $seconds } second
   *[other] No response within { $seconds } seconds
}

# $reason is the HTTP body reader's original error.
chat-login-response-failed = Can't read your home server's response: { $reason }

# $reason is the JSON parser's original error.
chat-login-invalid-token = Your home server returned a login token in an unexpected format: { $reason }

# $login_type is the login type the home server returned; $expected is the required one.
chat-login-wrong-type = Your home server returned the login type { $login_type } instead of { $expected }

# $status is the unchanged HTTP status code.
chat-login-server-status = Your home server returned HTTP { $status }

# $reason is the original storage error.
chat-recovery-key-not-saved = Your recovery key couldn't be saved on this device. Write it down now: { $reason }

# $error is the HTTP client's original error.
chat-confirmation-stop-lost = No response from your home server. You can try again. ({ $error })

# Shown when starting a direct chat with an empty username.
chat-contacts-empty-username = Enter a username.

# Shown when sending a message through Siri, Shortcuts or the chat screen with empty text.
chat-empty-message = Type a message first.

# Shown when the bot ID handed in from Siri or Shortcuts doesn't parse.
chat-ask-invalid-bot-id = The default agent's ID isn't formatted correctly.

# Shown when a saved direct chat's room ID doesn't parse.
chat-ask-invalid-room-id = The direct chat's room ID isn't valid.

# Shown when the shell routes a chat action this core doesn't know.
chat-invalid-action = Not a chat action.

# Shown when the shell sends an answer this core doesn't know.
chat-unknown-answer = This version of Wonton doesn't know that answer.

# Shown when a chat action arrives before Matrix sign-in.
chat-not-signed-in = You're not signed in to Matrix yet.

chat-send-no-response = The Matrix server stopped responding

# Shown when sending into a room that only carries notifications.
chat-read-only-room = This room only carries notifications, so you can't send messages here.

# $error is the event ID parser's original error.
chat-invalid-event-id = The message's event ID isn't valid: { $error }

# Shown when the message to forward or remove doesn't offer that action (already removed, not sent yet, no permission, …).
chat-message-action-unavailable = You can't do that with this message.

# Shown when the room to forward to doesn't exist or hasn't been joined.
chat-forward-target-not-joined = You haven't joined the room you're forwarding to.

# Chat operations that failed in the background. Shown as the problem message;
# $error is the Matrix SDK's or the home server's original error.
chat-op-ask-agent-failed = Can't ask the Default Agent: { $error }

chat-op-join-failed = Can't join the room: { $error }

chat-op-leave-failed = Can't leave the room: { $error }

chat-op-cancel-send-failed = Can't cancel sending: { $error }

chat-op-paginate-failed = Can't load older messages: { $error }

chat-op-mark-read-failed = Can't mark the room as read: { $error }

chat-op-direct-chat-failed = Can't start the direct chat: { $error }

chat-op-send-text-failed = Can't send the message: { $error }

chat-avatar-cache-hash-collision = Two avatars in the cache have the same hash.

chat-draft-clear-failed = Clearing the sent draft failed: { $error }

chat-draft-read-interrupted = Loading the draft was interrupted
