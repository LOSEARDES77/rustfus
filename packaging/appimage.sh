#!/bin/sh
# Builds Rustfus-<version>-<arch>.AppImage from a release binary.
# Usage: packaging/appimage.sh <binary> <version> <arch: x86_64|aarch64> <out dir>
set -eu
bin=$1 version=$2 arch=$3 out=$4
work=$(mktemp -d)
app="$work/Rustfus.AppDir"
install -Dm755 "$bin" "$app/usr/bin/rustfus"
install -Dm644 res/rustfus.png "$app/usr/share/icons/hicolor/256x256/apps/rustfus.png"
install -Dm644 LICENSE "$app/usr/share/licenses/rustfus/LICENSE"
install -Dm644 THIRD_PARTY_NOTICES.md "$app/usr/share/doc/rustfus/THIRD_PARTY_NOTICES.md"
# Inside an AppImage the binary isn't at /usr/bin, so the entry runs it directly;
# AppRun asks for root through pkexec (keeping the display) when needed.
sed 's|^Exec=.*|Exec=rustfus|' packaging/rustfus.desktop > "$app/rustfus.desktop"
cp res/rustfus.png "$app/rustfus.png"
cat > "$app/AppRun" <<'RUN'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
if [ "$(id -u)" -ne 0 ] && command -v pkexec >/dev/null; then
  exec pkexec env DISPLAY="${DISPLAY:-}" XAUTHORITY="${XAUTHORITY:-$HOME/.Xauthority}" \
    WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-}" XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-}" \
    "$here/usr/bin/rustfus" "$@"
fi
exec "$here/usr/bin/rustfus" "$@"
RUN
chmod +x "$app/AppRun"
tool="$work/appimagetool"
curl -fsSL -o "$tool" "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$arch.AppImage"
chmod +x "$tool"
mkdir -p "$out"
ARCH=$arch "$tool" --appimage-extract-and-run --no-appstream "$app" "$out/Rustfus-$version-$arch.AppImage"
rm -rf "$work"
