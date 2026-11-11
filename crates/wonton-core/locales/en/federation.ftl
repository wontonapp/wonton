## Notice sent to the owner chat when the cross-server chat list (ADR-0050) fails to update (ADR-0043).
## The first paragraph is one plain sentence; $error carries the host path and raw error, details only.

fed-update-failed = The cross-server chat list was not updated, so messages with the other server may be temporarily missing. It is retried automatically the next time pairing changes or the home server restarts.
fed-update-failed-detail = Details: { $error }
