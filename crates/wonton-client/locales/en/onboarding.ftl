# $error is the JSON parser's original error.
home-ticket-invalid-json = The connection info isn't valid JSON: { $error }

# No parameters. The scanned connection info has no way to reach the home server.
home-ticket-no-addresses = The connection info has no relay address or direct address for your home server.

home-ticket-relays-invalid = The relay table isn't valid: { $reason }

# $domain is the unchanged base domain from the connection info.
home-ticket-invalid-domain = The base domain isn't valid: { $domain }

# $error is the URL parser's original error.
home-ticket-invalid-web-url = The address for app pages isn't valid: { $error }

# $domain is the connection info's base domain, $url the unchanged address from the connection info.
home-ticket-web-url-under-domain = The address for app pages must be an http(s) address under { $domain }: { $url }

# $error is the URL parser's original error.
home-ticket-invalid-core-url = The home server address isn't valid: { $error }

# $domain is the connection info's base domain, $url the unchanged address from the connection info.
home-ticket-core-url-under-domain = The home server address must be an http(s) address of { $domain }: { $url }

# $error is the network setup's original error, shown while pairing or while opening a connection.
# No parameters. The device name field on the pairing screen was left empty.
home-pairing-name-empty = The device name is empty.

home-pairing-bind-failed = Can't set up the network connection: { $error }

# $seconds is the bind deadline in seconds. The phone's network stack wasn't ready in time; it retries on its own.
home-bind-timeout = The phone's network wasn't ready within { $seconds } seconds.

# $reason is the connection-info parser's original error.
home-ticket-corrupt = Saved connection information is damaged: { $reason }

home-no-connection-info = There's no connection information for your home server yet.

# Connection setup failed locally; the underlying error is in the separate diagnostic.
home-proxy-start-failed = The app couldn't start its local connection proxy. Try again.

home-tunnel-closed = The tunnel client has closed.

home-lan-unsupported = This version of the client core doesn't support local network settings.

home-port-mapping-save-failed = Couldn't save the port mapping switch: { $error }

# Local storage of the connection (crates/wonton-client/src/home/storage.rs).
# $path is the file's path, $reason is the original parser or I/O error.
home-storage-device-key-corrupt = The device key file { $path } is damaged: { $reason }

home-storage-io-failed = Can't read or write { $path }: { $reason }

# Pairing watchdog: $seconds is the overall watchdog limit, not the exchange stage limit.
home-pairing-timeout = Pairing did not finish within { $seconds } seconds.

home-pairing-aborted = Pairing stopped before it finished. Try again.

home-pairing-not-pairing = No pairing is in progress.

# Readable pairing details. $reason is localized; elapsed is total time, stage is the limit for each exchange stage, both in seconds.
home-pairing-failed-detail = { $reason } Total elapsed: { $elapsed_seconds } seconds. Each pairing exchange stage has a { $stage_seconds }-second limit.

home-pairing-joining-pending = This is a joining code: waiting for an admin to approve.
home-pairing-joining-pending-role = This is a joining code: waiting for an admin to approve. Once approved, you join as { $role }.

# Failure categories without underlying error text; raw details stay in Diagnostics.
home-failure-transport = The connection to your home server couldn't be established or was interrupted. Check both networks and try again.
home-failure-unauthorized = Your home server rejected this device. Pair it again from an authorized device or the server.
home-failure-upstream = Your home server is reachable, but the service it needs isn't ready. Try again later.
home-failure-protocol = The app and home server couldn't agree on the connection protocol. Check for updates on both.
home-network-setup-failed = The app couldn't prepare the phone's network connection. It will retry automatically.
home-pairing-network-setup-failed = The app couldn't prepare the phone's network connection for pairing. Try again.
home-proxy-route-failed = The app couldn't register its local connection route. Try again.

# Explicitly opened, copyable diagnostics. Pairing secrets and relay tokens are redacted.
home-tunnel-diagnostic = Error code: { $code }. Original error: { $error }
home-diagnostic-unavailable = Diagnostic information is unavailable because the saved connection details could not be read safely.
home-pairing-diagnostic = Direct addresses: { $addresses }. Relays: [{ $relays }]. { $error }

# Pairing-code validation. No parameter contains scanned text or a secret.
home-code-not-pairing = This isn't a Wonton pairing code. Scan the code from your home server.
home-code-unsupported-version = This pairing code uses a format this app doesn't recognize. Update the app and try again.
home-code-invalid-version = The pairing code is missing its format version. Get a new code and try again.
home-code-invalid-address = The pairing code has missing or invalid server connection information. Get a new code and try again.
home-code-invalid-relay = The pairing code has invalid relay information. Relay addresses must use HTTPS. Get a new code and try again.
home-code-invalid-domain = The pairing code has an invalid server address. Get a new code and try again.
home-code-invalid-secret = The pairing code's pairing key is missing or invalid. Get a new code and try again.
home-code-invalid = The pairing code has invalid connection information. Get a new code and try again.
home-code-rejected = The server didn't accept this pairing code. Get a new code and try again.
home-code-expired = This pairing code has expired. Get a new code and try again.
home-code-used = This pairing code has already been used or revoked. Get a new code and try again.
home-code-closed = The server no longer accepts this kind of pairing code. Add the device from an authorized device or the server.
home-code-request-invalid = The server couldn't process the pairing request. Check that the app and server are up to date.
home-code-server-error = The server couldn't complete pairing. Try again later.
