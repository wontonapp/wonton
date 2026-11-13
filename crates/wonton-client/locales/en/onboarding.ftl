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

# $error is the local proxy's original bind error.
home-proxy-start-failed = Can't start the local proxy: { $error }

home-tunnel-closed = The tunnel client has closed.

home-lan-unsupported = This version of the client core doesn't support local network settings.

home-port-mapping-save-failed = Couldn't save the port mapping switch: { $error }

# Local storage of the connection (crates/wonton-client/src/home/storage.rs).
# $path is the file's path, $reason is the original parser or I/O error.
home-storage-device-key-corrupt = The device key file { $path } is damaged: { $reason }

home-storage-io-failed = Can't read or write { $path }: { $reason }

# Pairing details. Addresses/relay URLs contain no pairing secret; error is the unchanged upstream reason.
home-pairing-timeout = Pairing did not finish within { $seconds } seconds.

home-pairing-aborted = Pairing stopped before it finished. Try again.

home-pairing-not-pairing = No pairing is in progress.

home-pairing-failed-detail = Direct addresses: { $addresses }. Relays: [{ $relays }]. Elapsed: { $milliseconds } ms; time limit: { $seconds } seconds. Reason: { $error }

home-pairing-joining-pending = This is a joining code: waiting for an admin to approve.
home-pairing-joining-pending-role = This is a joining code: waiting for an admin to approve. Once approved, you join as { $role }.
home-pairing-joining-rejected = An admin turned down this joining request.
home-pairing-joining-code-used = Another device already used this joining code.
