## Texts the gateway and core-served web pages send to browsers, the App and bots (ADR-0043).
## One message per sentence; data (hosts, IDs, statuses, upstream reasons) goes in as parameters,
## untranslated. English is the base language, in Sentence case. Errors read like "Can't reach …".

# Shared across the gateway API, the MCP bus and the tools policy.
gateway-mcp-disabled = The home server doesn't have the MCP bus enabled

# Login page.
gateway-login-title = Sign In
gateway-login-heading = Sign in to Wonton
gateway-login-token-hint = Sign in with the token in <code>core/owner-token</code> under the home server's data directory; once the phone app pairs, it signs in by scanning a code instead.
gateway-login-failed = That token isn't right.
gateway-login-submit = Sign In

# Status page.
gateway-status-title = Status
gateway-status-logged-in = Signed in as { $user }
gateway-status-apps-heading = Apps
gateway-status-empty = No apps installed yet.
gateway-status-logout = Sign Out

# App states on the status page.
gateway-state-installing = Installing
gateway-state-running = Running
gateway-state-stopped = Stopped
gateway-state-degraded = Unhealthy: { $reason }
gateway-state-uninstalling = Uninstalling
gateway-state-updating = Updating

# Error page.
gateway-page-error-title = Something went wrong
gateway-page-error-detail = Details
gateway-not-found = There's no page here
gateway-cross-site-write = This home server only accepts changes made from its own pages

# Errors while dispatching by Host and on the plugin subdomains.
gateway-unknown-host = Unknown address { $host }
gateway-oidc-unconfigured = The home server has no identity provider configured
gateway-no-app-for-host = No app is installed for { $host }
gateway-no-webui = This app has no web page
gateway-app-not-ready = The app { $id } isn't ready yet
gateway-plugin-not-shared = { $id } isn't shared with family members
gateway-proxy-sso-failed = Can't get the sign-in credentials for the app { $id }: { $error }
gateway-cross-site-login = The gateway doesn't trust sign-in state on requests from another site; use the app's own page
gateway-login-required = Sign in to Wonton first
gateway-bot-web-permission = Apps on the plugin network cannot open Bot web pages or use browser sign-in
gateway-bot-web-storage = Can't read the Bot web page records; check the server's storage and try again

# Names shown when an upstream behind the gateway fails.
gateway-name-oidc = identity provider
gateway-name-models = model gateway

# Client entries behind the TLS entry container (ADR-0021).
gateway-entry-header-invalid = The entry gateway didn't label this request correctly
gateway-entry-closed = This entry is turned off; the owner can turn it on in the Wonton app
gateway-entry-public-denied = This path isn't available on the public address
gateway-entry-setup-pending = Finish the app's first-time setup on its web page first, then try again
gateway-entry-auth-unsupported = Signing in with a Wonton-issued password isn't supported for this app yet
gateway-entry-state-unavailable = Can't read this app's entry settings; the owner can check them in Wonton
gateway-entry-setup-closed = The app { $id } has no admin account any more, so its entry addresses were turned off to stop someone from claiming one. Reopen them in the Wonton app once the app is set up again.
gateway-entry-setup-timeout = Checking this app's first-time setup took too long
gateway-entry-target-unavailable = This app's entry must point to one shared service on its own app network

# SSO handoff and OIDC login and consent.
gateway-sso-invalid-next = That redirect address isn't valid
gateway-sso-local-only = Redirects can only go to this server's app addresses
gateway-sso-missing-code = The handoff code is missing
gateway-sso-code-invalid = The handoff code is invalid or expired; open the page again
gateway-sso-user-mismatch = The identity provider remembers a different user
gateway-proxy-sso-member-missing = Your account is no longer on this home server; ask the owner to invite you again
gateway-sso-consent-mismatch = The authorization request doesn't match the signed-in user

# Forwarding to upstreams (auth endpoints, model gateway).
gateway-upstream-unreachable = { $name } is temporarily unreachable: { $error }
gateway-upstream-timeout = { $name } isn't responding; try again later
gateway-invalid-path = The request path isn't valid

# Plugin management API.
gateway-invalid-body = The request body isn't right: { $error }
gateway-invalid-package = That isn't a plugin package: expected a tar, tar.gz, zip, or compose file
gateway-denied = This device isn't paired or has been removed; pair it again. Command-line calls need "Authorization: Bearer <token>"
gateway-cross-site-api = The app management API doesn't accept requests from other web pages
gateway-disk-space = Can't check disk space: { $error }
gateway-invalid-lan-choice = The request body isn't a local-network setting choice: { $error }
gateway-icon-not-found = No such image
gateway-icon-failed = Can't process the icon: { $error }

# Devices and pairing codes.
gateway-devices-not-found = This device isn't registered
gateway-devices-invalid-id = The request body should be {"{"}"endpoint_id": "<64 hex digits>"{"}"}
gateway-devices-invalid-addrs = The request body should be {"{"}"direct_addrs": ["IP:port"]{"}"}: { $error }
gateway-devices-already-paired = This server already has paired devices, so it won't issue a first-pairing code
gateway-devices-no-addresses = The tunnel isn't connected to a relay yet and the request gave no direct addresses, so the code has no reachable address

# Model gateway.
gateway-models-permission = The app { $id } hasn't requested models permission
gateway-plugin-network-only = This address is only for apps on the plugin network
gateway-models-down = The model gateway isn't running

# Onboarding.
gateway-onboarding-invalid-step = step must be profile or finished, not { $step }
gateway-onboarding-invalid-agent = primary_agent isn't an optional agent: { $agent }
gateway-onboarding-index-missing = The official index has no onboarding list

# Core one-click update.
gateway-upgrade-disabled = One-click update isn't enabled on this home server

# Pusher registration.
gateway-pushers-invalid-body = Can't read the pusher registration body: { $error }
gateway-pushers-invalid-json = The pusher registration body isn't JSON: { $error }
gateway-pushers-proxy-only = A pusher can only point at this home server's push proxy { $url }
gateway-pushers-kind-refused = Pushers of kind { $kind } aren't allowed
gateway-pushers-missing-kind = The pusher registration body is missing kind

# MCP bus and tools policy (owner's language; agents and bots read these).
gateway-tools-permission = The app { $id } hasn't requested tools permission
gateway-agent-post-only = This endpoint only accepts POST
gateway-body-unreadable = Can't read the request body: { $error }
gateway-body-invalid = The request body isn't valid: { $error }
gateway-bot-not-found = No such bot: { $caller }
gateway-bot-not-in-plugin = The app { $id } has no such bot: { $caller }
gateway-tool-not-found = The app { $id } has no tool { $tool }
gateway-tool-deletes = The tool { $tool } deletes or overwrites data, so it asks every time and can't be marked read-only
gateway-plugin-not-running = This app isn't running, or it doesn't declare any tools

# Confirmation answers from the App.
gateway-confirm-invalid-body = The request body should be {"{"}"answer": "approve" | "approve_wider" | "allow_window" | "reject"{"}"}: { $error }
gateway-no-allow-window = There's no active allow window

# Pairing (server → the phone that scanned the code; owner's language).
gateway-pair-unknown-code = This server never issued this pairing code
gateway-pair-code-used = This pairing code was already used; get a new one
gateway-pair-code-expired = The pairing code has expired; get a new one
gateway-pair-first-used = This server already has paired devices and the first-pairing code can't be reused; run sudo wonton pair on the server to issue a new code
gateway-pair-register-failed = The server couldn't register the device: { $error }
gateway-pair-role-unavailable = The role for this invitation is unavailable. Ask an admin for a new invitation.

# Owner viewing plugin secrets in the App.
gateway-owner-secrets-plugin-not-installed = The app { $id } isn't installed
gateway-owner-secrets-record-unreadable = The app { $id } record is unreadable, so its secrets can't be listed
gateway-owner-secrets-not-viewable = The app { $id } doesn't declare a viewable secret { $name }
gateway-owner-secrets-not-generated = The secret { $name } has no value yet; start the app once (or upgrade) to generate it again
gateway-owner-secrets-store-failed = Couldn't read the app secret: { $error }
gateway-owner-secrets-not-text = The secret { $name } isn't stored as text
gateway-owner-secrets-audit-unavailable = The audit log isn't wired on this server, so secret values can't be shown
gateway-plugin-scope-unsupported = This core does not support changing which bots can use a plugin. Upgrade the core first.

# MCP Apps 宿主页（#35）。
mcp-app-loading = Loading…
mcp-app-failed = Can't load the app: { $error }
mcp-app-page-failed = The app didn't load.
mcp-app-retry = Retry

# Friends opening a page over a share connection (ADR-0051).
gateway-share-identity = This server can't confirm who you are
gateway-share-required = The owner hasn't shared this app with you
gateway-share-revoked = The owner stopped sharing this app
gateway-share-expired = This share has expired
gateway-confirmation-gone = There is no such confirmation, or it ended more than a day ago
# 静态网页的数据存储（ADR-0080 第 6 节）
gateway-web-storage-undeclared = This app doesn't store data
gateway-web-storage-cross-site = Only this app's own pages can change its data
gateway-web-storage-not-ready = The app that keeps this app's data isn't running; try again later

# 第三方客户端和公网访问（ADR-0021、ADR-0089）。
clients-header-missing = The request is missing the X-Wonton-Clients header
clients-unauthenticated = Only the owner's paired devices can change this
clients-unavailable = The core isn't running in its container, so domain settings are unavailable
clients-domain-read-failed = Couldn't read the domain settings: { $error }
gateway-chains-disabled = The home server doesn't have the call chain ledger enabled

# agent_acp forwarding (ADR-0061, block 5): connect-time refusals and per-frame errors, returned to the consumer by category.
gateway-agent-acp-websocket-only = This entry only accepts WebSocket connections
gateway-agent-acp-path = agent_acp only serves /acp
gateway-agent-acp-initiator-bot-required = The connection must name the calling bot with X-Wonton-Bot
gateway-agent-acp-initiator-bot-unrecognized = {$bot} is not a bot of this app
gateway-agent-acp-on-behalf-of = The connection must say who it speaks for with X-Wonton-On-Behalf-Of
gateway-agent-acp-adopter-not-listened = {$bot} does not listen to the person this connection claims to speak for
gateway-agent-acp-target-not-listened = {$bot} does not listen to this person
gateway-agent-acp-no-bot = {$provider} has no usable bot yet; set up a bot first
gateway-agent-acp-multi-bot = {$provider} has several bots; pick one on its detail page first
gateway-agent-acp-token-missing = {$provider} has no provider token yet; reinstall it to get one
gateway-agent-acp-provider-unreachable = The agent runtime did not take over this connection: {$detail}
gateway-agent-acp-busy = This app already has the maximum number of concurrent agent connections; try again later
