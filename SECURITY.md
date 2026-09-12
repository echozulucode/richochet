# Security

## Scope, honestly

Richochet is a single-maintainer project. There is no security team and no response-time
commitment. What there is: reports get read, and anything that holds up gets fixed in the next
release.

## Reporting

Use [GitHub's private vulnerability reporting](https://github.com/echozulucode/richochet/security/advisories/new)
rather than a public issue, so a fix can ship before the details are public. Include what you did,
what happened, and the version from the Settings menu.

Please don't use public issues for anything exploitable.

## What is worth reporting

Richochet parses **untrusted input**: clipboard HTML written by another process. That is the
interesting surface, and the place a real bug is most likely to be:

- Clipboard HTML that gets script or an active payload past `ammonia` and into the WebView.
- Clipboard HTML that reads or writes outside the document — a path, a network request, a file.
- A crafted CF_HTML header that causes a panic, a hang, or an out-of-bounds read in the decoder.
  The offsets in that header are attacker-controlled byte indices, and they are validated, but
  that validation is the kind of thing worth attacking.
- Anything that escapes the app's CSP (`default-src 'self'`).

## What is not a vulnerability

- **Conversion that produces wrong or ugly Markdown.** That's a bug — please file it as an issue,
  with a fixture if you can. It isn't a security issue.
- **The Windows SmartScreen warning on first install.** Expected: the installer isn't
  Authenticode-signed. See the README.
- **A dependency advisory with no path to exploitation here.** Reports are welcome, but a version
  number alone isn't a finding.

## Supported versions

The latest release only. There are no backports.
