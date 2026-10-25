#!/bin/sh
# Installs Folio for this user: ~/.local/bin/folio, its themes, a menu entry
# with the icon, and makes Folio open .folio files and folio:// links (such
# as Folio Link invites). Run it from the unpacked folder: ./install.sh
# To uninstall: ./install.sh --uninstall
set -e
cd "$(dirname "$0")"
PREFIX="${PREFIX:-$HOME/.local}"

if [ "$1" = "--uninstall" ]; then
  rm -f "$PREFIX/bin/folio" "$PREFIX/share/applications/folio.desktop" \
        "$PREFIX/share/icons/hicolor/256x256/apps/folio.png" "$PREFIX/share/mime/packages/folio.xml"
  command -v update-mime-database >/dev/null && update-mime-database "$PREFIX/share/mime" || true
  rm -rf "$PREFIX/share/folio"
  echo "Removed Folio. Your settings in ~/.config/folio are kept."
  exit 0
fi

mkdir -p "$PREFIX/bin" "$PREFIX/share/folio" "$PREFIX/share/applications" \
         "$PREFIX/share/icons/hicolor/256x256/apps" "$PREFIX/share/mime/packages"
install -m 755 folio "$PREFIX/bin/folio"
rm -rf "$PREFIX/share/folio/themes"
cp -R themes "$PREFIX/share/folio/themes"
install -m 644 folio.png "$PREFIX/share/icons/hicolor/256x256/apps/folio.png"
install -m 644 folio.desktop "$PREFIX/share/applications/folio.desktop"
install -m 644 folio-mime.xml "$PREFIX/share/mime/packages/folio.xml"
command -v update-mime-database >/dev/null && update-mime-database "$PREFIX/share/mime" || true
command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" || true
# Open .folio documents and folio:// links (Folio Link invites) with Folio.
if command -v xdg-mime >/dev/null; then
  xdg-mime default folio.desktop application/x-folio || true
  xdg-mime default folio.desktop x-scheme-handler/folio || true
fi

echo "Installed Folio to $PREFIX/bin/folio. Find it in your app menu, or run: folio"
# Libraries Folio needs that minimal systems sometimes lack.
if command -v ldconfig >/dev/null && ! ldconfig -p | grep -q libxkbcommon-x11.so; then
  echo "Folio also needs libxkbcommon-x11: sudo apt install libxkbcommon-x11-0 (Debian/Ubuntu), sudo dnf install libxkbcommon-x11 (Fedora)."
fi
command -v zenity >/dev/null || command -v kdialog >/dev/null || command -v xmessage >/dev/null \
  || echo "For Folio's questions (like saving before closing), install zenity: sudo apt install zenity (or your system's equivalent)."
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "(Add $PREFIX/bin to your PATH to run it from a terminal.)" ;; esac
