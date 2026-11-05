# Security

Firelight reads untrusted content (every connected repo's `.firelight/`
folder), serves it to people and agents, and runs as a long-lived server.
Reports about content escaping (script on the page, a link sent where its
author couldn't send it, an agent told to do something), path escapes, auth,
the webhook, model download verification or the release pipeline are welcome
and taken seriously.

**Report privately** through GitHub: *Security* tab → *Report a vulnerability*
(<https://github.com/prosperity-solutions/firelight/security/advisories/new>).
If that button is not there yet, open an issue that says only that you have a
security report (no details), and a maintainer will arrange a private channel.
Please put nothing exploitable in a public issue.

Supported: the latest release, and `main`. Older releases are not patched;
upgrade to the newest.

How releases are built and verified, and what is and is not trusted, is in
[docs/security.md](docs/security.md) and [docs/release.md](docs/release.md).
