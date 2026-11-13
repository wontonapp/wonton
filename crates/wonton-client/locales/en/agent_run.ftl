# The Agent chain list and the chain actions (crates/wonton-client/src/agent_chains/).

# Shown when a chain list fetch or chain action needs the home server but the app isn't connected to it.
agent-api-not-connected = Try again once you're connected to your home server.

# $status is the unchanged HTTP status code.
agent-api-status = Your home server returned HTTP { $status }

# $reason is the JSON parser's original error.
agent-api-unreadable = Can't read your home server's response: { $reason }

# $reason is the HTTP client's original error.
agent-api-client = Can't create an HTTP client: { $reason }
