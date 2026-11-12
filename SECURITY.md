# Security

is-alive is a network testing tool that runs on client sites, so a security problem in it matters. Thank you for reporting one.

## Reporting a vulnerability

Please report it privately, not in a public issue:

1. Open the repository's **Security** tab on GitHub.
2. Choose **Report a vulnerability**.
3. Say what you found, which version (bottom right corner of the window), and how to reproduce it.

You'll get a reply within a week. A fix ships as a new release, and the release notes credit you unless you'd rather they didn't.

## Supported versions

Only the [latest release](https://github.com/hammerknife404/is-alive/releases/latest) gets fixes. The app's status bar shows a green **Update** link when a newer one is out.

## Checking a download

Each release has a `.sha256` file beside `is-alive.exe` and `is-alive-x86_64.AppImage`. Compare it with the file you downloaded:

- **Windows:** `Get-FileHash is-alive.exe -Algorithm SHA256`
- **Linux:** `sha256sum -c is-alive-x86_64.AppImage.sha256`

The Windows build is not code-signed yet, so Windows shows *Unknown Publisher* when it starts.

## What is-alive sends

There is no telemetry, no account and no analytics. When it opens, the app makes one DNS query to OpenDNS for the public address and one HTTPS request to GitHub for the latest release; both can be turned off in Settings. Everything else is sent only when you run a tool, and the manual says what each tool sends and to whom.
