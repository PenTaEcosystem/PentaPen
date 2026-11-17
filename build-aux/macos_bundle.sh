#!/usr/bin/env bash
# Packs a meson install of the app into PentaPen.app and a .dmg (unsigned: ad-hoc signature only).
# Usage: macos_bundle.sh <install prefix> <app version> <output dmg>
# Needs Homebrew with: gtk4 libadwaita adwaita-icon-theme librsvg dylibbundler
set -euo pipefail

prefix="$1"
version="$2"
dmg="$3"
brew_prefix="$(brew --prefix)"
src_root="$(cd "$(dirname "$0")/.." && pwd)"

work="$(mktemp -d)"
app="$work/PentaPen.app"
contents="$app/Contents"
macos="$contents/MacOS"
frameworks="$contents/Frameworks"
res="$contents/Resources"
loaders="$res/lib/gdk-pixbuf-2.0/2.10.0/loaders"
mkdir -p "$macos" "$frameworks" "$res/share" "$res/etc" "$loaders"

echo "### binary"
cp "$prefix/bin/rnote" "$macos/rnote"

echo "### image loaders"
brew_loaders="$brew_prefix/lib/gdk-pixbuf-2.0/2.10.0/loaders"
cp "$brew_loaders"/*.so "$loaders/"
# Query the Homebrew copies (the bundled ones only load from inside the app) and point the paths
# into the app; loader paths must be absolute, so the launcher fills in where the app actually is
gdk-pixbuf-query-loaders "$brew_loaders"/*.so     | sed "s|$brew_loaders|@APP@/Contents/Resources/lib/gdk-pixbuf-2.0/2.10.0/loaders|g"     > "$res/etc/loaders.cache.in"
grep -q "@APP@" "$res/etc/loaders.cache.in"

echo "### libraries"
fix_args=(-x "$macos/rnote")
for so in "$loaders"/*.so; do fix_args+=(-x "$so"); done
dylibbundler -of -b "${fix_args[@]}" \
    -d "$frameworks" -p @executable_path/../Frameworks/ \
    -s "$brew_prefix/lib"
for so in "$loaders"/*.so; do
    install_name_tool -id "@executable_path/../Resources/lib/gdk-pixbuf-2.0/2.10.0/loaders/$(basename "$so")" "$so"
done

# dyld refuses a library that lists the same LC_RPATH twice, which dylibbundler can leave behind
rpaths() { otool -l "$1" | awk '/cmd LC_RPATH/{r=1} r&&/ path /{print $2; r=0}'; }
while IFS= read -r -d '' f; do
    file "$f" | grep -q Mach-O || continue
    rpaths "$f" | sort | uniq -d | while IFS= read -r dup; do
        while [ "$(rpaths "$f" | grep -cxF "$dup")" -gt 1 ]; do
            install_name_tool -delete_rpath "$dup" "$f"
        done
    done
done < <(find "$macos" "$frameworks" "$res/lib" -type f -print0)

echo "### data"
cp -R "$prefix/share/." "$res/share/"
mkdir -p "$res/share/icons/hicolor" "$res/share/glib-2.0/schemas"
cp -R "$brew_prefix/share/icons/Adwaita" "$res/share/icons/"
cp "$brew_prefix/share/icons/hicolor/index.theme" "$res/share/icons/hicolor/"
# Links that point nowhere outside Homebrew would still be listed in the icon cache
find -L "$res/share" -type l -print0 | while IFS= read -r -d '' link; do
    echo "drop broken link: $link"
    rm -f "$link"
done
cp "$brew_prefix"/share/glib-2.0/schemas/org.gtk.*.xml "$res/share/glib-2.0/schemas/"
glib-compile-schemas "$res/share/glib-2.0/schemas"
for theme in "$res/share/icons/Adwaita" "$res/share/icons/hicolor"; do
    gtk4-update-icon-cache -q -t -f "$theme" || true
done

echo "### launcher"
cat > "$macos/PentaPen" <<'LAUNCHER'
#!/bin/sh
here="$(cd "$(dirname "$0")" && pwd)"
app="$(cd "$here/../.." && pwd)"
cache="${TMPDIR:-/tmp}/pentapen-loaders.cache"
sed "s|@APP@|$app|g" "$app/Contents/Resources/etc/loaders.cache.in" > "$cache"
export GDK_PIXBUF_MODULE_FILE="$cache"
exec "$here/rnote" "$@"
LAUNCHER
chmod +x "$macos/PentaPen"

echo "### icon"
iconset="$work/PentaPen.iconset"
mkdir -p "$iconset"
svg="$src_root/crates/rnote-ui/data/icons/scalable/apps/rnote.svg"
for size in 16 32 128 256 512; do
    rsvg-convert -w "$size" -h "$size" "$svg" -o "$iconset/icon_${size}x${size}.png"
    rsvg-convert -w $((size * 2)) -h $((size * 2)) "$svg" -o "$iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$iconset" -o "$res/PentaPen.icns"

cat > "$contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>PentaPen</string>
    <key>CFBundleDisplayName</key><string>PentaPen</string>
    <key>CFBundleIdentifier</key><string>com.github.flxzt.rnote</string>
    <key>CFBundleExecutable</key><string>PentaPen</string>
    <key>CFBundleIconFile</key><string>PentaPen.icns</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>${MACOSX_DEPLOYMENT_TARGET:-15.0}</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

echo "### check: nothing may still point into Homebrew"
leftover=0
while IFS= read -r -d '' f; do
    if file "$f" | grep -q Mach-O && otool -L "$f" | tail -n +2 | grep -q "$brew_prefix"; then
        echo "still links to Homebrew: $f"
        otool -L "$f" | grep "$brew_prefix"
        leftover=1
    fi
    if file "$f" | grep -q Mach-O && [ -n "$(rpaths "$f" | sort | uniq -d)" ]; then
        echo "duplicate LC_RPATH: $f"
        leftover=1
    fi
done < <(find "$macos" "$frameworks" "$res/lib" -type f -print0)
[ "$leftover" = 0 ]

echo "### ad-hoc signature (Apple Silicon refuses unsigned code)"
# Each library on its own, then the app (--deep would treat the loader folders as bundles)
while IFS= read -r -d '' f; do
    if file "$f" | grep -q Mach-O; then codesign --force --sign - "$f"; fi
done < <(find "$frameworks" "$res/lib" "$macos/rnote" -type f -print0)
codesign --force --sign - "$app"
codesign --verify --deep "$app"

echo "### dmg"
stage="$work/dmg"
mkdir -p "$stage"
mv "$app" "$stage/"
ln -s /Applications "$stage/Applications"
cp "$src_root/build-aux/macos-HUONG-DAN.txt" "$stage/HUONG-DAN-MO-APP.txt"
hdiutil create -volname "PentaPen $version" -srcfolder "$stage" -ov -format UDZO "$dmg"
echo "### done: $dmg"
