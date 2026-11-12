## Store, sources, network settings, model providers and app updates: text the home server
## sends to the app (ADR-0043). One complete sentence per message; data (paths, ports, image
## names, IDs, addresses, Docker's own words) is passed as arguments, never translated.

# ── Common ───────────────────────────────────────────────────────────────────

## with_note joins the original message and an added note into one sentence.
plugin-settings-note-appended = { $message } ({ $note })

# ── Missing email (owner.rs, used by installs and the web port) ──────────────

plugin-settings-email-missing = You haven't set your email yet. Add it in Wonton's settings and try again
plugin-settings-email-needed = This app needs your email. Set it before installing

# ── Risks of the web page's local network port (lan/webui.rs, shown in details) ──

plugin-settings-lan-webui-risk-impersonate = Devices on your local network, and apps on this computer that can reach the internet, can open it as you without a password
plugin-settings-lan-webui-risk-register = Before an administrator registers, devices on your local network or apps on this computer could register first
plugin-settings-lan-webui-risk-client = Devices on your local network can connect with the username and password. Changing the password in the app's own pages doesn't update it here

# ── Manifest lan and lan.client (lan/mod.rs, lan/client.rs) ──────────────────

plugin-settings-lan-client-prefix = lan.client: { $message }
plugin-settings-lan-client-address-host = address must contain { $host }
plugin-settings-lan-client-template = { $name } can't be used here
plugin-settings-lan-need-ports = network.lan is requested, so lan.ports must list the ports to open
plugin-settings-lan-need-permission = lan is declared, so the network.lan permission must be requested too
plugin-settings-lan-no-matrix-bot = network.lan can't be combined with matrix.bot: a changed address rebuilds the containers, and the home server doesn't keep the bot's access token
plugin-settings-lan-ports-empty = lan.ports needs at least one entry
plugin-settings-lan-port-zero = lan.ports can't use port 0
plugin-settings-lan-port-duplicate = { $port } appears more than once in lan.ports
plugin-settings-lan-port-mcp = The mcp port is only reached through the home server and can't be listed in lan.ports ({ $port } of { $service })
plugin-settings-lan-network-none = The lan.ports service can't set network_mode: none

# ── lan.client connection details (lan/client.rs, shown to the owner) ────────

plugin-settings-lan-client-secret-missing = This app has no stored secrets
plugin-settings-lan-client-container = Can't find container { $container }: { $error }
plugin-settings-lan-client-not-published = The port isn't published on any local network address

# ── Local network detection and choice (lan/detect.rs, select.rs, monitor.rs) ──

plugin-settings-lan-fixed-by-env = Local network addresses are set by the home server's WONTON_LAN_ADDRESSES and can't be changed in settings
plugin-settings-lan-announce-by-env = The addresses phones use are set by the home server's WONTON_ANNOUNCE_ADDRESSES and can't be changed in settings
plugin-settings-lan-desktop-vm = The home server runs in a virtual machine on a computer, so phones can't reach the virtual machine's address. The launcher has to set WONTON_ANNOUNCE_ADDRESSES to the computer's local network address. Kernel: { $evidence }
plugin-settings-lan-announced = announced
plugin-settings-lan-fixed-missing = { $ip } from WONTON_LAN_ADDRESSES isn't on the host computer
plugin-settings-lan-detect-failed = Can't detect the network interfaces of the host computer: { $failure }
plugin-settings-lan-detect-error = Error while detecting the host computer's network interfaces: { $error }
plugin-settings-lan-detect-core-image = Can't find the image of home server container { $container }
plugin-settings-lan-detect-exit = The detection container exited with code { $code }: { $reason }
plugin-settings-lan-detect-output = Can't read the detection container's output: { $error }
plugin-settings-lan-detect-timeout = Container { $container } didn't finish within { $seconds } seconds
plugin-settings-lan-detect-no-exit-code = Docker didn't report an exit code for container { $container }
plugin-settings-lan-listen-error = Error while checking which ports are in use: { $error }
plugin-settings-lan-choice-unknown = This version of the home server doesn't recognize the saved local network settings. Choose them again
plugin-settings-lan-no-default-route = The host computer has no IPv4 default route
plugin-settings-lan-default-virtual = The default route goes through { $name } ({ $why }), so no address is chosen automatically
plugin-settings-lan-default-no-private = The default-route interface { $name } has no private IPv4 address
plugin-settings-lan-interface-gone = The chosen interface { $name } is no longer on the host computer, or has no private address
plugin-settings-lan-interface-no-private = The chosen interface { $name } has no private IPv4 address
plugin-settings-lan-address-gone = The chosen address { $address } is no longer on the host computer
plugin-settings-lan-no-selection = No interface or address is selected
plugin-settings-lan-choice-unrecognized = This choice isn't recognized
plugin-settings-lan-interface-not-selectable = { $name } isn't an interface that can be chosen on the host computer
plugin-settings-lan-address-not-selectable = { $address } isn't an address that can be chosen on the host computer
plugin-settings-lan-kind-tunnel = tunnel interface, such as Tailscale
plugin-settings-lan-kind-wireguard = WireGuard interface
plugin-settings-lan-kind-interface = interface
plugin-settings-lan-bridge-or-veth = a Docker bridge or veth
plugin-settings-lan-bound = { $ip } on { $interface }
plugin-settings-lan-select-failed = Can't pick a local network address: { $reason }
plugin-settings-lan-lookup-failed = Can't find where the local network ports are published: { $error }
plugin-settings-lan-rebuild-failed = Can't rebuild the app for the new local network address: { $error }
plugin-settings-lan-check-failed = Can't check the app: { $error }

# ── Local network ports in the install preview (lan/grant.rs) ────────────────

plugin-settings-lan-port-line = Local network port: { $key } of { $service }, published on { $place }
plugin-settings-lan-port-line-plain = Local network port: { $key } of { $service }
plugin-settings-lan-direct-notice = Devices on your local network connect to these ports directly, without signing in to the home server; the app verifies who they are itself
plugin-settings-lan-conflict-taken = Local network port { $key } is already used by app { $id }
plugin-settings-lan-note-reselect = you can choose a different interface or address in the local network settings

# ── The web page's local network port switch (lan/webui.rs) ──────────────────

plugin-settings-lan-webui-no-switch = App { $id } has no lan_webui switch: its local network ports don't need one
plugin-settings-lan-webui-busy = App { $id } is installing, uninstalling, or rebuilding. Try again when it finishes
plugin-settings-lan-webui-close-failed = Can't close the web page's local network port: { $error }
plugin-settings-lan-webui-rollback-failed = Rolling back after the switch failed also failed: { $error }

# ── Full sentences for install errors (construction sites of InstallError variants) ──

plugin-settings-not-installed = App { $id } isn't installed
plugin-settings-permission-not-granted = These permissions need your confirmation before installing: { $permissions }
plugin-settings-core-upgrading = The home server is updating. Try again when it finishes
plugin-settings-bot-matrix-unavailable = The home server isn't connected to Matrix (WONTON_MATRIX_URL), so apps declaring matrix.bot can't be installed
plugin-settings-note-revoke-device-failed = Can't revoke the device created for installation: { $error }
plugin-settings-note-migration-containers-gone = The old containers are already gone while moving the network. Retry this rebuild, or reinstall the app (keeping its data) to recover
plugin-settings-note-registry-mirror = you can turn on a download mirror in settings
plugin-settings-upgrade-rolled-back = Can't update to { $new }; rolled back to { $version }
plugin-settings-upgrade-rollback-failed = Can't update to { $new }; rolling back to { $version } also failed: { $error }
plugin-settings-upgrade-cleanup-unfinished = Cleanup didn't finish, there may be leftovers: { $failures }
plugin-settings-upgrade-cleanup-model-credentials = Model gateway credentials: { $error }

# ── Internet proxy (egress/) ─────────────────────────────────────────────────

plugin-settings-egress-plugin-and-url = An Internet proxy address can't be set while a proxy app is in use
plugin-settings-egress-restarts-changed = The apps to restart changed since you confirmed. Confirm again
plugin-settings-egress-cidr-invalid = Bypass range { $cidr } isn't valid: { $error }
plugin-settings-egress-cidr-overlaps-network = Bypass range { $cidr } overlaps Docker network { $network } ({ $subnet }): allowing it would let the proxy container reach other apps directly
plugin-settings-egress-env-locked = The proxy address is set by WONTON_EGRESS_PROXY and can't be changed here: change the settings in the install directory and run the install script again
plugin-settings-egress-address-invalid = The proxy address isn't valid: { $error }
plugin-settings-egress-address-scheme = Proxy scheme { $scheme } isn't supported: use http, https, or socks5
plugin-settings-egress-address-no-host = The proxy address has no host name
plugin-settings-egress-address-no-path = The proxy address is only scheme, host, and port, without a path or query
plugin-settings-egress-address-utf8 = The username or password in the proxy address isn't valid UTF-8: { $error }
plugin-settings-egress-no-response = The proxy at { $address } didn't respond for { $seconds } seconds: the host firewall must allow Docker's ranges to this port
plugin-settings-egress-refused = The proxy at { $address } refused the connection: nothing is listening on the port, or it only listens on the loopback address
plugin-settings-egress-unreachable = Can't reach the proxy at { $address }: { $error }
plugin-settings-egress-upstream-failed = The proxy itself works, but it can't reach the internet: { $error }
plugin-settings-egress-rejected = The proxy didn't let this step through: { $error }
plugin-settings-egress-disconnected = The proxy at { $address } closed the connection: { $error }
plugin-settings-egress-not-http = The proxy at { $address } didn't answer with HTTP
plugin-settings-egress-auth-407 = The proxy at { $address } answered 407: it needs a username and password, or they're wrong
plugin-settings-egress-status = The proxy at { $address } answered { $code }
plugin-settings-egress-relay-not-running = The proxy container isn't running
plugin-settings-egress-relay-detail = The proxy is reachable directly but not through the proxy container: { $detail }. Check the logs of container { $container }
plugin-settings-egress-diagnose-exit = The diagnose container exited with code { $code }: { $reason }
plugin-settings-egress-provider-missing = No app that can act as the Internet proxy is installed
plugin-settings-egress-provider-manifest = Can't read the declaration of proxy app { $id }
plugin-settings-egress-provider-conflict = Proxy app { $id } is already installed: there can only be one at a time
plugin-settings-egress-provider-not-running = The proxy app isn't running: { $detail }
plugin-settings-egress-relay-cidr-overlap = Bypass range { $cidr } overlaps the Docker network the proxy container is on, so it wasn't allowed: allowing it would let it reach other apps directly
plugin-settings-egress-token-unreadable = Service { $service } put the Matrix token in command or configs, so it can't be read back
plugin-settings-egress-token-reinstall = Reinstalling the app (keeping its data) issues a new device, and the bot has to rebuild its encrypted identity
plugin-settings-egress-token-read-back = Can't read the bot's token back: { $reason }. { $reinstall }

plugin-settings-egress-bot-record-missing = This app has no bot record
plugin-settings-egress-old-container-gone = The old container { $container } is gone
plugin-settings-egress-token-not-in-env = The old container { $container } doesn't carry the token in environment variable { $variable }
plugin-settings-egress-token-mismatch = The old containers don't agree on the token
plugin-settings-egress-token-not-used = No service in the old version used the token

# ── Download mirror (registry_mirror/) ───────────────────────────────────────

plugin-settings-mirror-address-invalid = Download mirror address { $address } isn't valid: use only a host name and an optional port, without a scheme, path, or credentials
plugin-settings-mirror-policy-never = Image { $image } has pull_policy: never and isn't present locally
plugin-settings-mirror-pull-failed = Mirror { $mirror }: { $error }. Test the mirrors in settings and pick another one
plugin-settings-mirror-digest-mismatch = The local image's RepoDigests don't include the required digest { $digest }; refusing to use it
plugin-settings-mirror-probe-timeout = The whole test timed out ({ $ms } ms): { $error }
plugin-settings-mirror-no-challenge = No valid Bearer WWW-Authenticate challenge
plugin-settings-mirror-token-request-rejected = The anonymous token request was rejected
plugin-settings-mirror-token-response-invalid = The anonymous token response isn't valid: { $error }
plugin-settings-mirror-token-missing = The anonymous token response has no token/access_token
plugin-settings-mirror-manifest-rejected = The manifest request was rejected
plugin-settings-mirror-manifest-digest = Manifest digest mismatch: expected { $digest }, got { $actual }
plugin-settings-mirror-status = HTTP { $status }: { $detail }

# ── Store and sources (storefront/, store.rs, store_info.rs) ─────────────────

plugin-settings-store-source-unreadable = The added sources can't be read: { $why }
plugin-settings-store-source-not-added = { $address } was never added
plugin-settings-store-source-limit = At most { $max } sources can be added. Remove the ones you don't use first
plugin-settings-store-source-save-failed = Can't save the added sources: { $error }
plugin-settings-store-source-in-official = { $repo } is already in the Store
plugin-settings-store-source-neither = The root of { $repo } has neither { $compose } nor { $index }: it's neither an app nor an index
plugin-settings-store-source-already-added = { $address } is already added
plugin-settings-store-source-index-missing = { $provider } has no index file
plugin-settings-store-source-not-an-index = { $address } isn't an index: { $error }
plugin-settings-store-source-plugin-missing = Index { $provider } has no { $id }
plugin-settings-store-community-partial = Some couldn't be read: { $missing }
plugin-settings-store-community-page-failed = Page { $page } of the search results ({ $error })
plugin-settings-store-community-unavailable = Can't fetch community apps: { $error }
plugin-settings-store-system-app = System apps only come preinstalled with the home server
plugin-settings-store-permission-unsupported = The home server doesn't support the { $permission } permission yet
plugin-settings-store-index-newer = The index format is { $format }; this version of the home server only understands { $supported }. Update the home server to use the new index
plugin-settings-store-index-invalid = Can't read the index: { $error }
plugin-settings-store-index-compose-digest = The sha256 of { $file } doesn't match what files records
plugin-settings-store-index-version-mismatch = version { $version } doesn't match the manifest's version { $manifest }
plugin-settings-store-index-source-duplicate = source appears more than once
plugin-settings-store-index-id-duplicate = App ID { $id } appears more than once
plugin-settings-store-index-source-with-version = source can't carry a version
plugin-settings-store-index-commit-length = commit must be a 40-character commit hash
plugin-settings-store-index-file-digest = The { $path } of { $repo } doesn't match the sha256 recorded in the index
plugin-settings-store-index-file-missing = The index has no { $path } for { $source }
plugin-settings-store-index-blob-invalid-sha = The sha256 in the official index isn't valid: { $sha256 }
plugin-settings-store-index-blob-missing = The index directory has no { $path }: { $error }
plugin-settings-store-record-id-mismatch = The record's ID is { $record }, which doesn't match the file name { $file }

# ── Store listing details (store_info.rs, shown when installing official apps) ──

plugin-settings-store-info-spec-missing = The manifest needs spec, like spec: "{ $version }"
plugin-settings-store-info-spec-string = spec must be a string, like "{ $version }" (quote it in YAML, or it reads as a number)
plugin-settings-store-info-spec-version = spec { $text } isn't a spec version (like "{ $version }")
plugin-settings-store-info-author = author.name can't be empty and is limited to { $max } characters
plugin-settings-store-info-license = license { $license } isn't an SPDX license expression (like MIT, AGPL-3.0-only): { $error }
plugin-settings-store-info-screenshots-count = screenshots needs 1 to { $max } entries
plugin-settings-store-info-screenshot-format = Screenshot { $raw } must be PNG or JPEG
plugin-settings-store-info-screenshot-duplicate = Screenshot { $raw } appears more than once
plugin-settings-store-info-tags-count = tags allows at most { $max } entries
plugin-settings-store-info-tag-length = Tag { $tag } must be 1 to { $max } characters, without leading or trailing spaces
plugin-settings-store-info-tag-control = Tag { $tag } contains control characters
plugin-settings-store-info-tag-duplicate = Tag { $tag } appears more than once
plugin-settings-store-info-field-length = { $field }.{ $lang } exceeds { $max } characters
plugin-settings-store-info-https-url = { $field } { $value } must be a URL starting with https://

# ── Update checks (upgrade/) ───────────────────────────────────────────────

plugin-settings-upgrade-none = No new version of the app { $id }
plugin-settings-upgrade-no-origin = The app { $id } has no source, so no new version can be looked up
plugin-settings-upgrade-id-changed = The package is { $package }, but the update is for { $record }: an update can't change the ID
plugin-settings-upgrade-same-version = Same as the installed { $version }; nothing to update
plugin-settings-upgrade-changed = The new version of { $id } changed after the preview (its package digest is now { $digest }): preview and confirm again
plugin-settings-upgrade-older = Older than the installed { $version }. The app's data may already have been migrated to the new version, which the old one may not read; back up first
plugin-settings-upgrade-source-changed = The source changed from { $old } to { $new }; Wonton won't offer updates from the old source anymore
plugin-settings-upgrade-origin-upload = the file you uploaded
plugin-settings-upgrade-origin-index = the index at { $via }
plugin-settings-upgrade-index-unavailable = The official index could not be fetched, so new versions cannot be checked: { $details }
plugin-settings-upgrade-system-app = System apps update with the home server and can't be updated here
plugin-settings-upgrade-lan-ports-closing = Local network ports { $ports } will close after the update; turn on the switches below to keep them open
plugin-settings-upgrade-check-failed = No new version of { $id } could be found: { $error }
plugin-settings-upgrade-official-commit-moved = Version { $version } in the official index now points to a different commit: not counted as a new version; the installed one is unchanged
plugin-settings-upgrade-tag-moved = The author moved the tag { $tag } of this version to a different commit: not counted as a new version; the installed one is unchanged
plugin-settings-upgrade-index-removed = The index { $provider } has been removed from the store and will no longer be checked for new versions
plugin-settings-upgrade-compose-not-utf8 = { $file } is not UTF-8 text
plugin-settings-upgrade-manifest-id-changed = The manifest's id changed to { $id }
plugin-settings-upgrade-manifest-version = The manifest's version is { $version }
plugin-settings-upgrade-bundled-failed = Can't update the built-in { $version }; { $from } is still running, and it will try again at the next start: { $error }

# ── Talking to GitHub (github.rs, github_source.rs) ────────────────────────

plugin-settings-github-tags-what = reading the tags of { $repo }
plugin-settings-github-no-repo = There is no public repository { $repo } on GitHub
plugin-settings-github-status = GitHub replied { $status } while { $what }
plugin-settings-github-tags-parse = The tags of { $repo } that GitHub returned could not be read: { $error }
plugin-settings-github-file-what = fetching { $path } of { $repo } at commit { $commit }
plugin-settings-github-missing = { $what }: not found
plugin-settings-github-url-what = fetching { $url }
plugin-settings-github-api-what = fetching { $path } of { $repo } at { $reference } through the API
plugin-settings-github-rate-limited = { $what }: GitHub rate-limited the request; try again in a few minutes
plugin-settings-github-rate-limited-plain = GitHub rate-limited the request; try again in a few minutes
plugin-settings-github-api-status = { $what }: GitHub replied { $status }
plugin-settings-github-search-url = The search address is invalid: { $error }
plugin-settings-github-search-what = searching community apps
plugin-settings-github-search-status = GitHub replied { $status } while searching community apps
plugin-settings-github-search-parse = The search results GitHub returned could not be read: { $error }
plugin-settings-github-context = { $what }: { $error }
plugin-settings-github-timeout = { $what }: the request timed out and did not finish in time
plugin-settings-github-unreachable = { $what }: GitHub could not be reached ({ $error })
plugin-settings-github-too-big = Larger than { $max } KiB
plugin-settings-github-read-what = reading the response
plugin-settings-github-no-tag = { $repo } has no tag { $tag }
plugin-settings-github-no-release = { $repo } has no stable release: there must be a tag like v<semantic version>, e.g. v1.0.0
plugin-settings-github-no-release-tag = There is no stable-release tag like v<semantic version>
plugin-settings-github-tag-version-mismatch = The tag { $tag } does not match the manifest version { $version }: when publishing, the tag must be v<manifest version>
plugin-settings-github-tag-version-short = The tag { $tag } does not match the manifest version { $version }
plugin-settings-github-source-invalid = The source `{ $text }` is invalid: { $why }; write it as github:<owner>/<repo>, optionally with @<tag> or @<40-digit commit>
plugin-settings-github-source-scheme = Only sources starting with github: are supported
plugin-settings-github-source-no-name = The repository name is missing
plugin-settings-github-source-name-chars = The owner and repository name may only contain letters, digits, `-`, `_`, and `.`
plugin-settings-github-source-bad-tag = The tag after @ is invalid
plugin-settings-github-sibling-url = { $path } next to { $url } is not a valid address: { $error }

# ── Source and index addresses (source_address.rs, index_location.rs) ──────

plugin-settings-source-address-invalid = The address `{ $text }` is invalid: { $why }
plugin-settings-source-address-tls = Only https: unreviewed sources need TLS to confirm the content has not been altered
plugin-settings-source-address-https-only = Only https addresses and GitHub repositories are supported
plugin-settings-source-address-formats = Write a repository address, owner/repo, or the https address of an index
plugin-settings-source-not-a-repo = `{ $text }` is not an app repository. Add the index in the store and install from there
plugin-settings-source-page-invalid = The GitHub address `{ $path }` is invalid: write it as https://github.com/<owner>/<repo>, optionally with /tree/<tag> or /commit/<commit>
plugin-settings-index-address-invalid = The index address `{ $text }` is invalid: { $why }
plugin-settings-index-address-https = Only https is supported
plugin-settings-item-source-invalid = The source `{ $source }` is invalid

# ── Model gateway and model providers (models.rs, model_providers.rs) ──────

plugin-settings-models-gateway-not-running = The model gateway { $id } is not running
plugin-settings-models-gateway-absent = There is no model gateway installed (a system app declaring provides.models)
plugin-settings-models-revoke-not-running = The model gateway is not running, so the credentials of { $id } cannot be revoked
plugin-settings-models-revoke-provider-not-running = The model gateway { $provider } is not running, so the credentials of { $id } cannot be revoked
plugin-settings-models-reconcile-failed = Can't sync the model gateway's credentials: { $error }
plugin-settings-models-credential-rejected = The model gateway { $provider } rejected the credentials { $name } issued by the server: listing models still returned 401 after they were issued
plugin-settings-models-unreachable = The model gateway { $provider } is unreachable: { $error }
plugin-settings-models-list-error = The model gateway { $provider } returned an error while listing models: HTTP { $status }
plugin-settings-models-provider-error = The default model's provider failed: { $error }
plugin-settings-models-provider-invalid-reply = the provider did not return a valid chat completion
plugin-settings-models-list-malformed = The model gateway { $provider } returned a malformed model list
plugin-settings-models-connect-first = Connect a model provider in Models before the bot can reply.
plugin-settings-models-not-ready = The model gateway { $provider } is not ready: { $error }
plugin-settings-models-command-rejected = The model gateway { $provider } rejected the credential command ({ $command }, exit code { $code }): { $stderr }
plugin-settings-model-action-save = save
plugin-settings-model-action-test = test
plugin-settings-model-disconnected-saved = Could not reach the provider, but the key was saved
plugin-settings-model-disconnected-unsaved = Could not reach the provider; nothing was saved
plugin-settings-model-no-provider-commands = The model gateway { $provider } declares no provider commands
plugin-settings-model-provider-unsupported = The model gateway does not support the provider { $id }
plugin-settings-model-custom-unsupported = The model gateway does not support custom providers
plugin-settings-model-custom-encode-failed = Can't encode the request for the custom provider
plugin-settings-model-command-timeout = The model gateway { $provider } command failed or timed out
plugin-settings-model-invalid-endpoint = The provider address or model name is invalid
plugin-settings-model-key-rejected = The provider rejected the key; it was not saved
plugin-settings-model-provider-command-failed = The model gateway { $provider } failed to { $action } the provider with exit code { $code }
plugin-settings-model-list-failed = The model gateway { $provider } failed to list providers: { $error }
plugin-settings-model-list-exit = The model gateway { $provider } failed to list providers, exit code { $code }
plugin-settings-model-list-not-json = The provider list from the model gateway { $provider } is not valid JSON
plugin-settings-model-key-invalid = The key must not be empty, must not contain line breaks, and must be at most 4 KiB
plugin-settings-model-url-invalid = The provider address must be an http or https URL, must not contain whitespace, and must be at most 2 KiB
plugin-settings-model-name-invalid = The model name must not be empty, must not contain whitespace or control characters, and must be at most 200 bytes
plugin-settings-model-request-format = The model provider request format is invalid

# ── Store index (storefront) ───────────────────────────────────────────────

plugin-settings-store-index-fetch-failed = The official index could not be fetched: { $error }

# ── Health checks (health.rs) ──────────────────────────────────────────────

plugin-settings-health-timeout = The container { $container } did not become ready within the deadline (last state: { $state })
plugin-settings-health-unhealthy = The container { $container } failed its health check
plugin-settings-health-exited = The container { $container } has exited (exit code { $code })
plugin-settings-health-no-healthcheck = The container { $container } is required to be service_healthy but has no health check configured
plugin-settings-health-webui-timeout = The web entry { $url } did not respond within the deadline: { $last_error }
plugin-settings-health-log-tail = Recent logs:
plugin-settings-health-probes = Recent probes:
plugin-settings-health-probe-line = [exit code { $code }] { $output }
plugin-settings-health-probe-line-unknown = [exit code ?] { $output }
plugin-settings-health-container-missing = Container { $container } not found
plugin-settings-health-one-shot-exit = The one-shot container { $container } did not exit with 0 ({ $status }, exit code { $exit_code })
plugin-settings-health-not-running = The container { $container } is not running ({ $status })

# ── Plugin secrets (secrets.rs) ────────────────────────────────────────────

plugin-settings-secret-missing = The app's key { $name } is missing
plugin-settings-secret-not-text = The app's key { $name } isn't text
plugin-settings-secret-invalid = App key { $name }: { $path } in container { $container } { $reason }
plugin-settings-secret-not-regular = is not a regular file
plugin-settings-secret-too-big = is larger than 64 KiB

# ── The app's bot (bots.rs) ────────────────────────────────────────────────

plugin-settings-bot-avatar-missing = bot.avatar points to { $path }, which is not in the plugin package
plugin-settings-bot-avatar-type = bot.avatar supports only png, jpg, gif, and webp: { $path }
plugin-settings-bot-device-name = Wonton app { $id } (bot { $bot })
plugin-settings-bot-account-deactivated = The bot account { $user } has been deactivated: uninstall this app choosing to delete its data and reinstall, and a new account will be assigned
plugin-settings-bot-device-record-missing = There is no device record for bot { $bot }: reinstall the app (you can keep its data)
plugin-settings-bot-device-revoked = The bot { $user }'s device has been revoked, so the Matrix token the app holds no longer works: reinstall it (you can keep its data) to get a new one

# ── LAN detection, the egress container and the store index (lan/detect.rs, docker_limits.rs, egress, storefront) ──

plugin-settings-lan-detect-wait-exit = waiting for the one-shot container to exit
plugin-settings-lan-detect-read-output = reading the one-shot container's output
plugin-settings-lan-detect-list-networks = listing networks
plugin-settings-limits-update-container = updating container { $name }
plugin-settings-mirror-docker-proxy = querying Docker's proxy settings
plugin-settings-mirror-inspect = inspecting the image
plugin-settings-mirror-inspect-image = inspecting image { $image }
plugin-settings-mirror-pull-image = pulling image { $image }
plugin-settings-egress-rebuild-failed = Rebuilding with the new proxy settings failed: { $error }
plugin-settings-egress-gost-config = Can't generate gost's configuration: { $error }
plugin-settings-egress-prepare-dir = Can't prepare { $dir }: { $error }
plugin-settings-egress-write-file = Can't write { $path }: { $error }
plugin-settings-egress-remove-dir = Can't remove { $dir }: { $error }
plugin-settings-store-index-no-format = no format
plugin-settings-store-index-docs-problem = The official index's docs could not be read: { $error }
plugin-settings-store-index-onboarding-problem = The official index's onboarding could not be read: { $error }
plugin-settings-store-index-mirrors-problem = The official index's registry_mirrors could not be read: { $error }
plugin-settings-store-index-read-failed = The official index { $path } could not be read: { $error }

plugin-secret-unknown = Unknown key in the request: { $names }
plugin-secret-required-missing = Fill in the required keys: { $names }
plugin-secret-required-clear = The required key { $name } can't be cleared
plugin-secret-not-input = { $name } is not a key the owner fills in
chat-card-secrets-not-used = This confirmation does not take keys
