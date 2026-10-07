#!/bin/sh
# Installs the latest Upleveler release for macOS or Linux.
#
#   curl -fsSL https://upleveler.dev/install.sh | sh
#
# It downloads the archive for your system from GitHub, checks it against the
# release's SHA256SUMS, and puts `upleveler` in ~/.local/bin (or in
# $UPLEVELER_INSTALL_DIR). Nothing else on your system is changed.
set -eu

REPO="k61b/upleveler"
DIR="${UPLEVELER_INSTALL_DIR:-$HOME/.local/bin}"
# For testing the script against another location (a mirror, a local server).
BASE="${UPLEVELER_DOWNLOAD_URL:-https://github.com/$REPO/releases/latest/download}"

fail() {
    echo "upleveler: $*" >&2
    exit 1
}

os=$(uname -s)
arch=$(uname -m)
case "$os-$arch" in
    Darwin-arm64) target=aarch64-apple-darwin ;;
    Darwin-x86_64) target=x86_64-apple-darwin ;;
    Linux-x86_64 | Linux-amd64) target=x86_64-unknown-linux-musl ;;
    Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-musl ;;
    *) fail "there is no prebuilt binary for $os $arch. Install from source: https://github.com/$REPO#install" ;;
esac

download() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$1" -O "$2"
    else
        fail "needs curl or wget to download"
    fi
}

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 "$1" | cut -d ' ' -f 1
    fi
}

archive="upleveler-$target.tar.gz"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

echo "Downloading $archive"
download "$BASE/$archive" "$tmp/$archive" || fail "could not download $BASE/$archive"
download "$BASE/SHA256SUMS" "$tmp/SHA256SUMS" || fail "could not download the checksums"

expected=$(grep " $archive\$" "$tmp/SHA256SUMS" | cut -d ' ' -f 1 || true)
actual=$(sha256 "$tmp/$archive")
[ -n "$expected" ] || fail "$archive is not listed in SHA256SUMS; nothing was installed"
[ "$expected" = "$actual" ] || fail "checksum mismatch for $archive; nothing was installed"

tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$DIR"
cp "$tmp/upleveler" "$DIR/upleveler.new"
chmod 755 "$DIR/upleveler.new"
mv "$DIR/upleveler.new" "$DIR/upleveler"

echo "Installed $("$DIR/upleveler" --version) to $DIR/upleveler"
case ":$PATH:" in
    *":$DIR:"*) ;;
    *) echo "Add $DIR to your PATH, for example: echo 'export PATH=\"$DIR:\$PATH\"' >> ~/.profile" ;;
esac
echo "Next: install Ollama (https://ollama.com), run 'ollama pull gemma4:12b', then 'upleveler'."
