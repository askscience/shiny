# PEAK'D! APT repository

Signed static APT repository for the `peakd` package.

```sh
sudo install -m 0644 peakd-archive.gpg /usr/share/keyrings/peakd-archive.gpg
echo 'deb [signed-by=/usr/share/keyrings/peakd-archive.gpg] https://askscience.github.io/shiny/apt/ trixie main' \
  | sudo tee /etc/apt/sources.list.d/peakd.list
sudo apt update
sudo apt install peakd
```

The private signing key is never stored in this repository. The GitHub Pages
workflow deploys the already-built, signed tree; signing belongs to a protected
release environment or an offline signer.
