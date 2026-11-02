# Maintainer: v0-0x
# In the repository, `makepkg -si` builds this checkout. Anywhere else (a clean chroot, an AUR
# helper, a copy of just this file) it builds the release tagged v$pkgver.
pkgname=multimusic
# The same as `version` in Cargo.toml (a test checks). A fixed version, not a pkgver() function:
# makepkg writes what pkgver() returns into this file, which then blocks `git pull`.
pkgver=0.9.6
pkgrel=1
pkgdesc="Utako: lightweight native music player for local files, Spotify and SoundCloud with synced lyrics, Discord Rich Presence and Last.fm scrobbling"
arch=('x86_64' 'aarch64')
url="https://github.com/v0-0x/Utako"
license=('MIT')
depends=('mpv' 'libpulse' 'alsa-lib' 'openssl' 'libxkbcommon' 'libglvnd' 'wayland' 'libx11' 'libxcursor' 'libxrandr' 'libxi'
         'hicolor-icon-theme' 'gcc-libs' 'glibc')
makedepends=('cargo')
optdepends=('pipewire-pulse: Spotify output on PipeWire desktops (usually already installed)'
            'noto-fonts: symbols (☆ ✞ ♡ ...) in song and artist names'
            'noto-fonts-cjk: Japanese, Chinese and Korean song titles'
            'inter-font: nicer interface font'
            'discord: Rich Presence (also works with Vesktop / arRPC)'
            'yt-dlp: YouTube music videos, and Apple Music downloads (found on YouTube)'
            'deno: unlocks YouTube video links for yt-dlp (node or bun work too)')
# The app is called Utako (Sumo until 0.9.0); the package keeps its first name so installed copies
# update in place.
provides=('multimusic' 'sumo' 'utako')
conflicts=('medley')
replaces=('medley')
options=('!lto' '!debug')

# makepkg sets startdir to the folder holding this file before reading it.
if [[ -f "${startdir:-.}/Cargo.toml" ]]; then
  source=()
  sha256sums=()
else
  makedepends+=('git')
  source=("sumo-music::git+$url.git#tag=v$pkgver")
  sha256sums=('SKIP')
fi

_root() {
  if [[ -f "$startdir/Cargo.toml" ]]; then
    printf '%s' "$startdir"
  else
    printf '%s' "$srcdir/sumo-music"
  fi
}

prepare() {
  cd "$(_root)"
  export RUSTUP_TOOLCHAIN=stable
  cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
  cd "$(_root)"
  export RUSTUP_TOOLCHAIN=stable
  cargo build --frozen --release
}

check() {
  cd "$(_root)"
  export RUSTUP_TOOLCHAIN=stable
  cargo test --frozen --release
}

package() {
  cd "$(_root)"
  install -Dm755 target/release/multimusic "$pkgdir/usr/bin/multimusic"
  ln -s multimusic "$pkgdir/usr/bin/utako"
  ln -s multimusic "$pkgdir/usr/bin/sumo"
  install -Dm644 packaging/multimusic.desktop "$pkgdir/usr/share/applications/multimusic.desktop"
  # The icon is named after the app (it was "sumo"), so desktops that cached the old logo under
  # the old name load the new one.
  install -Dm644 assets/logo.svg "$pkgdir/usr/share/icons/hicolor/scalable/apps/utako.svg"
  local size
  for size in 16 24 32 48 64 128 256 512; do
    install -Dm644 "assets/icon-$size.png" "$pkgdir/usr/share/icons/hicolor/${size}x${size}/apps/utako.png"
  done
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
