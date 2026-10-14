#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
name="hedwig-server_${version}_linux_amd64"
binary_dir=${HEDWIG_BIN_DIR:-target/release}
stage="dist/staging/$name"
mkdir -p "$stage/bin" "$stage/web"
for binary in hedwig-server hedwigctl hedwig-agent hedwig-mount; do
  install -m 755 "$binary_dir/$binary" "$stage/bin/$binary"
done
# Replace only the generated web payload when packaging again.
rm -rf "$stage/web"
cp -R web/dist "$stage/web"
cp LICENSE "$stage/"
cp docs/installation-ubuntu.md "$stage/INSTALLATION.md"
tar -czf "dist/${name}.tar.gz" -C dist/staging "$name"
echo "dist/${name}.tar.gz"
