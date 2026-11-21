# Security policy

## Reporting a vulnerability

Please **do not open a public issue** for security problems. Report them privately through
GitHub's [security advisories](https://github.com/askscience/shiny/security/advisories/new)
for this repository, with:

- what the problem is and what an attacker could do with it;
- steps to reproduce (a request, a page, a plugin, a configuration);
- the LiteAura version or commit and how it was installed (APT package, built from source).

You will get an answer as soon as possible. Fixes are released first; the advisory is made
public once an update is available.

## Supported versions

Security fixes go into the latest release (APT suite `liteaura`) and `main`. Please test on
the latest version before reporting.

## What to keep in mind

- **Network exposure.** The server listens on `127.0.0.1` by default. Setting
  `SERVER_HOST=0.0.0.0` serves it to your network: do that only on networks you trust, and put
  HTTPS in front of it (browsers also require HTTPS for the microphone, except on `localhost`).
- **Accounts.** With `SHINY_LINUX_USERS=true`, passwords are checked against the system through
  PAM by the small root helper `shiny-auth`; issues in that helper are especially important.
- **Plugins run with LiteAura's own permissions.** Installing a plugin means running its code,
  whether it is a process plugin or a library loaded into the server. Install only plugins you
  trust. A plugin doing what its own code says is not a vulnerability; a way to make LiteAura
  load or run plugin code without the user's consent is.
- **The assistant.** The agent acts with the permissions of the signed-in user. Content it
  reads (web pages, documents) is untrusted data; ways to make it perform actions the user did
  not ask for through such content (prompt injection) are in scope.
- **Packages.** The APT suite is signed (`liteaura-archive.gpg`); a way to get unsigned or
  tampered packages accepted is in scope.

## Out of scope

- Problems in third-party software LiteAura relies on (Ollama, models, browsers, system
  libraries) — please report those to their projects, but do tell us if LiteAura makes them
  exploitable.
- Attacks that need someone who is already signed in as the same user, or physical access to an
  unlocked session.
