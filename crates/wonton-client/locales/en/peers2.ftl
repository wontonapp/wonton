# $seconds is the request timeout in seconds.
peers-timeout = { $seconds ->
    [one] No response within { $seconds } second
   *[other] No response within { $seconds } seconds
}

# $status is the unchanged HTTP status code.
peers-server-status = Your home server returned HTTP { $status }

# $reason is the tunnel's original refusal text.
peers-rejected = Your home server rejected this device: { $reason }
