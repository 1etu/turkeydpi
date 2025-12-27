#!/bin/bash
set -e

cd "$(dirname "$0")"

cd .. && cargo build --release 2>/dev/null; cd TurkeyDPI-App
swift build -c release

APP="TurkeyDPI.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

[ -f "../target/release/turkeydpi" ] && cp ../target/release/turkeydpi "$APP/Contents/MacOS/turkeydpi-engine"
cp .build/release/TurkeyDPI "$APP/Contents/MacOS/"
cp Resources/Info.plist "$APP/Contents/"
chmod +x "$APP/Contents/MacOS/"*

echo "Done: $APP"
