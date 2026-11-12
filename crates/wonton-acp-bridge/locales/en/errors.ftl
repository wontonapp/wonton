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
err-hop-limit = Several bots went back and forth here too many times, so the gateway held { $count } of my messages. They have been sent now.

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

# 调用链（ADR-0061 第 10 节）：核心不给链时这一轮不发给 Agent，房间里如实说明。
chain-limit-depth = This conversation chain has reached the Agent depth limit, so this turn didn't start. To continue, tap "grant more" for the chain in Wonton.
chain-limit-rounds = This conversation chain has run out of turns, so this turn didn't start. To continue, tap "grant more" for the chain in Wonton.
chain-limit-cost = This conversation chain has reached its spending limit, so this turn didn't start. To continue, tap "grant more" for the chain in Wonton.
chain-limit-other = This conversation chain has reached a limit, so this turn didn't start. To continue, tap "grant more" for the chain in Wonton.
chain-stopped = This conversation chain was stopped, so this turn didn't start. To continue, just say it again as a new message.
err-chain-refused = the chain service declined this turn: { $message }
chain-unreachable = talking to the chain service failed: { $error }
chain-invalid = the chain service response isn't valid (HTTP { $status }): { $error }
chain-client = building the chain HTTP client failed: { $error }
chain-url = WONTON_MCP_URL isn't valid: { $error }
outbox-client = building the outbox HTTP client failed: { $error }
