privacy-what-title = No cloud
privacy-what-own = Wonton has no cloud of its own. The server is the machine in your home (or one you rent): messages, photos and files live on it.
privacy-what-none = We get no accounts or device information. The app has no analytics, ads or crash reporting.

privacy-messages-title = Chats and files
privacy-messages-e2ee = Chats and files are end-to-end encrypted, and only your devices can open them.
privacy-messages-history = History stays on your own server, and you hold the backup key.

privacy-push-title = Notifications (Android only)
privacy-push-keepalive = With "stay connected", notifications never leave your tunnel: the other end is your home.
privacy-push-unified = With UnifiedPush, your home core first encrypts each notification with your device key and hands it to the distributor. The distributor sees ciphertext only, and it is decrypted on the phone. The distributor is a third-party service.

privacy-network-title = Reaching home
privacy-network-lan = On the same network the app connects directly. Away from home it tries to punch a hole first and only relays when that fails.
privacy-network-relay = A relay forwards encrypted traffic and sees no content. The default is iroh's public relay; your home can switch to its own.

privacy-preview-title = Link previews
privacy-preview-core = When a chat shows a link, your home core fetches the page's title and image, so the site sees your home server's address.

privacy-plugins-title = Third-party sign-in and plugin pages
privacy-plugins-oidc = Signing in to a plugin with another account sends the name and password to that plugin's server only; the home core keeps no third-party password.
privacy-plugins-web = Plugin pages open in your phone's browser engine, and whether and where they go online is up to the plugin.

privacy-device-title = What stays on the phone
privacy-device-files = Device keys, connection details, caches and logs sit in the app's private directory, unreadable by other apps. Uninstalling removes them and leaves the server history alone.
privacy-device-backup = These files follow the system backup (iCloud or Google), and the system setting decides whose account holds that backup.
