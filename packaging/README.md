# Packaging keys

`liteaura-archive.gpg` / `liteaura-archive.asc` are the **public** keys for the
LiteAura APT repository. The private key is never committed: GitHub Actions
imports it from the `LITEAURA_GPG_PRIVATE_KEY` repository secret and signs
`dists/liteaura/InRelease` on every publish.

The key fingerprint is printed by:

```sh
gpg --show-keys --with-colons packaging/liteaura-archive.gpg | awk -F: '/^fpr:/ {print $10; exit}'
```
