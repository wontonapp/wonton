## Texts of the agent sign-in flow (chat commands, terminal sign-in over the control socket).
## The command names in them stay untranslated.

login-redact-reason = Passed on to the agent sign-in
login-answer-first = Answer the sign-in link first: /wonton login accept or /wonton login decline.
login-code-limits = The code must be a single line of text, at most 16 KiB.
login-not-sent = Signing in has ended; this message wasn't sent to the agent.
login-no-forward = Chat isn't forwarded while signing in: finish on the official page, or send /wonton login cancel.
login-no-direct = Can't find our direct chat; send /wonton login there.
login-in-direct = Signing in happens in our direct chat.
login-cancelled = Sign-in cancelled.
login-no-pending = There's no sign-in link waiting for an answer.
login-finish-current = Finish the current turn before starting a sign-in.
login-starting = Starting the agent sign-in. To cancel, send /wonton login cancel.
login-unsupported-method = This sign-in method isn't supported yet.
login-unknown-method = The agent doesn't offer this sign-in method; send /wonton login to see them.
login-secret-delete = This message holds a sign-in secret; delete it yourself.
login-done = Signed in.
login-no-methods = The agent offers no sign-in method.
login-methods-header = Choose how to sign in to the agent:
login-method-line = { $name }: /wonton login { $id }
login-link =
    Signing in to the agent:
    { $message }
    { $url }
    Open the link, then reply /wonton login accept; to refuse, reply /wonton login decline.

# Details inside the error prefixes (see errors.ftl).
login-auth-failed = agent sign-in failed (ACP { $code })
login-auth-kind-unsupported = the agent offers an unsupported sign-in type
login-timeout = sign-in timed out; start it again
login-socket-connect = can't reach the login control socket { $path }: { $error }
login-invalid-frame = the login socket returned an invalid frame
login-process-exited = the agent sign-in process exited early
login-failed-detail = agent sign-in failed: { $detail }
login-exit-code = the agent sign-in process failed with exit code { $code }
login-socket-closed = the login socket closed: { $error }
login-chat-closed = the sign-in chat closed
