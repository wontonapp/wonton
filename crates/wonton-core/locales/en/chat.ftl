## Texts the home server sends into chat (ADR-0043): confirmation cards and their outcome
## lines, the plain-text card body for other Matrix clients, the kick reason when a bot's
## room can't be handed over, and help-tool texts the agent relays to the owner. One
## message per sentence; data (bot names, app titles, versions, paths, upstream error
## text) goes in as parameters, untranslated. English is the base language, in Sentence
## case; errors read like "Can't …". Buttons follow Apple's Title Case.

# Buttons on confirmation cards.
chat-choice-install = Install
chat-choice-deny-install = Don't Install
chat-choice-update = Update
chat-choice-deny-update = Don't Update
chat-choice-uninstall = Uninstall
chat-choice-cancel = Cancel
chat-choice-allow = Allow
chat-choice-deny = Deny

# Window choice on tool-approval cards: further calls run without asking for a while.
# Parameters: a whole number of minutes or seconds.
chat-card-window-minutes =
    { $minutes ->
        [one] Allow for the next minute
       *[other] Allow for the next { $minutes } minutes
    }
chat-card-window-seconds =
    { $secs ->
        [one] Allow for the next second
       *[other] Allow for the next { $secs } seconds
    }

# Lines on install and update cards. $requester is the bot's name, $title the app's name.
chat-card-image = Service { $service } runs image { $image }
chat-card-ports-added = New local network ports: { $ports }
chat-card-gpu-added = New GPU services: { $services }
chat-card-version-official = Version { $version }, from the official index
chat-card-version = Version { $version }
chat-card-update-from = Updating from version { $from } to { $to }
chat-card-no-permissions = No extra permissions needed
chat-card-permissions-header = Permissions to grant:
chat-card-agent-source = From your agent · { $requester }, not reviewed: the agent wrote the code, you decide what to trust
chat-card-agent-ask-again = Future changes by { $requester } won't ask again, unless they ask for more permissions
chat-card-update-header-added = What this update adds:
chat-card-update-header-risky = This update asks for high-risk permissions:
chat-card-update-header-ask-every = Permissions didn't widen; you turned on "Ask every time":
chat-card-update-header-adopt = You installed this app yourself; approving hands it to { $requester }:
chat-card-update-adopt-replace = Approving replaces the version you installed; data is kept, failed updates roll back

# Card titles. $requester is the bot's name; $title, $plugin and $tool are app-provided names.
chat-card-install-title = { $requester } wants to install { $title }
chat-card-update-title = { $requester } wants to update { $title }
chat-card-uninstall-title = { $requester } wants to uninstall { $title }
chat-card-permission-enable-title = { $requester } wants to turn on { $permission } for { $title }
chat-card-permission-disable-title = { $requester } wants to turn off { $permission } for { $title }
chat-card-permission-off-loss = What is lost: { $loss }
chat-card-permission-root = Once on, this app can control the whole host; actions run as root.
chat-choice-permission-enable = Turn on
chat-choice-permission-disable = Turn off
chat-permission-done = Updated the permission for { $title }
chat-permission-failed = Changing the permission for { $id } failed: { $error }
chat-card-tool-title = { $requester } wants to use { $tool } from { $plugin }

# Title of an agent-permission card. $title is what the agent called its request.
chat-card-agent-title = { $requester }: { $title }

# The extra confirmation for the highest permission level (ADR-0036).
chat-card-highest-title = { $title } can control the whole machine
chat-card-highest-line = Its code runs as root on this machine
chat-card-highest-confirm = I understand this app can control the whole machine

# Uninstall card: what happens to the app's data.
chat-card-uninstall-purge = Its data will be deleted too; this can't be undone
chat-card-uninstall-keep = Keep its data; it's still there if you reinstall later

# Tool-call card lines. $args is the raw JSON arguments, truncated when too long.
chat-card-tool-data = This tool changes data
chat-card-tool-root = This runs as root on this machine
chat-card-args = Arguments: { $args }

# Bot file-save card. $name and $size describe one file; $total is the combined size.
chat-card-save-file = { $name } ({ $size })
chat-card-save-more =
    { $count ->
        [one] …and 1 more file
       *[other] …and { $count } more files
    }
chat-card-save-total = Total: { $total }
chat-card-save-unreviewed = Provided by a bot; content isn't reviewed
chat-card-save-allow = Allow saving into { $folder }
chat-card-save-title =
    { $count ->
        [one] { $requester } wants to save 1 file into { $folder }
       *[other] { $requester } wants to save { $count } files into { $folder }
    }

# The card body shown to other Matrix clients. $message is the outcome line written when
# the card was answered, in the owner's language.
chat-card-state-pending = Approve it in Wonton.
chat-card-state-running = Running…
chat-card-state-succeeded = Done: { $message }
chat-card-state-failed = Didn't go through: { $message }
chat-card-state-uncertain = It's unclear whether it worked: { $message }
chat-card-state-rejected = Denied, not carried out
chat-card-state-expired = Expired, not carried out

# Line under the card body about the approval window: still open, or how it ended. $count
# is how many calls the window let through after the first one; 0 means unknown or none.
chat-window-active = Approved until the window ends; no more prompts before that.
chat-window-ended =
    { $reason ->
        [expired]
            { $count ->
                [0] The window has expired.
               *[other] The window has expired; { $count ->
                    [one] 1 further action was approved.
                   *[other] { $count } further actions were approved.
                }
            }
        [stopped]
            { $count ->
                [0] The window was stopped.
               *[other] The window was stopped; { $count ->
                    [one] 1 further action was approved.
                   *[other] { $count } further actions were approved.
                }
            }
        [restarted] The home server restarted; the window has ended.
       *[other]
            { $count ->
                [0] The window has ended.
               *[other] The window has ended; { $count ->
                    [one] 1 further action was approved.
                   *[other] { $count } further actions were approved.
                }
            }
    }

# Outcome lines that end up on cards after the owner answers.
chat-install-done = Installed { $title }
chat-install-done-url = Installed { $title }. Its web page: { $url }
chat-install-failed = Can't install ({ $kind }): { $error }
chat-install-failed-id = Can't install { $id } ({ $kind }): { $error }
chat-install-note-writer = Couldn't record which agent wrote the app ({ $error }); its next update will fail
chat-outcome-with-note = { $message }. { $note }
chat-update-done = Updated to { $version }
chat-update-done-url = Updated to { $version }. Its web page: { $url }
chat-update-failed = Can't update { $id } ({ $kind }): { $error }
chat-uninstall-done-purge = Uninstalled { $title }; its data was deleted
chat-uninstall-done-keep = Uninstalled { $title }; its data was kept
chat-uninstall-failed = Can't uninstall { $id } ({ $kind }): { $error }
chat-tool-ran = The tool ran
chat-tool-failed = The tool reported a failure
chat-save-done = Saved into { $folder }
chat-save-done-note = Saved into { $folder }. { $note }
chat-save-partial = Saved { $saved }; the rest failed: { $failure }
chat-save-note-grant = Couldn't record the permission ({ $error }); you'll be asked again next time
chat-agent-chose = Chose “{ $choice }”
chat-agent-abandoned = The agent stopped waiting; this request is void
chat-superseded = Superseded by a newer request
chat-uncertain-restart = The home server restarted; whether this finished is unknown

# Kick reason, shown in the room timeline when a bot's room can't be handed over.
chat-kick-reason = Wonton couldn't finish setting up this chat

# Help tool. The agent reads these and answers the owner with them. $details carries the
# upstream reason; $required and $core are version numbers.
chat-help-docs-unavailable = This home server has no help docs (nothing in { $dir } and none fetched)
chat-help-refresh-failed = Couldn't fetch newer help docs: { $details }
chat-help-refresh-needs-core = There are newer help docs, but they need the home server to be { $required } or later; this one is { $core }. Update the server first
chat-help-docs-age = { $problem } This copy is { $days ->
    [one] 1 day
   *[other] { $days } days
} old
chat-help-topic-missing = No topic “{ $topic }”. Available topics: { $topics }
chat-help-page-needs-core = This page covers features that need the home server to be { $required } or later; this one is { $core }. Update the server first

chat-mail-convert-failed = The HTML body of this email could not be converted to text. The original is attached as message.html.
chat-mail-unknown-source = An app email came from an unknown source or an app without mail permission. It wasn't delivered to anyone.
chat-mail-recipients-failed = Couldn't update who can receive app mail: { $error }. Wonton will try again when household membership changes or the server restarts.
chat-notify-rate-limited = { $plugin } is sending notifications and mail too often. They are paused for about { $seconds } seconds.
