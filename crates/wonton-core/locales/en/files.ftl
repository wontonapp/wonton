## Texts of the file service, shown to the App and to bots and agents (ADR-0043). One message per
## sentence; data (file IDs, sizes, plugin IDs, upstream reasons) goes in as parameters, untranslated.
## English is the base language, in Sentence case. Errors read like "Can't read …".

files-not-found = No such file
files-moved-or-deleted = The file was moved or deleted
files-not-shared = This file isn't shared with you
files-share-expired = This file ID's share has expired; look the file up again
files-changed = The file changed after it was registered, so what you saw may not be the current content; look it up again
files-share-not-authorized = The app { $plugin } isn't authorized for the shared folder { $share }
files-mcp-gone = The app { $plugin } is running but no longer provides MCP service, so this resource can't be read
files-plugin-unavailable = The app { $plugin } it came from was uninstalled or isn't running
files-notify-no-matrix = The home server isn't connected to Matrix (WONTON_MATRIX_URL), so it can't send to the "Wonton" chat
files-matrix-send-failed = Can't send to the "Wonton" chat: { $error }
files-upstream-not-own = { $service } isn't a service of the app { $plugin }

# Previews and thumbnails.
files-too-big-for-thumbnail = The file is over { $limit } MiB, so no thumbnail is made
files-pdf-unreadable = Can't parse this PDF (it may be encrypted or damaged), so its content can't be read
files-pdf-no-text = This PDF has no text layer (a scan or images only); it has { $pages } pages, and the charts and pictures inside it can't be read
files-too-big-for-parsing = The file is over { $limit } MiB, so its contents can't be extracted
files-video-unsupported = This video's container can't be read (only MP4, M4V and MOV are); download or share it instead
files-video-unreadable = This video doesn't parse (its container or structure isn't recognized), so its metadata can't be read
files-audio-unreadable = Audio can't be read here; download or share it instead
files-thumbnail-timeout = The original file didn't finish reading within { $after }
files-idle-limited = The app's response had no new bytes for { $idle }
files-declared-size-over-limit = The size the app declared exceeds the resources/read limit ({ $limit } MiB)
files-not-text-nul = { $name } isn't a text file (its content has NUL bytes), so its content can't be read: binary files like PDFs can only be downloaded or shared.
files-not-text-utf8 = { $name } isn't a text file (its content isn't valid UTF-8), so its content can't be read. Binary files like PDFs can only be downloaded or shared.
files-no-preview = This type has no preview or thumbnail
files-preview-failed = Can't make a preview or thumbnail: { $why }
files-decode-busy = Image decoding is busy
files-view-busy = This file is already being read as fast as it can be; try again in a moment
files-read-busy = Image reading is busy

# Saving, uploading and deleting.
files-stage-no-space = These { $total } bytes don't fit; the disk keeps 5% free
files-delete-transfers-only = The home server only deletes files in the shared folder { $folder }
files-size-mismatch = The uploaded data is { $actual } bytes, but the creation declared { $size } bytes
files-no-hard-links = { $name }: the filesystem holding the shared folder { $folder } doesn't support hard links
files-name-taken = { $preferred } already has { $count } files with the same name
files-short-read = Only { $copied } bytes were read, expected { $size }
files-upload-disk-full = The disk can't fit this { $size }-byte file ({ $percent }% must stay free, and inodes can't run out either)
files-upload-leftover = The disk has only { $free } bytes left, and this upload needs { $left } more
files-upload-quota-full = The disk quota is full, this upload does not fit
files-upload-inodes-full = The disk is out of inodes, this upload does not fit
files-upload-record-label = pending upload record
files-upload-record-unwritable = The pending upload record couldn't be written, so the file isn't placed yet
files-path-collision = This path is already a different file: the file ID was removed, the file itself wasn't touched
files-residual-undeletable = Can't clean up what's left of this upload, so the file wasn't deleted

# Read slots for app resources.
files-plugin-read-overrun = The app { $plugin } has a read that has run past { $seconds } seconds without finishing
files-slots-full = The app { $plugin } has used up its read slots; try again later
files-plugin-slots-exhausted = All read slots for app resources are busy

# Reading resources from apps.
files-resource-invalid-url = The internal address isn't valid: { $error }
files-resource-http-only = The internal address can only use http
files-resource-no-credentials = The internal address can't carry a username or password
files-resource-host = The internal address's host must be the app's service name, not an IP address
files-resource-unreachable = Can't reach the app's service: { $why }
files-resource-redirect = The app's internal address returned a redirect ({ $status }), which the home server doesn't follow
files-upstream-status = The app returned { $status }: { $detail }
files-upstream-read-failed = Can't read the app's response: { $error }

# Trash slots for deleting files.
files-trash-foreign-slot = The trash slot { $name } wasn't created by the home server (owner { $owner }, mode { $mode }, expected { $euid }, 700); restart the home server before deleting again
files-trash-placeholder-missing = The placeholder file for trash slot { $index } is gone; restart the home server before deleting again
files-trash-full = All trash slots in { $folder } are in use; restart the home server to recover the files inside before deleting again

# Shared folders.
files-check-share = There's no shared folder { $share }; use wonton__list_shares to see which ones exist

# File API over the gateway.
files-service-disabled = The home server doesn't have the file service enabled
files-device-only = The file API is only for paired devices over the tunnel, and it needs { $header }: 1
files-stream-shortened = The file got shorter while downloading
files-invalid-room-body = The request body should be {"{"}"room": …{"}"}: { $error }

# Uploads through tusd.
files-tusd-unconfigured = The home server has no tusd configured (WONTON_TUSD_URL)
files-tusd-required = The upload API needs Tus-Resumable: 1.0.0
files-tusd-timeout = tusd isn't responding; try again later
files-tusd-gone = tusd no longer has this upload's data; upload it again
files-upload-disconnected = The request that created this upload has disconnected
files-upload-length-required = The upload must declare its length at creation (Upload-Length)
files-upload-filename-required = The upload must carry filename in Upload-Metadata
files-upload-finished = This upload is already complete; to delete the file use DELETE /_wonton/files/{ $id }
files-upload-other-device = This upload was created by another device
files-upload-stale = This upload has had no progress for a long time and was cleaned up; upload it again
files-hook-tusd-only = The file hooks are only for tusd
files-hook-not-from-gateway = This request didn't come through the gateway
