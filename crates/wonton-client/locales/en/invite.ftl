# Add Device details, before a pairing code can be requested.
invite-not-paired = You haven't paired with your home server yet.

# $reason is the connection-info parser's original error.
invite-invalid-ticket = Saved connection information is damaged: { $reason }

invite-not-connected = You're not connected to your home server yet.

# $reason is the proxy URL parser's original error.
invite-invalid-proxy = The local proxy address isn't valid: { $reason }

# $reason is the HTTP client's original error.
invite-http-client = Can't create an HTTP client: { $reason }

# $seconds is the request timeout in seconds.
invite-timeout = { $seconds ->
    [one] No response within { $seconds } second
   *[other] No response within { $seconds } seconds
}

# $reason is an unchanged upstream error or a localized timeout/protocol detail.
invite-request-failed = Can't get a pairing code from your home server: { $reason }

# $reason is the JSON parser's original error.
invite-invalid-response = Your home server returned a pairing code in an unexpected format: { $reason }

# $status is the unchanged HTTP status code.
invite-server-status = Your home server returned HTTP { $status }

# $message and $kind are the server's original error text and error code.
invite-server-error = { $message } ({ $kind })
