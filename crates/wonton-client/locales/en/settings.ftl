# The push method setting (crates/wonton-client/src/push_method.rs).
# $path is the settings file's path, $reason is the original I/O error.
push-method-write-failed = Can't write the push method settings file { $path }: { $reason }

# The fake-IP path setting (crates/wonton-client/src/home/path_settings.rs).
# $path is the settings file's path, $reason is the original I/O error.
path-settings-write-failed = Can't write the path settings file { $path }: { $reason }

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

# Notification text for an attachment (ADR-0118): the type and the filename.
push-preview-image = Photo { $filename }
push-preview-file = File { $filename }

push-core-closed = The client core has shut down.

push-not-paired = You haven't connected to your home server yet.

push-home-unreachable = Can't reach your home server: { $reason }

# $seconds is how long signing in waited.
push-session-timeout = { $seconds ->
    [one] Couldn't sign in to Matrix within { $seconds } second.
   *[other] Couldn't sign in to Matrix within { $seconds } seconds.
}

push-room-key-missing = The room key for this message hasn't arrived yet.

# $path is the private device file; $reason is the underlying storage error.
notification-lock-screen-storage-failed = Can't read or save lock screen settings { $path }: { $reason }

# Privacy page (crates/wonton-client/src/about/mod.rs).
privacy-what-title = No cloud
privacy-what-own = Wonton has no cloud of its own. The server is the machine in your home (or one you rent): messages, photos and files live on it.
privacy-what-none = We get no accounts or device information. The app has no analytics, ads or crash reporting.

privacy-messages-title = Chats and files
privacy-messages-e2ee = Chats and files are end-to-end encrypted, and only your devices can open them.
privacy-messages-history = History stays on your own server, and you hold the backup key.

privacy-push-title = Notifications (Android only)
privacy-push-keepalive = With "stay connected", notifications never leave your tunnel: the other end is your home.
privacy-push-unified = With UnifiedPush, your home core first encrypts each notification with your device key and hands it to the distributor. The distributor sees ciphertext only, and it is decrypted on the phone. The distributor is a third-party service.

privacy-network-title = Reaching home
privacy-network-lan = On the same network the app connects directly. Away from home it tries to punch a hole first and only relays when that fails.
privacy-network-relay = A relay forwards encrypted traffic and sees no content. The default is iroh's public relay; your home can switch to its own.

privacy-preview-title = Link previews
privacy-preview-core = When a chat shows a link, your home core fetches the page's title and image, so the site sees your home server's address.

privacy-plugins-title = Third-party sign-in and plugin pages
privacy-plugins-oidc = Signing in to a plugin with another account sends the name and password to that plugin's server only; the home core keeps no third-party password.
privacy-plugins-web = Plugin pages open in your phone's browser engine, and whether and where they go online is up to the plugin.

privacy-device-title = What stays on the phone
privacy-device-files = Device keys, connection details, caches and logs sit in the app's private directory, unreadable by other apps. Uninstalling removes them and leaves the server history alone.
privacy-device-backup = These files follow the system backup (iCloud or Google), and the system setting decides whose account holds that backup.

# License list load errors (crates/wonton-client/src/about/mod.rs).
licenses-parse-failed = The license list didn't parse: { $error }
licenses-empty = The license list has no components.
licenses-no-text = License { $id } has no full text.
licenses-none-to-show = The license list has no licenses to show.
