# Multiple servers (crates/wonton-client/src/servers.rs, client/push_route.rs).
servers-removed = This server has been removed from this phone.

servers-push-from-removed = The server that sent this message has been removed from this phone.

servers-push-scope-closed = That server has already been closed.

# $seconds is how long registration waited.
servers-push-register-timeout = { $seconds ->
    [one] Couldn't register for notifications within { $seconds } second.
   *[other] Couldn't register for notifications within { $seconds } seconds.
}

# The list of paired servers (crates/wonton-client/src/servers.rs).
# $path is the server list file's path, $reason is the original parser or I/O error.
servers-list-corrupt = The server list file { $path } is damaged: { $reason }

servers-add-current-unpaired = Set up this server before adding another one.

servers-name-required = The name can't be empty.

# $path is the path being read, written or deleted, $reason is the original I/O error.
servers-storage-failed = Can't read or write { $path }: { $reason }

# Shown when the shell routes a server list action this core doesn't know.
servers-invalid-action = Not a server list action.
