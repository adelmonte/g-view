# Maintainer: adelmonte
pkgname=g-view
pkgver=0.1.0
pkgrel=2
pkgdesc="Minimal image viewer (GTK3 port of qView)"
arch=('x86_64')
license=('GPL-3.0-or-later')
depends=('gtk3' 'lcms2' 'gdk-pixbuf2' 'cairo' 'pango' 'glib2' 'hicolor-icon-theme')
makedepends=('cargo')
options=('!debug')

build() {
  cd "$startdir"
  cargo build --release --locked
}

package() {
  cd "$startdir"
  install -Dm755 target/release/g-view "$pkgdir/usr/bin/g-view"
  install -Dm644 dist/g-view.desktop "$pkgdir/usr/share/applications/g-view.desktop"
  for d in dist/hicolor/*/apps; do
    size=$(basename "$(dirname "$d")")
    for f in "$d"/*; do
      ext="${f##*.}"; case "$f" in *symbolic*) name=g-view-symbolic;; *) name=g-view;; esac
      install -Dm644 "$f" "$pkgdir/usr/share/icons/hicolor/$size/apps/$name.$ext"
    done
  done
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
