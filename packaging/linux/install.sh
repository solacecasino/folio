#!/bin/sh
# Installs Folio for this user: ~/.local/bin/folio, its themes, and a menu
# entry with the icon. Run it from the unpacked folder: ./install.sh
# To uninstall: ./install.sh --uninstall
set -e
cd "$(dirname "$0")"
PREFIX="${PREFIX:-$HOME/.local}"

if [ "$1" = "--uninstall" ]; then
  rm -f "$PREFIX/bin/folio" "$PREFIX/share/applications/folio.desktop" \
        "$PREFIX/share/icons/hicolor/256x256/apps/folio.png"
  rm -rf "$PREFIX/share/folio"
  echo "Removed Folio. Your settings in ~/.config/folio are kept."
  exit 0
fi

mkdir -p "$PREFIX/bin" "$PREFIX/share/folio" "$PREFIX/share/applications" \
         "$PREFIX/share/icons/hicolor/256x256/apps"
install -m 755 folio "$PREFIX/bin/folio"
rm -rf "$PREFIX/share/folio/themes"
cp -R themes "$PREFIX/share/folio/themes"
install -m 644 folio.png "$PREFIX/share/icons/hicolor/256x256/apps/folio.png"
install -m 644 folio.desktop "$PREFIX/share/applications/folio.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" || true

echo "Installed Folio to $PREFIX/bin/folio. Find it in your app menu, or run: folio"
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "(Add $PREFIX/bin to your PATH to run it from a terminal.)" ;; esac
