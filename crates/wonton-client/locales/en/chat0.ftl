chat-sign-in-no-connection-info = There's no connection information for your home server yet.

chat-sign-in-no-saved-session = No session to save after signing in.

# Login-token request details. $reason is an unchanged upstream error or a localized timeout detail.
# $reason is the proxy URL parser's original error.
chat-login-invalid-proxy = The local proxy address isn't valid: { $reason }

chat-login-request-failed = Can't get a Matrix login token from your home server: { $reason }

# $message and $kind are the server's original error text and error code.
chat-login-server-error = { $message } ({ $kind })

# Recovery details.
chat-recovery-not-connected = You're not connected to your home server yet.

chat-recovery-reset-no-response = Your home server didn't respond. Your backup is unchanged; try again later.

chat-recovery-empty-key = Your recovery key can't be empty.

# Approval request (confirmation card) details, shown in the problem detail under the card
# when an answer or a stop on the time window didn't go through.
# $message is the home server's original error text, or a localized detail.
chat-confirmation-answer-failed = The answer didn't go through: { $message }

# $error is the HTTP client's original error.
chat-confirmation-answer-lost = No response from your home server, so it isn't clear whether the answer went through. You can try again. ({ $error })

# $message is the home server's original error text, or a localized detail.
chat-confirmation-stop-failed = Stopping didn't take effect: { $message }

# $error is the JSON parser's original error.
chat-confirmation-response-unreadable = The response from your home server couldn't be read ({ $error })

# $status is the unchanged HTTP status code.
chat-confirmation-core-status = Your home server returned HTTP { $status }

chat-confirmations-list-failed = Couldn't load the list of confirmations: { $message }

# $reason is the Matrix ID parser's original error.
chat-contacts-invalid-user-id = That user ID isn't valid: { $reason }

chat-layout-invalid-bot-id = That Matrix ID isn't formatted correctly, like @name:server.

chat-layout-change-failed = Couldn't change the chat layout: { $reason }

# Shown when the account is marked failed because Matrix revoked this device.
chat-session-device-revoked = This device was revoked on Matrix. Pair it with your home server again.

# Shown when starting a direct chat with a user ID that isn't @name:server.
chat-invalid-user-id = That user ID isn't valid. It should look like @name:server.

# $file_type is the picked file's unchanged MIME type.
chat-not-image-type = That file isn't an image: { $file_type }

# Shown when answering or stopping a confirmation before the connection is up.
chat-action-not-connected = You're not connected to your home server yet.

# $error is the room ID parser's original error.
chat-invalid-room-id = The room ID isn't valid: { $error }

# $room is the unchanged room ID.
chat-room-not-subscribed = Subscribe to the room's timeline first: { $room }

# Shown when asking the default agent before one is chosen.
chat-no-default-agent = There's no Default Agent yet. Choose one in Settings.

chat-op-display-name-failed = Can't sync your display name: { $error }

chat-op-resend-failed = Can't resend the message: { $error }

chat-op-send-image-failed = Can't send the image: { $error }

chat-op-forward-failed = Can't forward the message: { $error }

chat-op-redact-failed = Can't remove the message: { $error }

chat-avatar-cache-not-file = The avatar cache path isn't a file.

chat-avatar-cache-missing-mxc = An avatar in the cache has no MXC address.

chat-avatar-cache-path-not-utf8 = The avatar cache path isn't valid UTF-8.

# Chat local storage (crates/wonton-client/src/chat/storage.rs).
# $path is the file's path, $reason is the original parser or I/O error.
chat-storage-session-corrupt = The chat sign-in state file { $path } is damaged: { $reason }

chat-storage-io-failed = Can't read or write { $path }: { $reason }

# Chat transport problems (crates/wonton-client/src/chat/view.rs): the reason
# of a CoreError::TransportUnavailable and the message of a failed chat action.
chat-not-connected = Try again once you're connected to your home server.

chat-draft-save-failed = Saving the draft failed: { $error }

chat-draft-not-signed-in = Not signed in to Matrix yet

# Line under an approval card about the approval window, when it has ended. $count is how many
# calls the window let through after the first one; 0 means unknown or none.
chat-window-ended =
    { $reason ->
        [expired]
            { $count ->
                [0] The window has expired.
               *[other] The window has expired; { $count ->
                    [one] 1 further action was approved.
                   *[other] { $count } further actions were approved.
                }
            }
        [stopped]
            { $count ->
                [0] The window was stopped.
               *[other] The window was stopped; { $count ->
                    [one] 1 further action was approved.
                   *[other] { $count } further actions were approved.
                }
            }
        [restarted] The home server restarted; the window has ended.
       *[other]
            { $count ->
                [0] The window has ended.
               *[other] The window has ended; { $count ->
                    [one] 1 further action was approved.
                   *[other] { $count } further actions were approved.
                }
            }
    }
