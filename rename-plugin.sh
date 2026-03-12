#!/usr/bin/env bash
#
# Rename the template plugin from "abc" to a user-chosen name.
#
# Usage:  ./rename-plugin.sh <name>
#
# Example:
#   ./rename-plugin.sh xyz
#
# This will rename test_abc_* to test_xyz_*, and replace every occurrence of
# "abc" (in the plugin context) with "xyz" in Cargo.toml, src/*.rs,
# CMakeLists.txt, and test files.

set -euo pipefail

if [ $# -ne 1 ]; then
    echo "Usage: $0 <plugin-name>" >&2
    exit 1
fi

name="$1"

# Sanity-check: the name should be a valid C/Rust identifier fragment (lowercase
# alphanumeric + underscores, must not start with a digit).
if ! [[ "$name" =~ ^[a-z_][a-z0-9_]*$ ]]; then
    echo "Error: plugin name must match [a-z_][a-z0-9_]* (got \"$name\")" >&2
    exit 1
fi

cd "$(dirname "$0")"

# Detect the current plugin name from Cargo.toml (lib name = "na_plugin_<name>")
old_name=$(sed -n 's/^name = "na_plugin_\(.*\)"/\1/p' Cargo.toml | head -1)
if [ -z "$old_name" ]; then
    echo "Error: could not detect current plugin name from Cargo.toml" >&2
    exit 1
fi

if [ "$old_name" = "$name" ]; then
    echo "Plugin is already named \"$name\", nothing to do."
    exit 0
fi

echo "Renaming plugin: $old_name -> $name"

# 1. Replace inside top-level file contents
for f in Cargo.toml CMakeLists.txt; do
    if [ -f "$f" ]; then
        sed -i "s/${old_name}/${name}/g" "$f"
        echo "  updated $f"
    fi
done

# 2. Replace inside Rust source files
for f in src/lib.rs src/plugin.rs; do
    if [ -f "$f" ]; then
        sed -i "s/${old_name}/${name}/g" "$f"
        echo "  updated $f"
    fi
done

# 3. Replace inside test file contents
if [ -d test ]; then
    for f in test/CMakeLists.txt test/test_${old_name}_*.c; do
        if [ -f "$f" ]; then
            sed -i "s/${old_name}/${name}/g" "$f"
            echo "  updated $f"
        fi
    done

    # 4. Rename test source files
    for f in test/test_${old_name}_*.c; do
        if [ -f "$f" ]; then
            new_f="${f/test_${old_name}_/test_${name}_}"
            mv "$f" "$new_f"
            echo "  renamed $(basename "$f") -> $(basename "$new_f")"
        fi
    done
fi

echo "Done. Remember to remove any old build directory before rebuilding."
