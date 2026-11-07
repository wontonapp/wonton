## Errors the bridge posts into chat. A category prefix comes first; the detail (upstream text,
## I/O error) is a parameter and stays untranslated. Prefixes and the detail sentences below
## carry no trailing period, since the detail fills in after the prefix.

err-auth-required = The agent needs a sign-in: send /wonton login to see the available sign-in methods.
err-not-ready = Agent not ready: { $message }
err-transport = Transport unavailable: { $message }
err-session = ACP session unavailable: { $message }
err-upstream = Upstream error: { $message }
err-invalid = Invalid input: { $message }
err-confirmation = Confirmation card unavailable: { $message }
err-busy = Busy: { $message }
err-matrix = Matrix error: { $message }

# Details written by the bridge itself; they appear inside the prefixes above.
acp-no-http-mcp = the agent doesn't advertise HTTP MCP
acp-no-session-id = session/new returned no sessionId
acp-no-steering = the agent doesn't advertise _session/steering
acp-connection-closed = the ACP connection closed
acp-turn-timeout = the ACP turn timed out; cancellation requested
acp-output-limit = the ACP reply passed the output limit; cancellation requested
acp-encode-failed = couldn't encode the ACP request: { $error }
acp-request-failed = the ACP request failed
acp-frame-too-large = the ACP request passed the frame limit
acp-restore-unsupported = the agent doesn't support resuming an ACP session

# Confirmation card requests (the bot's permission prompts).
perm-mcp-url = WONTON_MCP_URL isn't valid: { $error }
perm-client = couldn't build the confirmation card HTTP client: { $error }
perm-broker-connect = can't reach the confirmation card endpoint: { $error }
perm-broker-read = couldn't read the confirmation card response: { $error }
perm-broker-timeout = the confirmation card request timed out
perm-broker-invalid = the confirmation card response isn't valid: { $error }
perm-tool-kind = ACP tool kind: { $kind }
perm-missing-options = the ACP permission request is missing options
perm-duplicate-option-id = the ACP permission request repeats an optionId
perm-invalid-kind = the ACP permission option kind isn't valid: { $error }
perm-empty-options = the ACP permission request lists no options
perm-missing-field = the ACP permission request is missing { $field }
perm-redacted = [hidden]
