## Errors of link previews (ADR-0065). The App shows its own text by category; these go in the details.
## { $detail } is the raw value: an address, a status, a content type or the underlying error.

link-preview-invalid-url = Only http and https links on ports 80, 443, 8080 or 8443 get a preview: { $detail }
link-preview-blocked-address = The link points to a local or private address, so the home server won't open it: { $detail }
link-preview-dns-failed = The domain couldn't be resolved through the proxy, so the site couldn't be reached: { $detail }
link-preview-upstream-error = The site couldn't be reached or returned an error: { $detail }
link-preview-timeout = The site didn't answer within { $detail }
link-preview-not-html = The link isn't a web page: { $detail }
link-preview-no-preview = The page has no title, description or image: { $detail }
link-preview-too-many-requests = This device asked for too many previews; try again in a minute
link-preview-image-gone = The preview image has expired; get the preview again
