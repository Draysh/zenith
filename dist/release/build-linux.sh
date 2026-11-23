#!/bin/sh
# Build the Linux release file for the version in Cargo.toml:
#   out/zenith-<version>-linux-x86_64.tar.gz
# holding the two programs, the launcher entry and the service.
set -eu
cd "$(dirname "$0")/../.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cargo build --release --locked -p zenith-daemon -p zenith
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
install -m755 target/release/zenith-clips target/release/zenithd "$stage/"
install -m644 dist/zenith-clips.desktop dist/zenithd.service "$stage/"
mkdir -p out
tar -czf "out/zenith-$version-linux-x86_64.tar.gz" -C "$stage" zenith-clips zenithd zenith-clips.desktop zenithd.service
echo "out/zenith-$version-linux-x86_64.tar.gz"
