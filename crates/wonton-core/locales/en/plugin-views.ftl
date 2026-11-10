## Texts this home server sends to the App (ADR-0043). One message per sentence;
## data (paths, ports, device names, codes) goes in as parameters, untranslated.
## English is the base language, in Sentence case. Errors read like "Can't reach …".

# Separator for listing data items (image repositories and the like). A string literal
# keeps the spaces; bare quotes in a pattern would render as literal quote characters.
list-separator = {", "}

# Permissions, shown in the install confirmation, approval cards and store search results.
permission-storage-share =
    { $access ->
        [read] Read the shared folder { $name }
       *[write] Read and write the shared folder { $name }
    }
permission-storage-home =
    { $access ->
        [read] Read your private space
       *[write] Read and write your private space
    }
permission-models = Use the model gateway
permission-tools = Use tools from your home server and other apps
permission-plugins-health = Read the health status of all apps
permission-matrix-bot = Send and receive messages as a bot
permission-notify = Send your household notifications and mail
permission-owner-identity = Know your username and email, and which language your family members use
permission-network-egress = Reach the internet and your local network
permission-egress-proxy = Provide an Internet proxy (high permission)
permission-network-lan = Open ports on your local network (high permission)
permission-network-host = Use the host's network (very high permission)
permission-capability = Add the kernel capability { $name }
permission-host-pid = Share the host process namespace (highest permission)
permission-host-ipc = Share the host IPC namespace (highest permission)
permission-security-opt = Change container security options (highest permission)
permission-privileged = Run in privileged mode (highest permission)
permission-devices-gpu = Use the GPU (high permission)
permission-device = Access the host device { $path } (high permission)
permission-devices-serial = Access host serial devices (high permission)
permission-devices-usb = Access host USB devices (high permission)
permission-docker-socket = Control Docker (highest permission)

# Longer explanations shown under the high permissions.
permission-notice-egress-proxy = When you select it as your Internet proxy, all requests from apps with network access pass through it: it can see which websites they visit and modify unencrypted content.
permission-notice-network-host = This app can directly access your home network and all ports on this machine, including services bound only to localhost, and bypass Wonton login to connect to some other apps. Its ports will appear on all network interfaces, including Wi-Fi, Tailscale, and IPv6 addresses; if this machine has a public address, the internet can reach them too. If the machine firewall is on, LAN devices cannot connect until you allow them yourself.
permission-notice-highest-risk = This app can control the entire host: read and write all files and other apps' data, change or stop Wonton itself. This is equivalent to giving it root on the host. Only install apps you trust. After uninstalling, containers it created and things it changed will not be cleaned up.
permission-notice-network-lan = Devices on your local network can connect directly to the app's ports without Wonton login; the app is responsible for authentication.
permission-notice-device = This app can access the host device { $path }.
permission-notice-devices = This app can access host devices you give it.

# Notices in the app detail view.
plugin-firewall-notice = A firewall on this machine may be blocking incoming connections, so devices on your local network may not be able to connect.
plugin-egress-proxy-select = Select it in Internet proxy settings
plugin-cross-origin-isolation = This app relaxes cross-origin isolation: on Android, its pages may share a process with other web pages.

# Notices and blocked reasons in the install and update previews.
install-retained-data = Data and keys from { $id } are still on this server; the app you install will be able to read them. Delete them completely first if you don't want that.
install-image = { $service } ({ $image })
install-images-from = Images come from { $images }
install-images-unpinned = Images aren't pinned by digest: { $images }
install-webui-sso = Opening its web page in Wonton signs you in automatically.
install-host-network-egress = Services in the host network reach the internet through the host's own routing, not Wonton's Internet proxy.
install-system-preinstalled = System apps can only come preinstalled with your home server.
install-already-installed = { $id } is already installed (version { $version })
install-unreviewed-provider = Wonton hasn't reviewed this app; the code and images are provided by { $provider }.
install-unreviewed-provider-via = Wonton hasn't reviewed this app; the code and images are provided by { $provider } and indexed by { $host }.
install-unreviewed-upload-author = Wonton hasn't reviewed this app. It came from a file you uploaded; the author named in its manifest is { $author }, which can't be verified.
install-unreviewed-upload = Wonton hasn't reviewed this app. It came from a file you uploaded, and its manifest doesn't name an author.
