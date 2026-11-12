## Texts shown to the app and on chat cards during server pairing (ADR-0050, ADR-0043).

peers-endpoint-detail = endpoint { $id }, last error: { $error }
peers-endpoint = endpoint { $id }
peers-request-title = { $owner }'s server wants to connect to yours
peers-request-server = Their server: { $server }
peers-connect = Connect
peers-reject = Reject
peers-connected = Connected
peers-error-code-expired = The pairing code has expired; issue a new one
peers-error-code-invalid = The code is not a valid Wonton server pairing code, or it was altered
peers-error-peer-unreachable = Cannot reach the other server; try again later
peers-error-peer-rejected = The other side declined the pairing
peers-error-already-paired = The two servers are already paired
peers-error-self-pairing = This is your own server's code
peers-error-not-paired = This server is not paired with yours
peers-error-unauthenticated = Pair this device with your server first, then open this from the app
peers-error-transport = The pairing did not complete; try again later
peers-error-matrix-unavailable = This server is not connected to Matrix, so pairing is unavailable
peers-error-not-paired-remove = This server is not paired
peers-error-code-revoked = This pairing code is no longer valid; ask for a new one
peers-error-too-many-requests = The other server cannot take pairing requests right now; try again later
peers-error-request-not-found = This request is no longer there
