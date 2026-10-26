pkgname=amph
pkgver=0.4.0_beta
pkgrel=1
pkgdesc='Arch Linux package harmonizer with a ratatui TUI and scriptable CLI'
arch=('x86_64' 'aarch64')
url='https://github.com/DAMIOTF/Arch-Meta-Package-Harmonizer'
license=('custom:PolyForm-Noncommercial-1.0.0')
depends=('glibc' 'pacman')
makedepends=('cargo')
source=("${pkgname}-${pkgver}.tar.gz::${url}/archive/refs/tags/v0.4.0-beta.tar.gz")
sha256sums=('SKIP')

prepare() {
  cd "${srcdir}/Arch-Meta-Package-Harmonizer-0.4.0-beta"
  cargo fetch --locked
}

build() {
  cd "${srcdir}/Arch-Meta-Package-Harmonizer-0.4.0-beta"
  cargo build --release --locked --offline
}

check() {
  cd "${srcdir}/Arch-Meta-Package-Harmonizer-0.4.0-beta"
  cargo test --release --locked --offline
}

package() {
  cd "${srcdir}/Arch-Meta-Package-Harmonizer-0.4.0-beta"
  install -Dm755 "target/release/${pkgname}" "${pkgdir}/usr/bin/${pkgname}"
  install -Dm644 LICENSE "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"
  install -Dm644 README.md "${pkgdir}/usr/share/doc/${pkgname}/README.md"
  install -Dm644 hooks/amph-history.hook \
    "${pkgdir}/usr/share/amph/hooks/amph-history.hook"
}
