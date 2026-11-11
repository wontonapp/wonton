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

plugins-api-server-too-old = Home server { $version } is too old: this needs { $minimum } or newer.

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

# Starting an app on the home server (crates/wonton-client/src/plugins/start.rs).
# $id is the unchanged app ID.
plugins-start-busy = { $id } is already starting, installing, updating, or uninstalling.

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
