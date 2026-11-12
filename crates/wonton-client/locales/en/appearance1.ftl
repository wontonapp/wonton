# Appearance settings errors. appearance-settings-unreadable is shown as AppearanceProblem.message on the appearance page;
# the others are reasons inside it, or the reason of a CoreError::Storage.
# $path is the settings file's path, $reason is the original parser or I/O error.
appearance-settings-unreadable = Can't read the appearance settings, so the defaults are in use for now: { $reason }

# $path is the settings file's path, $reason is the original I/O error.
appearance-settings-write-failed = Can't write the appearance settings file { $path }: { $reason }
