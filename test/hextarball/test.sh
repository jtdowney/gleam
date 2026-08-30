#!/bin/sh

# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 The Gleam contributors

set -eu

GLEAM_COMMAND=${GLEAM_COMMAND:-"cargo run --quiet --"}

g() {
  echo "Running: $GLEAM_COMMAND $@"
  $GLEAM_COMMAND "$@"
}

fail() {
  echo "$1" >&2
  exit 1
}

tarball=build/hextarball-0.1.0.tar

expected="gleam.toml
src/external_module.erl
src/hextarball.app.src
src/hextarball.erl
src/hextarball.gleam"

check() {
  echo Checking that the archive holds the files of the project
  files=$(tar -xOf "$tarball" contents.tar.gz | tar -tzf - | sort)
  if [ "$files" != "$expected" ]; then
    fail "Expected the archive to hold:
$expected
but it holds:
$files"
  fi

  echo Checking that the metadata names the files using their archive paths
  metadata=$(tar -xOf "$tarball" metadata.config |
    sed -n 's|^  <<"\(.*\)"/utf8>>,*$|\1|p' | sort)
  if [ "$metadata" != "$files" ]; then
    fail "Expected the metadata to name:
$files
but it names:
$metadata"
  fi
}

echo Resetting the build directory to get to a known state
rm -fr build

echo Exporting a hex tarball should include the files of the project
g export hex-tarball
check

echo
echo Success! 💖
echo
