## Texts for installing and running apps, sent to the Wonton app (ADR-0043). One complete
## sentence per message; data (paths, ports, image names, container names, Docker's own
## wording) goes in as parameters, untranslated. English is the base language, Sentence
## case. Errors read like "Can't connect …". No "please", no "Failed to", no "Error:".

# ── Operations in progress (manager.rs busy) ─────────────────────────────────

plugin-install-busy-installing = { $id } is being installed
plugin-install-busy-uninstalling = { $id } is being uninstalled
plugin-install-busy-upgrading = { $id } is being updated

# ── Install and uninstall conflicts and permissions (manager.rs, install_entry.rs, written_by.rs)

plugin-install-conflict-already-installed = { $id } is already installed
plugin-install-conflict-slug = { $id } and the installed { $other } use the same web address
plugin-install-conflict-models-gateway = The model gateway { $id } is already installed: there can only be one
plugin-install-conflict-egress-proxy-in-use = This Internet proxy app is in use: switch to another one in the Internet proxy settings, then uninstall
plugin-install-conflict-lan-rebuilding = { $id } is being rebuilt for the new local network address
plugin-install-uninstall-unfinished = Couldn't finish uninstalling: { $error }
plugin-install-permission-not-declared = Granted permission { $permission } is not declared in the manifest
plugin-install-permission-unsupported = Your home server doesn't support permission { $permission } yet
plugin-install-record-unreadable = Can't read this app's install record. Uninstall it (you can keep its data) and install it again: { $error }
plugin-install-record-detail-failed = Can't build the details page for { $id }
plugin-install-not-agent-written = { $id } wasn't written by an agent, so it has no "Ask Every Time" switch

# ── Rollback (lifecycle.rs): items of the failure list and the summary ───────

plugin-install-rollback-item = { $what }: { $error }
plugin-install-rollback-unfinished = Couldn't finish rolling back, there may be leftovers: { $failures }
plugin-install-rb-container = container { $name }
plugin-install-rb-home-room = your main room { $room }
plugin-install-rb-bot-device = device of bot { $bot }
plugin-install-rb-model-credentials = model gateway credentials of { $id }
plugin-install-rb-oidc-client = OIDC client { $id }
plugin-install-rb-volume = volume { $volume }
plugin-install-rb-network = network { $network }
plugin-install-rb-core-leave-network = leaving network { $network } for the home server
plugin-install-rb-egress-leave-network = leaving network { $network } for the proxy container
plugin-install-rb-directory = directory { $dir }
plugin-install-rb-secret = keys of { $id }

# Words fed into name-taken messages (which kind of resource it is).
plugin-install-word-volume = volume
plugin-install-word-network = network
plugin-install-word-container = container

# ── Failures while deploying (lifecycle.rs) ──────────────────────────────────

plugin-install-oidc-unconfigured = Your home server has no identity provider configured (WONTON_OIDC_ADMIN_URL / WONTON_OIDC_PUBLIC_URL)
plugin-install-shared-folder-mkdir-failed = Can't create shared folder directory { $dir }: { $error }
plugin-install-shared-folder-prepare-failed = Can't prepare the shared folder: { $error }
plugin-install-app-data-prepare-failed = Can't prepare the app data directory: { $error }
plugin-install-configs-pack-failed = Can't pack config files for { $service }: { $error }
plugin-install-teardown-delete-failed = Can't delete { $dir }: { $error }
plugin-install-teardown-leftovers = Uninstalled, but these are left over: { $left }

# ── Taken names and stranger resources (leftovers.rs) ────────────────────────

plugin-install-name-taken-owned = { $what } { $name } belongs to app { $owner } and clashes with this app's { $what }
plugin-install-name-taken-unknown = { $what } { $name } already exists and wasn't created by your home server for an app
plugin-install-stranger-attached = Still attached to network { $network }: { $names } — maybe another home server or your own containers. When one Docker host runs several home servers, each needs its own instance name (WONTON_INSTANCE)

# ── Reconciling and starting (reconcile.rs, start.rs) ────────────────────────

plugin-install-network-mismatch = App network { $network } { $actual }, but the way the manifest and your home server work it should be { $expected }: reinstall the app (you can keep its data) to apply it
plugin-install-resident-stopped = This app's always-on services have stopped
plugin-install-resident-partial = Some always-on services are still running, these containers have stopped: { $containers }
plugin-install-containers-unhealthy = Some containers aren't healthy: { $failures }
plugin-install-container-unhealthy = Container { $container } failed its health check (last probe, exit code { $code }): { $output }
plugin-install-container-unhealthy-nocode = Container { $container } failed its health check (last probe, exit code unknown): { $output }
plugin-install-container-unhealthy-noprobe = Container { $container } failed its health check (Docker kept no probe result)
plugin-install-container-not-running = Container { $container } isn't running ({ $status })
plugin-install-one-shot-failed = The one-shot service's container { $container } didn't exit 0 ({ $status }, exit code { $code })
plugin-install-reach-none = doesn't exist
plugin-install-reach-egress = can reach the internet
plugin-install-reach-internal = can't reach the internet
plugin-install-reach-isolated = can't reach the internet and is cut off from the host
plugin-install-start-failed = Couldn't start: { $error }
plugin-install-conflict-rebuilding = { $id } is being rebuilt; try again when it finishes
plugin-install-record-unusable = This app's install record is unusable, so it can't start
plugin-install-container-missing = Container { $name } doesn't exist
plugin-install-container-not-owned = Container { $name } doesn't belong to app { $id }

# ── Out of memory (oom.rs) ───────────────────────────────────────────────────

plugin-install-oom-notice = Service { $service } ran out of memory and was stopped by the system{ $limit }
plugin-install-oom-reason = Service { $service } ran out of memory and was stopped by the system{ $limit }
plugin-install-oom-limit-both = (service limit { $size }, or all apps together may have hit the total limit)
plugin-install-oom-limit-service = (limit { $size })
plugin-install-oom-limit-total = (all apps together may have hit the total limit)

# ── CPU and memory limits (limits_update.rs, limits.rs API paths) ────────────

plugin-install-limits-busy = { $id } is being installed, uninstalled or rebuilt; try again when it finishes
plugin-install-limits-min-memory = memory must be at least { $mib } MiB
plugin-install-limits-ncpu-missing = Docker /info doesn't report NCPU
plugin-install-limits-ncpu-invalid = The NCPU reported by Docker /info is invalid
plugin-install-limits-ncpu-out-of-range = The NCPU reported by Docker /info is out of range
plugin-install-limits-cpu-percent-range = cpu_percent must be between 1 and { $max }
plugin-install-limits-host-memory-unknown = Can't read the host's total memory; restoring defaults waits for a rebuild
plugin-install-limits-ncpu-bad = The CPU count { $ncpu } reported by Docker is out of range
plugin-install-service-error = Service { $service }: { $error }
plugin-install-limits-mem-reservation-order = mem_reservation can't be larger than mem_limit
plugin-install-limits-memory-format = requirements.memory `{ $value }` isn't valid
plugin-install-limits-field-forward = { $field }: { $error }
plugin-install-limits-positive = { $field } must be greater than 0
plugin-install-limits-integer-positive = { $field } must be a whole number greater than 0
plugin-install-limits-cpus-format = cpus `{ $text }` isn't valid
plugin-install-limits-cpus-positive = cpus must be greater than 0
plugin-install-limits-cpus-range = cpus `{ $text }` is out of range
plugin-install-limits-memory-range = The memory limit is outside the range Docker supports

# ── App package (package.rs) ─────────────────────────────────────────────────

plugin-install-pkg-yaml = The compose file isn't valid YAML: { $error }
plugin-install-pkg-not-mapping = The top level of the compose file must be a mapping
plugin-install-pkg-missing-manifest = The compose file is missing the `x-wonton` extension field; a compose without a manifest needs its permissions declared first
plugin-install-pkg-manifest = `x-wonton` isn't valid: { $error }
plugin-install-pkg-service-mapping = A service definition must be a mapping
plugin-install-pkg-unsupported-key = `{ $key }` isn't supported: { $reason }
plugin-install-pkg-network-mode-unsupported = network_mode `{ $mode }` isn't supported: { $reason }
plugin-install-pkg-configs-mapping = Must be a mapping that sets file or content
plugin-install-pkg-configs-name-chars = Config names can only contain letters, digits, -, _ and .
plugin-install-pkg-configs-key-unsupported = `{ $key }` isn't supported
plugin-install-pkg-configs-file-content-one = Set file or content, not both
plugin-install-pkg-configs-file-content-required = Set file or content
plugin-install-pkg-configs-file-content-string = file and content must be strings
plugin-install-pkg-mapping-required = Must be a mapping
plugin-install-arch-gzip-failed = Can't decompress the gzip: { $error }
plugin-install-arch-unpacked-too-big = Over { $mib } MiB once decompressed
plugin-install-arch-too-many-entries = The archive contains more than { $max } entries
plugin-install-pkg-top-level = Top-level field `{ $key }` isn't supported: { $reason }
plugin-install-pkg-service = Service `{ $service }`: { $error }
plugin-install-pkg-interpolate = Can't interpolate service `{ $service }`: { $error }
plugin-install-pkg-unknown-service = { $field } refers to service `{ $service }`, which doesn't exist
plugin-install-pkg-volume = Named volume `{ $name }` isn't valid: only local volumes without options are supported
plugin-install-pkg-config = Config `{ $name }`: { $error }
plugin-install-pkg-config-interpolate = Can't interpolate config `{ $name }`: { $error }
plugin-install-pkg-reference = { $field }: { $error }
plugin-install-pkg-file = { $field } refers to file `{ $path }`: { $error }
plugin-install-pkg-compose-file = The app package's compose.yaml: { $error }
plugin-install-pkg-archive = The app package archive: { $error }
plugin-install-pkg-too-big = The app package is { $kib } KiB in total, over the { $max } KiB limit
plugin-install-pkg-not-utf8 = Not UTF-8 text
plugin-install-pkg-needs-service = At least one service is required
plugin-install-install-icon-file-missing = { $field } { $raw }: no such file in the package, or it isn't PNG/SVG
plugin-install-install-icon-error = Icon { $path }: { $reason }
plugin-install-pkg-icon-file = `{ $icon }` must be a PNG or SVG file in the app package
plugin-install-pkg-config-undeclared = Not declared in the top-level configs
plugin-install-pkg-config-missing = The app package doesn't contain `{ $path }`
plugin-install-pkg-ref-bot-service = { $field } can't point at a bot service: bot services are one per bot
plugin-install-pkg-egress-network-mode = provides.egress_proxy.service can't set network_mode
plugin-install-pkg-not-in-subset = Not in the Compose subset your home server supports
plugin-install-pkg-reserved-service = This service name is reserved for your home server's relays; apps can't use it
plugin-install-pkg-service-name-dns = Service names must be valid DNS labels: lowercase letters, digits and - only
plugin-install-pkg-service-name-dashes = Service names can't end with - or contain --

# ── App package archive (package_archive.rs) ─────────────────────────────────

plugin-install-arch-read-failed = Can't read the archive: { $error }
plugin-install-arch-path-invalid = Invalid path in the archive: { $error }
plugin-install-arch-zip-read-failed = Can't read the zip: { $error }
plugin-install-arch-symlink = symlink
plugin-install-arch-zip-path-invalid = Invalid path in the zip: { $error }
plugin-install-arch-two-roots = Both { $a } and { $b } contain compose.yaml: one archive holds one app
plugin-install-arch-missing-compose = The archive doesn't contain compose.yaml
plugin-install-arch-not-regular = `{ $rel }` isn't a regular file ({ $what }): app packages can't contain symlinks, hard links or devices
plugin-install-arch-duplicate-file = The archive contains two `{ $rel }`
plugin-install-arch-not-tar = Not a tar archive: { $error }
plugin-install-arch-extract-failed = Can't extract `{ $path }`: { $error }
plugin-install-arch-read-entry-failed = Can't read `{ $path }` from the archive: { $error }

# ── compose wording (compose.rs) ─────────────────────────────────────────────

plugin-install-compose-ports-forbidden = Apps don't expose ports to the host; web pages always go through your home server's gateway
plugin-install-compose-networks-forbidden = App networks are created and managed by your home server
plugin-install-compose-container-name-forbidden = Container names are assigned by your home server
plugin-install-compose-build-forbidden = Apps use published images only; they aren't built on the host
plugin-install-compose-env-file-forbidden = An app package has one compose file; put environment variables in environment
plugin-install-compose-extends-forbidden = An app package has one compose file; extends isn't supported
plugin-install-compose-links-forbidden = Services of one app share a network; reach each other by service name
plugin-install-compose-external-links-forbidden = Apps can't reach each other by default
plugin-install-compose-volumes-from-forbidden = Share data with named volumes or the app data directory
plugin-install-compose-top-networks-forbidden = App networks are created and managed by your home server
plugin-install-compose-top-secrets-forbidden = Keys are injected by your home server; compose can't declare secrets
plugin-install-compose-network-host = The host network is a high permission (network.host)
plugin-install-compose-network-mode = network_mode only accepts none, host, or service:<another service of this app>; app networks are created and managed by your home server
plugin-install-compose-config-external-forbidden = Configs can only come from the app package
plugin-install-compose-config-environment-forbidden = Your home server doesn't pass its own environment variables to apps; use content interpolation when you need one
plugin-install-compose-config-name-forbidden = Your home server doesn't create Docker config objects, so name isn't needed
plugin-install-compose-pair-missing = `{ $key }` in { $field } has no value (your home server doesn't pass host environment variables through)
plugin-install-compose-device-format = Device mapping `{ $text }` isn't formatted correctly
plugin-install-compose-device-missing-source = Device mapping is missing source/path_on_host
plugin-install-compose-device-missing-target = Device mapping is missing target/path_in_container
plugin-install-compose-duration-invalid = Duration `{ $text }` isn't valid (examples: 30s, 1m30s, 500ms)
plugin-install-compose-bytes-invalid = Size `{ $text }` isn't valid (examples: 64m, 1g)

# ── configs (configs.rs) ─────────────────────────────────────────────────────

plugin-install-cfg-undeclared = Config `{ $name }` isn't declared in the top-level configs
plugin-install-cfg-target-duplicate = The configs target `{ $raw }` is used twice
plugin-install-cfg-target-overlaps-mount = The configs target `{ $raw }` overlaps the mount { $mount }: configs live in the container's own file system and can't go into a mount
plugin-install-cfg-target-absolute = The configs target `{ $raw }` must be an absolute path and can't contain ..
plugin-install-cfg-target-root = The configs target can't be /
plugin-install-cfg-id-invalid = The configs { $field } `{ $value }` must be a non-negative integer
plugin-install-cfg-mode-invalid = The configs mode `{ $value }` isn't valid: it must be an octal permission from 0 to 0777

# ── Mounts (mounts.rs) ───────────────────────────────────────────────────────

plugin-install-mnt-short-format = Volume `{ $text }` isn't formatted correctly
plugin-install-mnt-option-unsupported = Volume option `{ $bad }` isn't supported
plugin-install-mnt-relative-path = Relative path `{ $path }` isn't supported; use a path under { $var }
plugin-install-mnt-target-absolute = Mount target { $target } must be an absolute path
plugin-install-mnt-volume-undeclared = Named volume { $name } isn't declared in the top-level volumes
plugin-install-mnt-bind-missing-source = The bind mount is missing source
plugin-install-mnt-host-path-parent = Host path `{ $path }` can't contain ..
plugin-install-mnt-host-path-outside = Host path `{ $path }` isn't in the app data directory or an authorized shared folder
plugin-install-mnt-shared-folder-whole = A shared folder can only be mounted whole, not just its `{ $path }`
plugin-install-mnt-shared-folder-readonly = Shared folder { $path } is authorized for reading only and must be mounted :ro
plugin-install-mnt-nested = Host path `{ $path }` sits inside { $outer_path } mounted by service { $outer }; bind mounts of one app can't nest (a container could swap the inner directory for a symlink and escape the app data directory), so use side-by-side subdirectories
plugin-install-mnt-owner-conflict = { $path } is mounted with a different owner than service { $other }; your home server can't tell who it belongs to
plugin-install-mnt-numeric-user = The app data directory is mounted, so user { $user } must be a numeric uid:gid: your home server uses it to decide who gets the directory

# ── Permissions (privileges.rs) ──────────────────────────────────────────────

plugin-install-priv-agent-high-risk = Agent and bot apps can't request the high or highest permission { $permission }
plugin-install-priv-fixed-device = Fixed device permissions only support /dev/net/tun and /dev/fuse, not { $permission }
plugin-install-priv-host-network-ports = A host network service can't also declare lan.ports; the app listens on host network ports directly
plugin-install-priv-host-network-sso = webui on the host network can't use sso: proxy; use oidc or none
plugin-install-priv-host-network-healthcheck = A webui service on the host network must have a valid healthcheck
plugin-install-priv-cap-invalid = Capability `{ $cap }` in cap_add isn't valid
plugin-install-priv-cap-drop-invalid = Capability `{ $cap }` in cap_drop isn't valid
plugin-install-priv-security-opt-invalid = Security option `{ $opt }` isn't valid (no-new-privileges takes only true or false)
plugin-install-priv-device-not-allowed = Device { $source } isn't one of the allowed fixed devices
plugin-install-priv-device-absolute = Device target { $target } must be an absolute path
plugin-install-priv-device-mode = Permissions { $permissions } of device { $source } aren't valid
plugin-install-priv-pid-host-only = pid only supports host
plugin-install-priv-ipc-host-only = ipc only supports host
plugin-install-priv-socket-mount = The Docker socket can only be mounted writable as /var/run/docker.sock:/var/run/docker.sock
plugin-install-priv-missing-cap-add = Permission { $permission } has no matching cap_add field
plugin-install-priv-missing-devices = Permission { $permission } has no matching devices field
plugin-install-priv-field-requires = { $field } needs manifest permission { $permission }
plugin-install-priv-missing-field = Manifest permission { $permission } has no matching { $field } field

# ── provides (provides.rs) ───────────────────────────────────────────────────

plugin-install-prv-egress-needs-permission = provides.egress_proxy needs the egress.proxy permission
plugin-install-prv-permission-needs-egress = The egress.proxy permission needs provides.egress_proxy
plugin-install-prv-egress-needs-network = provides.egress_proxy needs the network.egress permission
plugin-install-prv-egress-port = provides.egress_proxy.port can't be 0
plugin-install-prv-empty = provides must declare at least one capability
plugin-install-prv-models-system-only = Only kind: system apps can declare provides.models
plugin-install-prv-models-port = provides.models.port can't be 0
plugin-install-prv-models-apis-path = Paths in provides.models.apis must start with /
plugin-install-prv-models-default-model = provides.models.default_model can't be empty
plugin-install-prv-credentials-argv = provides.models.credentials.{ $field } must be a non-empty argv naming the credential with { $placeholder }
plugin-install-prv-list-argv = provides.models.providers.list must be a non-empty argv
plugin-install-prv-provider-argv = provides.models.providers.{ $field } must be a non-empty argv naming the provider id with { "{{" }id{ "}}" }
plugin-install-prv-custom-argv = provides.models.providers.custom.{ $field } must be a non-empty argv without placeholders

# ── Proxy apps' cookies and headers (proxy_auth.rs) ──────────────────────────

plugin-install-proxy-secret-unknown = { "{{" }secret:{ $name }{ "}}" } must refer to an item in secrets that your home server generates (generate) and that is at least { $min } bytes
plugin-install-proxy-cookie-template = A cookie value can only use { "{{" }secret:…{ "}}" } and fixed text: `{ $template }`
plugin-install-proxy-cookie-chars = A cookie value can't contain whitespace, quotes, commas, semicolons or backslashes: `{ $template }`
plugin-install-proxy-header-control-chars = A header value can't contain control characters: `{ $template }`
plugin-install-proxy-header-forbidden = Can't add header { $name } on the app's behalf
plugin-install-proxy-cookie-forbidden = Can't add cookie { $name } on the app's behalf
plugin-install-proxy-mcp-prefix = mcp.headers: { $error }
plugin-install-proxy-mcp-header-forbidden = Can't add header { $name } to the MCP upstream
plugin-install-proxy-mcp-secret-undeclared = { "{{" }secret:{ $name }{ "}}" } must refer to an item declared in secrets
plugin-install-proxy-mcp-identity-placeholder = MCP headers can't use identity placeholders like { "{{" }{ $placeholder }{ "}}" }: `{ $template }`
plugin-install-proxy-mcp-header-control-chars = A header value can't contain control characters: `{ $template }`
plugin-install-proxy-template-open = `{ $template }` has a { "}}" } without a matching { "{{" }
plugin-install-proxy-template-close = `{ $template }` has a { "{{" } without a matching { "}}" }
plugin-install-proxy-template-unknown-var = `{ $template }` contains unknown { "{{" }{ $var }{ "}}" }: only { "{{" }user{ "}}" }, { "{{" }name{ "}}" }, { "{{" }email{ "}}" } and { "{{" }secret:<name>{ "}}" } are available
plugin-install-proxy-secret-missing = App key { $name } isn't in the key store
plugin-install-proxy-secret-not-text = In mcp.headers { $name }, key { $secret } isn't text, so it can't go into a header
plugin-install-proxy-secret-newline = In mcp.headers { $name }, key { $secret } contains a line break or control character (a trailing line break counts)
plugin-install-proxy-secret-no-value = App key { $secret } has no value yet, and mcp.headers { $name } refers to it
plugin-install-proxy-mcp-no-identity = mcp.headers { $name } can't use identity placeholders
plugin-install-proxy-mcp-bad-value = mcp.headers { $name } doesn't fill out to a valid header value
plugin-install-proxy-mcp-bad-name = mcp.headers { $name } isn't a valid header name: { $error }

# ── Requirements (requirements.rs, host_check.rs) ────────────────────────────

plugin-install-req-size-format = { $field } `{ $size }` must be a size like 512M or 2G
plugin-install-req-arch-line = Only { $arches } architecture
plugin-install-req-memory-line = Needs { $memory } of memory
plugin-install-req-gpu-vram-suffix = , at least { $memory } of VRAM
plugin-install-req-gpu-required = Needs { $card }{ $memory }
plugin-install-req-gpu-optional = Uses { $card }{ $memory } when available
plugin-install-req-card-generic = a GPU
plugin-install-req-card-named = { $names } GPU
plugin-install-or-separator = {" or "}
plugin-install-host-unknown = Can't check { $what }: { $why }
plugin-install-host-arch-unknown-what = this host's architecture
plugin-install-host-arch-mismatch = This host is { $arch }
plugin-install-host-memory-wanted-what = how much memory is needed
plugin-install-host-memory-unknown-what = this host's memory
plugin-install-host-memory-low = This host only has { $size } of memory
plugin-install-host-gpu-unknown-what = this host's GPU
plugin-install-host-gpu-problem = { $card }: { $why }
plugin-install-host-gpu-missing = This host has no { $card }
plugin-install-host-fit-vram-unreadable = Can't parse the VRAM requirement { $text }
plugin-install-host-fit-unknown = Can't check VRAM: { $why }
plugin-install-host-fit-integrated = This integrated GPU shares memory with the system; its { $size } isn't a cap, so there's no way to tell whether it's enough
plugin-install-host-fit-vram-low = Only { $size } of VRAM, but { $text } is needed

# ── Host facts (host_facts.rs) ───────────────────────────────────────────────

plugin-install-facts-gpu-nvidia = NVIDIA { $device }
plugin-install-facts-gpu-amd = AMD { $device }
plugin-install-facts-gpu-intel = Intel { $device }
plugin-install-facts-gpu-other = Display device { $vendor }:{ $device }
plugin-install-facts-meminfo-missing = /proc/meminfo has no { $field }
plugin-install-facts-meminfo-unreadable = Can't read /proc/meminfo: { $error }
plugin-install-facts-docker-not-reported = Docker doesn't report it
plugin-install-facts-pci-unreadable = Can't read { $path }: { $error }
plugin-install-facts-driver-nouveau = The nouveau driver is loaded, which can't compute; the official NVIDIA driver is needed
plugin-install-facts-driver-bound = Bound to the { $driver } driver
plugin-install-facts-driver-none = No driver is loaded
plugin-install-facts-runtime-missing = Docker doesn't register the nvidia runtime: install the NVIDIA Container Toolkit
plugin-install-facts-runtime-unknown = Can't tell whether Docker has the nvidia runtime: { $reason }
plugin-install-facts-driver-no-render = The { $driver } driver has no render node, so it can't compute
plugin-install-facts-no-host-dev = Your home server runs in a container without the host's /dev mounted (WONTON_HOST_DEV), so it can't see device nodes
plugin-install-facts-dri-missing = The host's /dev/dri has no { $render }
plugin-install-facts-model-unknown = The driver doesn't report a model, only a device ID
plugin-install-facts-vram-not-in-sysfs = NVIDIA VRAM isn't in sysfs
plugin-install-facts-vram-unknown = The driver doesn't report a VRAM size
plugin-install-facts-core-not-in-container = Your home server isn't in a container, so there's no image to ask the NVIDIA driver with
plugin-install-facts-container-trial-failed = The trial run in a container failed: { $reason }
plugin-install-facts-container-image-unknown = Can't find the image of home server container { $core }
plugin-install-facts-nvidia-smi-exit = nvidia-smi exited with { $code }: { $reason }

# ── GPU (gpu.rs) ─────────────────────────────────────────────────────────────

plugin-install-gpu-needs-permission = Declaring gpu requires the devices.gpu permission
plugin-install-gpu-needs-services = Requesting devices.gpu requires listing the GPU services in gpu.services
plugin-install-gpu-needs-all-three = requirements.gpu must be declared together with the devices.gpu permission and gpu.services
plugin-install-gpu-services-nonempty = gpu.services needs at least one entry and no duplicates
plugin-install-gpu-describe-vendors = { $vendors } GPU: { $devices }; containers join groups { $groups }
plugin-install-gpu-describe-nvidia = NVIDIA GPU: all of it, via the NVIDIA Container Toolkit
plugin-install-gpu-problem-host-dev = Your home server runs in a container without the host's /dev mounted (WONTON_HOST_DEV), so it can't see GPU devices: restart your home server with the compose file of the current version
plugin-install-gpu-problem-unreadable = Can't read { $path }: { $error }
plugin-install-gpu-problem-no-device = No GPU found (no display device on PCI)
plugin-install-gpu-problem-no-driver = There's a GPU but no usable driver: { $detail }
plugin-install-gpu-problem-no-runtime = There's an NVIDIA driver, but Docker doesn't register the nvidia runtime: install the NVIDIA Container Toolkit and run `nvidia-ctk runtime configure --runtime=docker`
plugin-install-gpu-problem-no-wanted-vendor = No { $card }
plugin-install-gpu-vendor-other = vendor { $id }
plugin-install-gpu-no-driver-none = No driver is loaded
plugin-install-gpu-no-driver-nouveau = The nouveau driver is loaded, which can't compute; the official NVIDIA driver is needed
plugin-install-gpu-no-driver-no-render = The { $driver } driver is loaded but has no render node
plugin-install-gpu-device-entry = { $address } ({ $vendor }) { $state }
plugin-install-gpu-no-device-nodes = No device nodes inside
plugin-install-gpu-preview-granted = GPU goes to services { $services }: { $describe }
plugin-install-gpu-preview-missing = No matching GPU on this machine ({ $problem }); services { $services } will run without a GPU after install
plugin-install-gpu-preview-required = Services { $services } need a GPU: { $reason }

# ── Rendering (render.rs) ────────────────────────────────────────────────────

plugin-install-render-no-owner-identity = The app doesn't request owner.identity, so it can't reference `{ $var }`: add it to the manifest's permissions
plugin-install-render-bot-dependency = A service shared by all apps can't depend on bot service `{ $service }`: bot services are one per bot
plugin-install-render-one-shot-restart = A one-shot service depended on with service_completed_successfully must set restart: "no", or Docker keeps restarting it after it exits
plugin-install-render-network-mode-none = The service { $field } points at can't set network_mode: none
plugin-install-render-dependency-missing = depends_on refers to service `{ $missing }`, which doesn't exist
plugin-install-render-dependency-cycle = depends_on has a circular dependency
plugin-install-render-reserved-label = Labels starting with `wonton.` are reserved for your home server
plugin-install-render-quotes-unbalanced = Command `{ $text }` has unbalanced quotes
plugin-install-render-restart-invalid = restart value `{ $value }` isn't valid
plugin-install-render-healthcheck-format = The healthcheck.test list must start with CMD, CMD-SHELL or NONE

# ── Docker (docker.rs, docker_exec.rs, core_address.rs) ──────────────────────

plugin-install-docker-unavailable = Can't connect to Docker: { $error }
plugin-install-docker-error-join = { $context }: { $error }
plugin-install-docker-timed-out = Docker didn't finish in time ({ $context }); the operation may still be running
plugin-install-docker-not-found = Not found in Docker: { $what }
plugin-install-docker-rejected = Docker rejected the request ({ $context }): { $error }
plugin-install-port-taken = The port { $place } is already in use. Stop the container or program using it, then try again. ({ $error })
plugin-install-port-taken-unknown = A port the app needs is already in use. Stop the container or program using it, then try again. ({ $error })
plugin-lan-taken-by-plugin = the app { $id }
plugin-lan-taken-by-container = the container { $name }
plugin-lan-taken-by-host = a program on this host
plugin-install-lan-port-taken = The port { $place } is already used by { $who }. Stop it, then install again.
plugin-install-lan-port-hinted = The port { $place } is already used by { $who }, so starting the container will fail.
plugin-install-lan-port-probe-failed = Could not check whether a program on this host uses these ports: { $reason }
plugin-install-probe-image-replaced = The home server's image has been replaced and the probe container ran the image its container was created with; restarting the home server fixes this ({ $reason })
plugin-install-probe-image-missing = The home server's image is gone from Docker and the probe container can't start; restarting the home server fixes this. Docker said: { $error }
plugin-install-docker-pull-failed = Can't pull image ({ $image }): { $error }
plugin-install-docker-context-version = query version
plugin-install-docker-context-host-info = query Docker info
plugin-install-docker-context-image-healthcheck = check image { $image }
plugin-install-docker-context-network = query network { $name }
plugin-install-docker-no-ipv4-gateway = The network has no IPv4 gateway
plugin-install-docker-no-subnets = The network has no subnets
plugin-install-docker-context-remove-network = remove network { $name }
plugin-install-docker-context-list-published-ports = list published ports
plugin-install-docker-context-connect-network = connect { $container } to network { $network }
plugin-install-docker-context-disconnect-network = disconnect { $container } from network { $network }
plugin-install-docker-context-create-volume = create volume { $name }
plugin-install-docker-context-volume-labels = query volume { $name }
plugin-install-docker-context-remove-volume = remove volume { $name }
plugin-install-docker-context-create-container = create container { $name }
plugin-install-docker-context-upload-archive = write files into container { $container }
plugin-install-docker-context-container-missing = container { $container }
plugin-install-docker-context-read-file = read { $path } in container { $container }
plugin-install-docker-archive-unparseable = Can't parse the archive: { $error }
plugin-install-docker-context-start-container = start container { $name }
plugin-install-docker-context-inspect-container = inspect container { $name }
plugin-install-docker-context-kill-container = send { $signal } to container { $name }
plugin-install-docker-context-restart-container = restart container { $name }
plugin-install-docker-context-stop-container = stop container { $name }
plugin-install-docker-context-remove-container = remove container { $name }
plugin-install-docker-context-list-containers = list containers
plugin-install-docker-context-list-networks = list networks
plugin-install-docker-context-list-volumes = list volumes
plugin-install-docker-engine-too-old = Docker Engine { $version } is too old; your home server needs { $min } or newer: from version 28 on, other hosts on the local network can no longer reach unpublished container ports directly, and that is also what keeps app networks fully separated from the host. Update Docker
plugin-install-docker-engine-version-unrecognized = Can't recognize the Docker Engine version `{ $version }`
plugin-install-docker-context-exec = run { $argv } in container { $container }
plugin-install-docker-exec-no-io = The command has no input/output attached
plugin-install-docker-exec-timed-out = { $context }: no result after { $seconds } seconds
plugin-install-coreaddr-no-ipv4-layout = The network has no IPv4 range that can leave an address for your home server
plugin-install-coreaddr-pool-exhausted-context = Creating network { $name }: your home server has run out of address ranges for app networks, so the app can't be installed; uninstall apps you don't use, or enlarge the ranges in your home server's { $var } setting
plugin-install-coreaddr-pool-exhausted = Range { $subnet }: { $count } of { $count } /26 ranges in use
plugin-install-coreaddr-kept-taken = { $context }: all { $attempts } attempts collided with networks created at the same time
plugin-install-coreaddr-reserved-taken = The reserved range { $subnet } was taken by another program: retry this rebuild and another range will be picked
plugin-install-coreaddr-context-create-network = create network { $name }
plugin-install-coreaddr-context-inspect-network = inspect network { $name }
plugin-install-coreaddr-no-core-address = The network doesn't have an address reserved for your home server yet

# ── Upstreams and LAN relay (upstreams.rs, host_relay.rs) ────────────────────

plugin-install-upstream-container-missing = Container { $container } doesn't exist, or has no address in network { $network }
plugin-install-upstream-mcp-headers = Can't resolve the MCP headers of app { $id }: { $error }
plugin-install-relay-prepare-dir-failed = Can't prepare the relay directory: { $error }
plugin-install-relay-path-too-long = The app data directory path is too long for a unix socket; move your home server's data directory somewhere shorter ({ $path }: { $bytes } bytes, limit { $max } bytes)
plugin-install-relay-listens-unknown = Can't read the listening table, so the firewall isn't judged

# ── Images (images.rs, images/preflight.rs, image_worker.rs) ─────────────────

plugin-install-img-tab-icon-alpha = A tab icon's alpha channel can't be fully opaque or fully transparent
plugin-install-img-too-many-kib = Over { $kib } KiB
plugin-install-img-too-many-mib = Over { $mib } MiB
plugin-install-img-not-png-or-jpeg = Not a PNG or JPEG image
plugin-install-img-is-format = This is a { $format } image; screenshots only take PNG or JPEG
plugin-install-img-encode-jpeg-failed = Can't encode JPEG: { $error }
plugin-install-img-not-supported-format = Not a supported image format
plugin-install-img-format-no-thumbnail = { $format } can't make a thumbnail
plugin-install-img-over-edge-or-memory = Over { $max }×{ $max } pixels or the memory limit
plugin-install-img-decode-failed = Can't decode: { $error }
plugin-install-img-png-too-big = PNG icon over { $kib } KiB
plugin-install-img-png-content = The file name is .png but the content isn't PNG
plugin-install-img-not-square = Icons must be square; this one is { $width }×{ $height }
plugin-install-img-png-too-small = A PNG icon is at least { $min }×{ $min }; this one is { $width }×{ $height }
plugin-install-img-svg-too-big = SVG icon over { $kib } KiB
plugin-install-img-svg-not-utf8 = An SVG must be UTF-8 text (compressed SVG isn't accepted)
plugin-install-img-svg-invalid = Not a valid SVG: { $error }
plugin-install-img-svg-parse-failed = Can't parse the SVG: { $error }
plugin-install-img-svg-viewbox = The SVG viewBox must be square; this one is { $width }×{ $height }
plugin-install-img-canvas-alloc-failed = Can't allocate the canvas
plugin-install-img-canvas-convert-failed = Can't convert the canvas
plugin-install-img-over-max-edge = Over { $max }×{ $max } pixels
plugin-install-img-encode-png-failed = Can't encode PNG: { $error }
plugin-install-img-svg-root = The root element must be <svg xmlns="{ $ns }">
plugin-install-img-style-rules = Style sheets allow at most { $max } rules; this one has { $count }
plugin-install-img-style-combinator = Style sheet selector “{ $selector }” uses a combinator: only combinator-free selectors like .cls-1 are allowed
plugin-install-img-nesting-depth = SVG nesting deeper than { $max } levels
plugin-install-img-use-cycle = Circular <use> reference in the SVG
plugin-install-img-use-expanded = Expanding <use> in the SVG gives more than { $max } elements
plugin-install-img-use-href-count = <use> needs exactly one href (href or xlink:href)
plugin-install-img-use-href-format = A <use> href can only be #id without whitespace: { $href }
plugin-install-img-element-forbidden = Icons can't use <{ $name }>
plugin-install-img-clip-path-units = <clipPath> clipPathUnits can only be userSpaceOnUse
plugin-install-img-mask-units = <mask> needs maskUnits="userSpaceOnUse", and maskContentUnits can only be userSpaceOnUse
plugin-install-img-gradient-stops = Gradients allow at most { $max } <stop>s; this one has { $count }
plugin-install-img-worker-usage = Usage: wonton-core { $command } <icon.png|icon.svg|tab-icon.png|tab-icon.svg|screenshot|thumbnail|preview> (home server internal)
plugin-install-img-worker-unavailable = The image process isn't available: { $error }
plugin-install-img-worker-rlimit = Can't set resource limits: { $error }
plugin-install-img-worker-stdin = Can't read standard input: { $error }
plugin-install-img-worker-stdout = Can't write standard output: { $error }
plugin-install-img-worker-spawn-failed = Can't start { $program }: { $error }
plugin-install-img-worker-communicate = Can't talk to the image process: { $error }
plugin-install-img-worker-timeout = Took over { $seconds } seconds and was stopped
plugin-install-img-worker-crashed = Crashed while processing (exit code { $code })
plugin-install-img-worker-cpu-timeout = Used over { $seconds } seconds of CPU time and was stopped
plugin-install-img-worker-memory-limit = Needed more than { $mib } MiB of memory while processing
plugin-install-img-worker-killed = The image process was killed (SIGKILL), most likely out of memory
plugin-install-img-worker-signal = The image process was ended by signal { $signal }
plugin-install-img-worker-malformed = The image process produced output in the wrong format

## 预装（启动时随服务器带的包）失败的原因。
plugin-install-preinstall-retrying = Preinstall failed (tried { $attempts } time(s), will retry automatically)
plugin-install-preinstall-failed = Preinstall failed
