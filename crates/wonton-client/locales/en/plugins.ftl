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
files-playback-transport = Can't reach your home server.

# Shown when playback fails because this device can't decode the format.
files-playback-unsupported = This phone can't play this format. You can download the file and open it in another app.

# $detail is the unchanged reason from the home server or the connection.
files-playback-unavailable = Your home server can't provide this file: { $detail }

# $detail is the unchanged reason from the home server or the connection.
files-playback-error = Something went wrong on your home server: { $detail }.

# Shown when the playback probe times out.
files-playback-no-response = Your home server stopped responding

# $reason is the original io error.
files-stream-local-address = Can't open the local address for playback: { $reason }

# Diagnostic detail for playback failures while the tunnel is down.
files-stream-not-connected = The tunnel isn't connected

# $code is the unchanged tunnel error code.
files-stream-tunnel-unavailable = The tunnel isn't available ({ $code })

# Tabs ViewModel errors. Shown as the reason of a CoreError when a tab can't be stored, or as a TabsProblem message.
# No parameters.
tabs-view-always-pinned = Chat and Apps are already pinned to the Navigation bar.

tabs-view-plugin-list-not-ready = The app list hasn't loaded yet, so app pages can't be changed right now.

tabs-view-unknown-plugin = Can't find this app.

# $error is the local store's original error.
tabs-storage-problem = Couldn't save My Apps on this device; the next launch may restore the old state: { $error }

tabs-view-plugin-no-page = This app has no web page to open.

tabs-page-not-placed = This page isn't in the panel.

# Navigation bar settings file (tabs.json) errors. Shown as the reason of a CoreError::Storage / CoreError::InvalidInput,
# which the problem banner wraps as "Can't read or write local storage: …".
# $path is the settings file's path, $reason is the original parser or I/O error.
tabs-config-read-failed = Can't read the My Apps settings file { $path }: { $reason }

tabs-config-corrupt = The My Apps settings file { $path } is damaged: { $reason }

# $count is the maximum number of items on the Navigation bar (MAX_ITEMS).
tabs-config-too-many-items = { $count ->
    [one] The Navigation bar can hold at most { $count } item.
   *[other] The Navigation bar can hold at most { $count } items.
}

tabs-config-duplicate-page = My Apps has duplicate pages.

tabs-config-plugin-page-incomplete = An app page is missing its title and icon.

tabs-config-missing-fixed = The Navigation bar is missing Chat.

# $path is the settings file's path, $reason is the original I/O error.
tabs-config-write-failed = Can't write the My Apps settings file { $path }: { $reason }

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

# The session list (crates/wonton-client/src/session_store.rs).
sessions-title-required = The session title can't be empty.

# $id is the unchanged session ID.
sessions-not-found = There's no such session: { $id }

# Shown when the shell routes a session action this core doesn't know.
sessions-invalid-action = Not a session action.

# The bridge of app pages (crates/wonton-client/src/tabs/bridge/).
tabs-bridge-page-not-open = This app page isn't open.

tabs-bridge-invalid-environment = The page insets and safe area must be finite, non-negative numbers.

tabs-bridge-public-page = Public web pages get no Wonton bridge.

tabs-page-cannot-reload = This page can't be reloaded.

tabs-bar-not-pinned = This page isn't pinned to the Navigation bar.

tabs-bar-already-pinned = This page is already in My Apps.

tabs-bar-replace-required = The Navigation bar is full; pick a page to replace.

tabs-bar-replacing-missing = The page to replace isn't in the Navigation bar.

tabs-bar-fixed = Chat and Apps can't be replaced.

tabs-bar-not-full = The Navigation bar isn't full, so there's nothing to replace.

tabs-bar-remove-fixed = Chat and Apps can't be removed.

tabs-bar-page-missing = This page isn't in My Apps.

tabs-bar-index-out-of-range = That position is past the end of My Apps.

tabs-open-only-plugins = Only app pages have an opening mode.

tabs-fullscreen-only-plugins = Only app pages can go full screen.

tabs-fullscreen-none = Nothing is full screen right now.

tabs-fullscreen-not-current = That page isn't the full-screen one.

# Errors when opening an app as a Matrix widget inside a room (ADR-0096). No parameters.
tabs-widget-not-signed-in = Sign in to chat first, then open this app from the room.

tabs-widget-not-declared = This app can't be opened inside a room.

tabs-widget-not-pinnable = Apps opened inside a room can't be pinned.

tabs-view-not-a-page = Only app pages can be opened here.
tabs-view-no-page-open = No app page is open right now.

# 出错页默认收起的"详情"（ADR-0032）：核心按 App 语言写，错误码收在里面。
tabs-load-details-transport = The page reloads by itself once your home server is reachable again.

tabs-load-details-not-ready = The app's page wasn't ready yet (HTTP {$code}).

tabs-load-details-upstream = The app's page returned an error (HTTP {$code}).
