# Relay table settings (ADR-0048): why the table can't be read or saved right now.
relays-not-paired = You haven't paired with your home server yet.

relays-not-connected = You're not connected to your home server yet.

# The relay table is being saved; edits wait for it to finish.
relays-busy = The relay table is being saved; try again in a moment.
relays-not-loaded = The relay table hasn't been loaded yet; refresh it first.

# The table comes from the WONTON_RELAYS environment variable and is read-only.
relays-locked = This relay table is fixed by an environment variable and can't be changed here.

# $reason is the proxy URL parser's original error.
relays-invalid-proxy = The local proxy address isn't valid: { $reason }

# $reason is the HTTP client's original error.
relays-http-client = Can't create an HTTP client: { $reason }

# $seconds is the request timeout in seconds.
relays-timeout = No response within { $seconds } seconds

# $reason is an unchanged upstream error or a localized timeout/protocol detail.
relays-request-failed = Can't reach your home server: { $reason }

# $reason is the JSON parser's original error.
relays-invalid-response = Your home server returned a relay table in an unexpected format: { $reason }

# $status is the unchanged HTTP status code.
relays-server-status = Your home server returned HTTP { $status }

# $message and $kind are the server's original error text and error code.
relays-server-error = { $message } ({ $kind })

# $value is the address as typed.
relays-invalid-url = The relay address isn't a valid URL: { $value }

# $value is the address as typed.
relays-needs-https = Relay addresses must be https: { $value }

# $value is the address as typed.
relays-has-userinfo = The relay address can't carry a username or password; enter the token separately: { $value }

# $value is the normalized address.
relays-duplicate = That relay is already in the table: { $value }

# $value is the normalized address.
relays-missing = That relay is no longer in the table: { $value }

# $value is the normalized address of the entry to insert before.
relays-before-missing = The relay to insert before is no longer in the table: { $value }

# $reason is the connection-info parser's original error.
relays-invalid-ticket = Saved connection information is damaged: { $reason }
