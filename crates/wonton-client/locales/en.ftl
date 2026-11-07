## Collapsed technical details. Raw reasons, codes and upstream text stay unchanged.
## No length limit. Each parameter is diagnostic data, not a translation key.

# CoreError prefixes. $reason is the original error detail.
core-error-storage = Can't read or write local storage: { $reason }
core-error-not-ready = Not ready: { $reason }
core-error-transport = Can't reach your home server: { $reason }
core-error-unauthorized = Your home server rejected the request: { $reason }
core-error-upstream = Something went wrong on your home server: { $reason }

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

# Other servers (ADR-0050). Collapsed technical details.
# $reason is the original error detail.
peers-transport = Can't reach your home server: { $reason }
# $seconds is the request timeout in seconds.
peers-timeout = { $seconds ->
    [one] No response within { $seconds } second
   *[other] No response within { $seconds } seconds
}
# There's no connection information or local proxy yet, so the request can't be sent.
peers-not-connected = You're not connected to your home server yet.
# The shell routed an action here that isn't a server-pairing action.
peers-invalid-action = Not a server-pairing action.

# $reason is the local proxy's or HTTP client's original error.
peers-invalid-proxy = The local proxy isn't available: { $reason }
# $status is the unchanged HTTP status code.
peers-server-status = Your home server returned HTTP { $status }
# $reason is the JSON parser's original error.
peers-invalid-response = Can't read your home server's response: { $reason }
# $reason is the tunnel's original refusal text.
peers-rejected = Your home server rejected this device: { $reason }

# Onboarding steps. Shown as error details when an action lands on the wrong
# setup step or the setup status hasn't loaded.
onboarding-steps-not-in-flow = Setup isn't running.
onboarding-steps-already-committed = Setup is already finished.
onboarding-steps-not-profile-step = Setup isn't on the profile step.
onboarding-steps-not-agent-step = Setup isn't on the agent step.
onboarding-steps-state-missing = Setup status hasn't loaded yet.
onboarding-steps-agent-not-listed = That agent isn't in the list of choices.
onboarding-steps-agent-not-installable = That agent can't be installed.
onboarding-steps-not-apps-step = Setup isn't on the apps step.
onboarding-steps-need-not-listed = That item isn't in the recommendation list.
onboarding-steps-committing = Setup is already being submitted.
onboarding-steps-no-installable-agent = No installable agent is selected.
onboarding-steps-testing = A connection test is still running.
onboarding-steps-model-not-chosen = No model is selected.
onboarding-steps-cannot-skip = This step can't be skipped.
onboarding-steps-cannot-go-back = You can't go back from this step.

# Connecting a model provider during setup.
onboarding-model-key-required = The API key is empty.
onboarding-model-not-model-step = Setup isn't on the model step.
onboarding-model-testing = A connection test is already running.
onboarding-model-providers-missing = The provider list hasn't loaded yet.
onboarding-model-provider-not-listed = That provider isn't in the list.
onboarding-model-custom-unsupported = The model gateway doesn't support custom providers.
onboarding-model-name-required = The model name is empty.
onboarding-model-base-url-invalid = The base URL must be an https:// address.
onboarding-model-step-or-testing = Setup isn't on the model step, or a connection test is running.
onboarding-model-none-configured = The model gateway has no connected models.
onboarding-model-state-missing = Setup status hasn't loaded yet.
onboarding-model-login-missing = The selected agent doesn't have that sign-in method.

# The review page. A blocked item keeps its message until the preview refreshes.
onboarding-review-permissions-changed = The requested permissions changed since they were approved. Reinstall the app from its page.
# $installed and $listed are unchanged IDs from the install source and the review list.
onboarding-review-source-mismatch = The install source provides { $installed }, not the listed { $listed }.
onboarding-review-items-not-ready = Some items aren't ready yet.
onboarding-review-item-missing = That item isn't on the review page.
onboarding-review-no-failure = That item hasn't failed.

## Fragments for crates/wonton-client/src/chat/{login,sign_in,recovery}.rs (chat-a).
## These are the collapsed technical details of a chat problem or error:
## the CoreError reason and the ChatProblem message shown under "Details".

# Sign-in details. $reason is the Matrix SDK's original error.
chat-sign-in-restore-failed = Can't restore your Matrix session: { $reason }
chat-sign-in-no-connection-info = There's no connection information for your home server yet.
chat-sign-in-login-failed = Can't sign in to Matrix: { $reason }
chat-sign-in-no-saved-session = No session to save after signing in.

# Login-token request details. $reason is an unchanged upstream error or a localized timeout detail.
# $reason is the proxy URL parser's original error.
chat-login-invalid-proxy = The local proxy address isn't valid: { $reason }
# $reason is the HTTP client's original error.
chat-login-http-client = Can't create an HTTP client: { $reason }
# $seconds is the request timeout in seconds.
chat-login-timeout = { $seconds ->
    [one] No response within { $seconds } second
   *[other] No response within { $seconds } seconds
}
chat-login-request-failed = Can't get a Matrix login token from your home server: { $reason }
# $reason is the HTTP body reader's original error.
chat-login-response-failed = Can't read your home server's response: { $reason }
# $reason is the JSON parser's original error.
chat-login-invalid-token = Your home server returned a login token in an unexpected format: { $reason }
# $login_type is the login type the home server returned; $expected is the required one.
chat-login-wrong-type = Your home server returned the login type { $login_type } instead of { $expected }
# $status is the unchanged HTTP status code.
chat-login-server-status = Your home server returned HTTP { $status }
# $message and $kind are the server's original error text and error code.
chat-login-server-error = { $message } ({ $kind })

# Recovery details.
chat-recovery-not-connected = You're not connected to your home server yet.
chat-recovery-reset-no-response = Your home server didn't respond. Your backup is unchanged; try again later.
chat-recovery-empty-key = Your recovery key can't be empty.
# $reason is the original storage error.
chat-recovery-key-not-saved = Your recovery key couldn't be saved on this device. Write it down now: { $reason }
# Approval request (confirmation card) details, shown in the problem detail under the card
# when an answer or a stop on the time window didn't go through.
# $message is the home server's original error text, or a localized detail.
chat-confirmation-answer-failed = The answer didn't go through: { $message }
# $error is the HTTP client's original error.
chat-confirmation-answer-lost = No response from your home server, so it isn't clear whether the answer went through. You can try again. ({ $error })
# $message is the home server's original error text, or a localized detail.
chat-confirmation-stop-failed = Stopping didn't take effect: { $message }
# $error is the HTTP client's original error.
chat-confirmation-stop-lost = No response from your home server. You can try again. ({ $error })
# $error is the JSON parser's original error.
chat-confirmation-response-unreadable = The response from your home server couldn't be read ({ $error })
# $status is the unchanged HTTP status code.
chat-confirmation-core-status = Your home server returned HTTP { $status }
## Error details shown in the chat screen's problem banner and in a message's Details section.

# Shown when starting a direct chat with an empty username.
chat-contacts-empty-username = Enter a username.
# $reason is the Matrix ID parser's original error.
chat-contacts-invalid-user-id = That user ID isn't valid: { $reason }

# Shown when sending a message through Siri or Shortcuts with empty text.
chat-ask-empty-message = Type a message first.
# Shown when the bot ID handed in from Siri or Shortcuts doesn't parse.
chat-ask-invalid-bot-id = The default agent's ID isn't formatted correctly.
# Shown when a saved direct chat's room ID doesn't parse.
chat-ask-invalid-room-id = The direct chat's room ID isn't valid.

# Shown when the account is marked failed because Matrix revoked this device.
chat-session-device-revoked = This device was revoked on Matrix. Pair it with your home server again.
## File transfers and playback. Details show on transfer list entries, file cards and playback failure messages.

# Shown when a transfer or playback starts before the connection is up.
files-api-not-connected = You're not connected to your home server yet.
# $reason is the proxy URL parser's original error.
files-api-invalid-proxy = The local proxy address isn't valid: { $reason }
# $reason is the HTTP client's original error.
files-api-http-client = Can't create an HTTP client: { $reason }
# $reason is the JSON parser's original error.
files-api-invalid-response = Can't read your home server's response: { $reason }
# Shown when the home server rejects the device's authorization.
files-api-unauthenticated = Your home server no longer recognizes this device
# $code is the unchanged tunnel error code.
files-api-tunnel-unavailable = The tunnel isn't available ({ $code })
# $status is the unchanged HTTP status code.
files-api-status = Your home server returned HTTP { $status }

# Shown when the original file changed since it was chosen.
files-upload-source-changed = The original file has changed. Choose it again.
# Shown when the home server's progress (HEAD) response lacks the Upload-Offset header.
files-upload-head-no-offset = Your home server's response to the progress query has no Upload-Offset header
# Shown when the app shell can't open an opaque source.
files-upload-no-opener = The app shell didn't provide a file opener, so this source can't be opened
# Shown when the source isn't a regular seekable file.
files-upload-source-not-file = This source isn't a regular seekable file, so the upload can't resume
# $reason is the panic message of the task that opens the original file.
files-upload-open-task-failed = The task that opens the original file ended unexpectedly: { $reason }
# $reason is the original io error.
files-upload-source-denied = The system doesn't allow reading the original file: { $reason }
# $reason is the original io error.
files-upload-source-missing = The original file is gone: { $reason }
# $reason is the original io error.
files-upload-source-read = Can't read the original file: { $reason }
# Shown when the home server's create-upload response has no valid Location header.
files-upload-create-no-location = Your home server's response to creating the upload has no valid Location header
# Shown when the home server's progress (HEAD) response lacks the Upload-Length header.
files-upload-head-no-length = Your home server's response to the progress query has no Upload-Length header
# Shown when no bytes move within the stall timeout.
files-upload-stalled = The upload stopped making progress; the tunnel may be stuck
# Shown when the home server never answers the final PATCH.
files-upload-no-response = Your home server stopped responding
# Shown when the original file is shorter than recorded.
files-upload-source-shrank = The original file got shorter

# $reason is the original io error.
files-download-write = Can't write the downloaded file: { $reason }
# Shown when the home server never answers the download request.
files-download-no-response = Your home server stopped responding
# Shown when the partial file is longer than the home server's file.
files-download-shrunk = The file got shorter; downloading from the beginning
# Shown when the resumed range doesn't match what was already downloaded.
files-download-range-mismatch = The range from your home server doesn't match; downloading from the beginning
# Shown when the connection drops before all bytes arrive.
files-download-incomplete = The connection dropped before the file was complete
# Shown when no data arrives within the stall timeout.
files-download-stalled = No data is arriving; the tunnel may be stuck

# Shown when playback fails because the connection is down.
files-playback-transport = Can't reach your home server. You can download the file and open it in another app.
# Shown when playback fails because this device can't decode the format.
files-playback-unsupported = This phone can't play this format. You can download the file and open it in another app.
# $detail is the unchanged reason from the home server or the connection.
files-playback-unavailable = Your home server can't provide this file: { $detail }
# $detail is the unchanged reason from the home server or the connection.
files-playback-error = Something went wrong on your home server: { $detail }. You can download the file and open it in another app.
# Shown when the playback probe times out.
files-playback-no-response = Your home server stopped responding

# $reason is the original io error.
files-stream-local-address = Can't open the local address for playback: { $reason }
# Diagnostic detail for playback failures while the tunnel is down.
files-stream-not-connected = The tunnel isn't connected
# $code is the unchanged tunnel error code.
files-stream-tunnel-unavailable = The tunnel isn't available ({ $code })
## Details for the home connection and pairing, shown in the Details section of problems on the home screen.

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
# $error is the network setup's original error.
# No parameters. The device name field on the pairing screen was left empty.
home-pairing-name-empty = The device name is empty.
home-pairing-bind-failed = Can't set up the network connection: { $error }
# Tabs module (crates/wonton-client/src/tabs/view.rs, tabs/config.rs) and appearance module (appearance/mod.rs).
# These are user-facing "details" (CoreError reasons, problem messages) shown collapsed under errors in the shells.
# Category 2 (logs, English, not here): expect!/panic messages in the same files were reworded in English.
# Category 3 (unchanged, see tabs.report.md): plugin titles and page titles in tab metadata.

# Tabs ViewModel errors. Shown as the reason of a CoreError when a tab can't be stored, or as a TabsProblem message.
# No parameters.
tabs-view-always-pinned = Chat and Apps are already pinned to the Navigation bar.
tabs-view-plugin-list-not-ready = The app list hasn't loaded yet, so app pages can't be changed right now.
tabs-view-unknown-plugin = Can't find this app.
tabs-view-plugin-no-page = This app has no web page to open.

# Navigation bar settings file (tabs.json) errors. Shown as the reason of a CoreError::Storage / CoreError::InvalidInput,
# which the problem banner wraps as "Can't read or write local storage: …".
# $path is the settings file's path, $reason is the original parser or I/O error.
tabs-config-read-failed = Can't read the Navigation bar settings file { $path }: { $reason }
tabs-config-corrupt = The Navigation bar settings file { $path } is damaged: { $reason }
# $count is the maximum number of items on the Navigation bar (MAX_ITEMS).
tabs-config-too-many-items = { $count ->
    [one] The Navigation bar can hold at most { $count } item.
   *[other] The Navigation bar can hold at most { $count } items.
}
tabs-config-duplicate-page = The Navigation bar has duplicate pages.
tabs-config-plugin-page-incomplete = An app page is missing its title and icon.
tabs-config-missing-fixed = The Navigation bar is missing Chat or Apps.
# $path is the settings file's path, $reason is the original I/O error.
tabs-config-write-failed = Can't write the Navigation bar settings file { $path }: { $reason }

# Appearance settings errors. appearance-settings-unreadable is shown as AppearanceProblem.message on the appearance page;
# the others are reasons inside it, or the reason of a CoreError::Storage.
# $path is the settings file's path, $reason is the original parser or I/O error.
appearance-settings-unreadable = Can't read the appearance settings, so the defaults are in use for now: { $reason }
appearance-settings-read-failed = Can't read the appearance settings file { $path }: { $reason }
appearance-settings-corrupt = The appearance settings file { $path } is damaged: { $reason }
# $value is the unchanged invalid color string from the settings file.
appearance-settings-invalid-accent = The color in the appearance settings file { $path } isn't valid: { $value }
# $path is the settings file's path, $reason is the original I/O error.
appearance-settings-write-failed = Can't write the appearance settings file { $path }: { $reason }

## Plugin module: technical details, collapsed by default in the Details of
## errors, app task rows, and the Internet proxy / Download Mirror settings pages.
## No length limit. Each parameter is diagnostic data, not a translation key.

# Plugin management API (crates/wonton-client/src/plugins/api.rs): shown in the
# Details of a failed request, after the failure reason.
# $reason is the original error detail.
plugins-api-invalid-proxy = The local proxy address isn't valid: { $reason }
plugins-api-http-client = Can't create an HTTP client: { $reason }
# $reason is the JSON parser's original error.
plugins-api-invalid-response = Your home server returned something in an unexpected format: { $reason }
# $reason is the JSON serializer's original error.
plugins-api-invalid-request = Can't write the request body: { $reason }
# $reason is the rejected image address, or the URL parser's original error.
plugins-api-not-image-url = Not an app image address: { $reason }
# $status is the unchanged HTTP status code.
plugins-api-server-status = Your home server returned HTTP { $status }
plugins-api-not-connected = Try again once you're connected to your home server.

# Install, upgrade and uninstall tasks (crates/wonton-client/src/plugins/tasks.rs):
# shown under a task that ended without a known result, and in the Details of a refused task.
plugins-tasks-uncertain = The connection dropped, so the result from your home server is unknown. Check the installed list.
plugins-tasks-server-changed = Your home server changed, so the result from the previous one is unknown.
plugins-tasks-preview-no-source = This preview has no source, so it can't be installed.
# $id is the unchanged task ID.
plugins-tasks-no-such-task = There's no such task: { $id }
plugins-tasks-task-not-done = The task isn't finished yet.
# $title is the app's title, shown as it comes from the store.
plugins-tasks-task-running = { $title } is already being installed, updated, or uninstalled.

# Internet proxy settings (crates/wonton-client/src/plugins/egress.rs): shown
# in the Details of the settings page.
plugins-egress-save-in-progress = The previous change is still being saved.
plugins-egress-not-loaded = The Internet proxy settings haven't loaded yet.
plugins-egress-no-confirmation = There's no change waiting for approval.
plugins-egress-env-locked = The proxy is set by WONTON_EGRESS_PROXY and can't be changed here.
plugins-egress-plugin-and-address = You can't use a proxy app and a proxy address at the same time.
plugins-egress-plugin-not-installed = The proxy app isn't installed yet.
# $reason is the connection problem's original detail.
plugins-egress-save-uncertain = It isn't certain that the change was saved. Check again once you're connected: { $reason }

# Download Mirror settings (crates/wonton-client/src/plugins/registry_mirror.rs):
# shown in the Details of the settings page.
plugins-registry-save-in-progress = The previous Download Mirror change is still being saved.
plugins-registry-not-loaded = The Download Mirror settings haven't loaded yet.
# $reason is the connection problem's original detail.
plugins-registry-save-uncertain = It isn't certain that the change was saved. Read the settings again once you're connected: { $reason }

# Multiple servers (crates/wonton-client/src/servers.rs, client/push_route.rs).
servers-removed = This server has been removed from this phone.
servers-push-from-removed = The server that sent this message has been removed from this phone.
servers-push-scope-closed = That server has already been closed.
# $seconds is how long registration waited.
servers-push-register-timeout = { $seconds ->
    [one] Couldn't register for notifications within { $seconds } second.
   *[other] Couldn't register for notifications within { $seconds } seconds.
}

chat-avatar-cache-not-file = The avatar cache path isn't a file.
chat-avatar-cache-hash-collision = Two avatars in the cache have the same hash.
chat-avatar-cache-missing-mxc = An avatar in the cache has no MXC address.
chat-avatar-cache-path-not-utf8 = The avatar cache path isn't valid UTF-8.
scope-task-crashed = A background task ended unexpectedly: { $error }
scope-switched-server = You've switched to another server.

## The rest of the client core (ADR-0043): collapsed technical details for the
## modules that C1 and C2 didn't cover. Logs in the same files are English and
## not here. Push notification bodies and placeholders stay in code for now.

# File transfers (crates/wonton-client/src/files/mod.rs): details of refused
# actions, failed operations and local-file cleanup.
files-invalid-action = Not a file action.
files-name-required = The file name can't be empty.
files-download-already-running = This file is already downloading.
files-transfer-not-found = Can't find this transfer.
# Shown when resending the link of a transfer that isn't in the link-not-sent state.
files-resend-not-waiting = This transfer has no link waiting to be sent.
files-transfer-server-changed = Your home server has changed, so this transfer belongs to the previous one.
files-dismiss-transfer-running = This transfer is still running. Cancel it first.
# Shown when dismissing an upload and asking to delete its local file: the
# original file was provided by the system, so Wonton can't delete it.
files-dismiss-upload-source-protected = Wonton doesn't manage the original file of an upload.
# $reason is the original io error.
files-delete-local-failed = Can't delete the file on this phone: { $reason }
# The outcome of a background file operation. $id is the unchanged file ID,
# $message is the original failure detail.
files-delete-failed = Can't delete file { $id }: { $message }
files-send-failed = Can't send file { $id } to your devices: { $message }
files-grant-failed = Can't grant file { $id } to the chat: { $message }
files-play-failed = Can't play file { $id }: { $message }
# $path is the transfer list's path, $reason is the original I/O error.
files-store-write-failed = Can't write the transfer list { $path }: { $reason }

# Local storage of the connection (crates/wonton-client/src/home/storage.rs).
# $path is the file's path, $reason is the original parser or I/O error.
home-storage-device-key-corrupt = The device key file { $path } is damaged: { $reason }
home-storage-io-failed = Can't read or write { $path }: { $reason }

# Chat local storage (crates/wonton-client/src/chat/storage.rs).
# $path is the file's path, $reason is the original parser or I/O error.
chat-storage-session-corrupt = The chat sign-in state file { $path } is damaged: { $reason }
chat-storage-io-failed = Can't read or write { $path }: { $reason }

# The list of paired servers (crates/wonton-client/src/servers.rs).
# $path is the server list file's path, $reason is the original parser or I/O error.
servers-list-corrupt = The server list file { $path } is damaged: { $reason }
servers-add-current-unpaired = Set up this server before adding another one.
servers-name-required = The name can't be empty.
# $path is the path being read, written or deleted, $reason is the original I/O error.
servers-storage-failed = Can't read or write { $path }: { $reason }

# The session list (crates/wonton-client/src/session_store.rs).
sessions-title-required = The session title can't be empty.
# $id is the unchanged session ID.
sessions-not-found = There's no such session: { $id }

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

# Starting an app on the home server (crates/wonton-client/src/plugins/start.rs).
# $id is the unchanged app ID.
plugins-start-busy = { $id } is already starting, installing, updating, or uninstalling.

# The bridge of app pages (crates/wonton-client/src/tabs/bridge/).
tabs-bridge-page-not-open = This app page isn't open or pinned.
tabs-bridge-invalid-environment = The page insets and safe area must be finite, non-negative numbers.

# Chat transport problems (crates/wonton-client/src/chat/view.rs): the reason
# of a CoreError::TransportUnavailable and the message of a failed chat action.
chat-not-connected = Try again once you're connected to your home server.

# Onboarding records (crates/wonton-client/src/onboarding/storage.rs).
# $path is the file's path, $reason is the original I/O error.
onboarding-storage-io-failed = Can't read or write { $path }: { $reason }

# The home server's one-click update (crates/wonton-client/src/plugins/core_upgrade.rs):
# messages of uncertain stages and refused actions, shown on the update page and
# in the Details of errors.
# No parameters. The home server reported a result this version of Wonton can't read.
plugins-core-upgrade-unrecognized = Your home server returned something this version of Wonton doesn't understand. Update Wonton and check again.
# Shown in place of a missing running digest. Not a sentence.
plugins-core-upgrade-unknown-digest = unknown
# The home server reports the update as finished, but the running version doesn't match.
# $running is the running digest, or the unknown-digest word; $target is the unchanged target digest.
plugins-core-upgrade-completed-mismatch = Your home server says the update finished, but what's running isn't the target version: running { $running }, target { $target }.
# The deadline passed without a connection, so the outcome is unknown. $digest is the unchanged
# digest to pin for going back to the old version.
plugins-core-upgrade-timed-out = Your home server didn't respond within the time limit, so the update result is unknown. The result will show here once you're connected.
plugins-core-upgrade-timed-out-rollback = Your home server didn't respond within the time limit, so the update result is unknown. The result will show here once you're connected. To go back to the old version, run the install command on your computer again with WONTON_CORE_PIN={ $digest }.
plugins-core-upgrade-invalid-action = Not a home server update action.
plugins-core-upgrade-already-running = An update is already in progress.
plugins-core-upgrade-not-cancellable = The update can't be cancelled right now.
# $reason is the connection problem's original detail.
plugins-core-upgrade-start-uncertain = It isn't certain that the update was started. Check again once you're connected: { $reason }
