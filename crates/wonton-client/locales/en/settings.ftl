# The push method setting (crates/wonton-client/src/push_method.rs).
# $path is the settings file's path, $reason is the original I/O error.
push-method-write-failed = Can't write the push method settings file { $path }: { $reason }

# Push registration (crates/wonton-client/src/push/{mod,registration,notification}.rs):
# details of refused endpoints, failed registrations and notifications that
# couldn't show the message. The notification bodies and placeholders
# (unknown sender, invitation, [file]…) are not here yet.
# $reason is the original error detail.
push-matrix-pusher-failed = Can't register the pusher on Matrix: { $reason }

# $reason is the URL parser's original error.
push-endpoint-invalid-url = The push endpoint isn't a valid URL: { $reason }

push-endpoint-scheme = The push endpoint must be an http or https address.

push-endpoint-p256dh = The p256dh key must be a 65-byte uncompressed P-256 public key (base64url).

push-endpoint-auth = The auth secret must be 16 bytes (base64url).

# $reason is an unchanged upstream error or a localized timeout detail.
push-register-failed = Can't register for notifications with your home server: { $reason }

push-unregister-failed = Can't unregister notifications with your home server: { $reason }

# $reason is the JSON parser's original error.
push-register-invalid-response = Your home server returned the registration result in an unexpected format: { $reason }

# $reason is the local proxy's or HTTP client's original error.
push-invalid-proxy = The local proxy address isn't valid: { $reason }

push-http-client = Can't create an HTTP client: { $reason }

# $seconds is the request timeout in seconds.
push-timeout = { $seconds ->
    [one] No response within { $seconds } second
   *[other] No response within { $seconds } seconds
}

# $status is the unchanged HTTP status code.
push-server-status = Your home server returned HTTP { $status }

# $message and $kind are the server's original error text and error code.
push-server-error = { $message } ({ $kind })

# $reason is the JSON parser's original error.
push-payload-invalid = The push payload isn't in the expected format: { $reason }

# $reason is the ID parser's original error.
push-invalid-room-id = The room ID in the push isn't valid: { $reason }

push-invalid-event-id = The event ID in the push isn't valid: { $reason }

# $seconds is how long fetching waited.
push-fetch-timeout = { $seconds ->
    [one] Couldn't fetch this message within { $seconds } second.
   *[other] Couldn't fetch this message within { $seconds } seconds.
}

# $reason is the Matrix SDK's original error.
push-fetch-failed = Can't get this message: { $reason }

push-core-closed = The client core has shut down.

push-not-paired = You haven't connected to your home server yet.

push-home-unreachable = Can't reach your home server: { $reason }

# $seconds is how long signing in waited.
push-session-timeout = { $seconds ->
    [one] Couldn't sign in to Matrix within { $seconds } second.
   *[other] Couldn't sign in to Matrix within { $seconds } seconds.
}

push-room-key-missing = The room key for this message hasn't arrived yet.
