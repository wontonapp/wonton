# Capability access, dependency errors, and installation confirmation.
gateway-capability-permission = The app { $id } hasn't declared capability { $capability } or hasn't been approved to use its provider
plugin-capability-missing = Capability { $capability } needs a provider: install one before continuing
plugin-capability-stopped = The app { $provider } isn't ready to provide capability { $capability }
plugin-capability-credential-failed = The app { $provider } couldn't prepare or revoke access to capability { $capability } (exit code { $code }): { $detail }
plugin-capability-in-use = These apps still need this provider: { $plugins }. Install another provider or uninstall these apps first.
plugin-capability-provider-install = Also install { $provider } ({ $id })
plugin-capability-provider-permission = { $provider } needs this permission: { $permission }
plugin-capability-provider-optional-off = Optional permissions for { $provider } will stay off
plugin-capability-connect = { $consumer } will be able to use capability { $capability } through { $provider }
plugin-capability-provider-failed = Couldn't install the provider { $provider }: { $error }
plugin-capability-install-selection = The selected providers don't match the missing required capabilities. Open the installation preview again and choose providers from its list.
plugin-capability-request-format = Can't read these capability settings. Send a valid JSON request.
plugin-capability-model-migration = Declaring the model gateway as an optional capability needs a newer core. Keep using the models permission until the model gateway migration.
plugin-capability-plan-changed = The installation plan changed after preview. Open the preview again and confirm the updated connections.
plugin-capability-choices-unsupported = This core cannot apply selected devices or a restricted bot scope. These choices have not been changed or installed.
plugin-capability-uninstall-selection = The selected providers are still in use or weren't offered for removal. Open the uninstall preview again.
