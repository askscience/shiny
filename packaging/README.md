# Packaging keys

`peakd-archive.gpg` / `peakd-archive.asc` are the **public** keys for the
PEAK'D! APT repository. The private key is never committed: GitHub Actions
imports it from the `PEAKD_GPG_PRIVATE_KEY` repository secret and signs
`dists/peakd/InRelease` on every publish.

The key fingerprint is printed by:

```sh
gpg --show-keys --with-colons packaging/peakd-archive.gpg | awk -F: '/^fpr:/ {print $10; exit}'
```
