## Notices in the Wonton chat when the federation list could not be updated (ADR-0050, ADR-0043).
## The path and raw error stay in the core's log.

federation-update-invalid-name = The list of servers allowed to talk to yours was not updated: it contains an invalid server name. It will retry the next time a pairing or block changes, or when the server restarts.
federation-update-write-failed = The list of servers allowed to talk to yours could not be saved on this server. It will retry the next time a pairing changes or the server restarts.
federation-update-reload-failed = The list of servers allowed to talk to yours was saved, but the chat service has not picked it up yet. It will retry the next time a pairing changes or the server restarts.
