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
permission-models = Use the models you connected
permission-tools = Use the little tools your home server and other apps offer
permission-plugins-health = See whether all apps are working
permission-matrix-bot = Send and receive messages as a chat bot
permission-notify = Send your household notifications and mail
permission-owner-identity = Know your username and email, and which language your family members use
permission-network-egress = Reach the internet and your local network
permission-egress-proxy = Carry other apps' Internet traffic (high permission)
permission-network-lan = Let devices at home connect to it directly (high permission)
permission-network-clients = Once you flip the switch, let other apps connect from your home network or the internet
permission-network-host = Use your home server's own network (very high permission)
permission-capability = Add a system capability { $name }
permission-host-pid = See every program running on the server (highest permission)
permission-host-ipc = Exchange data directly with programs on the server (highest permission)
permission-security-opt = Change its security settings (highest permission)
permission-privileged = Run with the widest access (highest permission)
permission-devices-gpu = Use the graphics card (high permission)
permission-device = Use the server device { $path } (high permission)
permission-devices-serial = Use serial devices on the server (high permission)
permission-devices-usb = Use USB devices on the server (high permission)
permission-devices-disk = Read and write the whole disks you choose (very high permission)
permission-docker-socket = Manage Docker (highest permission)

# Longer explanations shown under the high permissions.
permission-notice-egress-proxy = When you pick it as your Internet proxy, everything the other apps send to the internet goes through it: it can see which sites they visit and change anything unencrypted.
permission-notice-network-host = This app can get online just like the server itself, including services meant only for the server, and can skip Wonton login to reach some other apps. The ports it opens are reachable on every network address, from your home network and from the internet if the server has a public address. If the server has a firewall on, devices at home can't get in until you allow them.
permission-notice-highest-risk = This hands the server's highest control to it: it can read and write every file and other apps' data, and change or stop Wonton itself. Only install apps you trust. Uninstalling doesn't clean up the containers it made or the things it changed.
permission-notice-network-lan = Devices at home can connect straight to the ports it opens, without Wonton login; it decides who is allowed itself.
permission-notice-device = This app can use the server device { $path }.
permission-notice-devices = This app can use server devices you hand it.
permission-notice-devices-disk = This app can read and write everything on the disks you choose.
permission-notice-devices-usb = This app can use every USB device on the server, including U disks and card readers.

# Notices in the app detail view.
plugin-firewall-notice = A firewall on this machine may be blocking incoming connections, so devices on your local network may not be able to connect.
plugin-egress-proxy-select = Select it in Internet proxy settings
plugin-cross-origin-isolation = This app needs cross-origin isolation: on Android, its pages open in your browser.

# Notices and blocked reasons in the install and update previews.
install-retained-data = Data and passwords from { $id } are still on the server; this app can read them. Delete them for good first if you don't want that.
install-image = { $service } ({ $image })
install-images-from = The programs come from { $images }
install-images-unpinned = The programs aren't locked to a specific version: { $images }
install-webui-sso = Opening its web page in Wonton signs you in automatically.
install-host-network-egress = It reaches the internet over the server's own network, not through Wonton's proxy.
install-host-network-proxied = It reaches the internet through Wonton's proxy too.
install-already-installed = { $id } is already installed (version { $version })
install-unreviewed-provider = Wonton hasn't reviewed this app. Its program comes from { $provider }.
install-unreviewed-provider-via = Wonton hasn't reviewed this app. Its program comes from { $provider }, listed by { $host }.
install-unreviewed-upload-author = Wonton hasn't reviewed this app. It came from a file you uploaded; the author the manifest names is { $author }, which can't be checked.
install-unreviewed-upload = Wonton hasn't reviewed this app. It came from a file you uploaded, and the manifest names no author.
plugin-widget-high = The app's page can read chat content in the room where you open it, and send or redact messages as you.
plugin-widget-normal = The app's page can also send and receive call and to-device events, send events later, find the call server, and get your login token.
