## Texts the home server writes about its own update (ADR-0025): what the App's update
## page shows and the reasons in the details. Background checks and the updater write
## these in the owner's language; the management API answers in the request's language.
## English is the base language, in Sentence case. Errors read like "Can't reach …".
## Data (paths, image names, digests, upstream output) goes in as parameters, untranslated.

# The update API refused the request.
upgrade-not-installed = This home server wasn't installed the standard way, so one-click updates don't work
upgrade-not-in-container = The home server isn't running in a container, so one-click updates don't work
upgrade-busy = An update is already running
upgrade-not-downloaded = The new version hasn't finished downloading
upgrade-channel-changed = The version on the update channel has changed; confirm the update again
upgrade-pinned = This installation is pinned to one version and doesn't follow the update channel
upgrade-not-upgrading = No update is running
upgrade-cancel-too-late = The update has already started making changes, so it can't be canceled
upgrade-launch-updater-failed = Can't start the updater: { $error }

# The API couldn't answer for its own sake.
upgrade-container-missing = Can't find the home server's container { $name }
upgrade-image-missing = Can't find the running image { $image }; restarting the home server fixes this

# Background check (every 6 hours, or when the update page opens).
upgrade-check-core-failed = Can't check the home server's container: { $error }
upgrade-latest-version-failed = Can't find the latest version of { $source }: { $error }
upgrade-image-lookup-failed = Can't look up the image: { $error }
upgrade-registry-digest-missing = The registry didn't return a digest for { $source }
upgrade-free-space-failed = Can't check the free space on { $path }: { $error }
upgrade-pull-stalled = Pulling { $image } made no progress for 5 minutes
upgrade-auto-download-skipped = Less than 5 GB of disk is free, so it wasn't downloaded automatically
upgrade-download-no-space = Not enough disk space: the download needs about { $needed } bytes, but only { $free } bytes are available
upgrade-download-failed = Can't download it: { $error }
upgrade-image-missing-after-pull = It was downloaded, but { $reference } still isn't on this machine

# The compatibility check before the update (ADR-0085). It never blocks the update:
# when it can't run, the details say so and the update goes ahead all the same.
upgrade-compat-records-unreadable = Can't read the installed apps' records: { $error }
upgrade-compat-materialize-failed = Can't stage the installed apps' packages for the compatibility check: { $error }
upgrade-compat-run-failed = Can't run the compatibility check in the new image { $image }: { $error }
upgrade-compat-command-failed = The compatibility check in the new image { $image } exited with code { $code }: { $error }
upgrade-compat-output-unreadable = Can't read the compatibility check's output: { $error }

# The updater container reports what it did into the state file, in the owner's language.
upgrade-updater-crashed = The updater keeps crashing
upgrade-phase-unreadable = Can't read the phase in the state file
upgrade-updater-gone = The updater is gone; it's unclear whether it got through { $phase }
upgrade-rollback-cause = { $message }. The rollback started because: { $cause }
upgrade-restore-also-failed = { $message }. Starting the old version again failed too: { $error }
upgrade-state-unreadable = Can't read { $path }; it may have been written by a newer version
upgrade-state-no-reason = The state file doesn't say why
upgrade-state-serialize = Can't serialize the update state
upgrade-no-backup-dir = The state file doesn't name a backup directory
upgrade-restore-pin-hint = To go back to the old version, run the install command on the computer again with WONTON_CORE_PIN={ $digest }

# Pulling and running things on the host.
upgrade-list-images-failed = The new image { $image } can't list the images it needs: { $error }
upgrade-no-image = The image { $image } isn't on this machine
upgrade-inspect-image-failed = Can't look up the image { $image }: { $error }
upgrade-pull-image-failed = Can't pull the image { $image }: { $error }
upgrade-pull-image-timed-out = Pulling the image { $image } took too long
upgrade-run-in-image-failed = Can't run { $cmd } in the image { $image }: { $error }
upgrade-window-exceeded = It took over 30 minutes from stopping the home server to the new version starting
upgrade-components-failed =
    { $target ->
        [new] Can't remove the components the new version doesn't support: { $error }
       *[old] Can't remove the components the old version doesn't support: { $error }
    }
upgrade-up-failed = The services didn't come up: { $error }
upgrade-core-image-wrong = The home server's container is running { $actual } (image { $actual_id }), not { $image } (image { $image_id })
upgrade-core-container-missing = The home server's container is missing
upgrade-containers-failed = Can't check the system services' containers: { $error }
upgrade-observe-timed-out = Watching the new version took too long
upgrade-rollback-timed-out = The rollback took over 20 minutes
upgrade-stop-service-failed = Can't stop the service { $service }: { $error }
upgrade-stop-core-failed = Can't stop the home server: { $error }

# Backup and restore (the state file says where it went wrong).
upgrade-backup-failed = Can't make the backup: { $error }
upgrade-backup-incomplete = The backup at { $path } wasn't finished, so it can't be restored from
upgrade-volume-size-failed = Can't measure the volume { $volume }: { $error }
upgrade-volume-size-unknown = Can't get the size of the volume { $volume }: { $error }
upgrade-disk-space-failed = Can't check free disk space: { $error }
upgrade-backup-no-space = Not enough disk space: the backup needs { $needed } bytes, only { $free } bytes are available, and old backups can free up { $old } bytes
upgrade-backup-volume-failed = Can't back up the volume { $volume }: { $error }
upgrade-restore-failed = Can't restore the backup: { $error }
upgrade-init-failed = The new version's init failed: { $error }

# What went wrong while a service was being watched after the switch.
upgrade-service-container-gone = The container for service { $service } is gone
upgrade-service-container-replaced = The container for service { $service } was replaced
upgrade-service-image-changed = Service { $service } switched to a different image: { $image }
upgrade-service-restarted = Service { $service } restarted
upgrade-service-status = Service { $service } is { $status }
upgrade-service-unhealthy = Service { $service } became unhealthy
upgrade-one-shot-exit-code = The one-shot service { $service } exited with code { $code }

# Short contexts in front of the system's own error, like "read /x: Permission denied".
upgrade-env-missing-key = { $path } is missing { $key }

# compose (in the core and in the updater) and one-shot jobs.
upgrade-compose-run-failed = Can't run { $path }: { $error }
upgrade-compose-timed-out = compose didn't finish within { $seconds } seconds
upgrade-compose-wait-failed = Can't wait for compose to finish: { $error }
upgrade-compose-failed = compose exited { $status }: { $error }
upgrade-one-shot-timed-out = The container { $name } didn't finish within { $seconds } seconds
upgrade-exit-code-unknown = Can't get the exit code of the container { $name }
