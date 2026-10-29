#!/bin/sh
# Installs text9000 for this user: ~/.local/bin/text9000, its themes, a menu
# entry with the icon, and makes text9000 open .t9k and .folio documents and
# text9000:// and folio:// links (such as Folio Link invites). Run it from
# the unpacked folder: ./install.sh
# To uninstall: ./install.sh --uninstall
#
# Folio's own install is replaced: its menu entry goes, and `folio` becomes
# a shortcut to text9000, so scripts that run `folio` keep working. Your
# settings are copied from ~/.config/folio the first time text9000 starts.
set -e
cd "$(dirname "$0")"
PREFIX="${PREFIX:-$HOME/.local}"

remove_folio() {
  rm -f "$PREFIX/share/applications/folio.desktop" \
        "$PREFIX/share/icons/hicolor/256x256/apps/folio.png" "$PREFIX/share/mime/packages/folio.xml"
  rm -rf "$PREFIX/share/folio"
}

if [ "$1" = "--uninstall" ]; then
  rm -f "$PREFIX/bin/text9000" "$PREFIX/share/applications/text9000.desktop" \
        "$PREFIX/share/icons/hicolor/256x256/apps/text9000.png" "$PREFIX/share/mime/packages/text9000.xml"
  # The `folio` shortcut, only if it's ours.
  [ "$(readlink "$PREFIX/bin/folio" 2>/dev/null)" = "text9000" ] && rm -f "$PREFIX/bin/folio"
  command -v update-mime-database >/dev/null && update-mime-database "$PREFIX/share/mime" || true
  rm -rf "$PREFIX/share/text9000"
  echo "Removed text9000. Your settings in ~/.config/text9000 are kept."
  exit 0
fi

mkdir -p "$PREFIX/bin" "$PREFIX/share/text9000" "$PREFIX/share/applications" \
         "$PREFIX/share/icons/hicolor/256x256/apps" "$PREFIX/share/mime/packages"
install -m 755 text9000 "$PREFIX/bin/text9000"
remove_folio
# `folio` keeps working, as a shortcut to text9000.
if [ ! -e "$PREFIX/bin/folio" ] || [ -L "$PREFIX/bin/folio" ] || [ -f "$PREFIX/bin/folio" ]; then
  rm -f "$PREFIX/bin/folio"
  ln -s text9000 "$PREFIX/bin/folio"
fi
rm -rf "$PREFIX/share/text9000/themes"
cp -R themes "$PREFIX/share/text9000/themes"
install -m 644 text9000.png "$PREFIX/share/icons/hicolor/256x256/apps/text9000.png"
install -m 644 text9000.desktop "$PREFIX/share/applications/text9000.desktop"
install -m 644 text9000-mime.xml "$PREFIX/share/mime/packages/text9000.xml"
command -v update-mime-database >/dev/null && update-mime-database "$PREFIX/share/mime" || true
command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" || true
if command -v xdg-mime >/dev/null; then
  for type in application/x-text9000 application/x-folio x-scheme-handler/text9000 x-scheme-handler/folio; do
    xdg-mime default text9000.desktop "$type" || true
  done
fi

echo "Installed text9000 to $PREFIX/bin/text9000. Find it in your app menu, or run: text9000"
# Libraries text9000 needs that minimal systems sometimes lack.
if command -v ldconfig >/dev/null && ! ldconfig -p | grep -q libxkbcommon-x11.so; then
  echo "text9000 also needs libxkbcommon-x11: sudo apt install libxkbcommon-x11-0 (Debian/Ubuntu), sudo dnf install libxkbcommon-x11 (Fedora)."
fi
command -v zenity >/dev/null || command -v kdialog >/dev/null || command -v xmessage >/dev/null \
  || echo "For text9000's questions (like saving before closing), install zenity: sudo apt install zenity (or your system's equivalent)."
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "(Add $PREFIX/bin to your PATH to run it from a terminal.)" ;; esac
