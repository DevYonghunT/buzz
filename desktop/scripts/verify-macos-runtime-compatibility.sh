#!/usr/bin/env bash
set -euo pipefail

# Verify the loader-facing macOS contract independently from code signing and
# notarization. A valid signature cannot rescue an app whose deployment target
# makes Swift resolve through a missing compatibility path.

xpc_identifier=io.github.schoolx520.app.schoolx-code-git
plist_minimum=10.15
expected_arch=
macho_minimum=

fail() {
  echo "$1" >&2
  exit 1
}

version_matches() {
  local observed=$1 expected=$2
  [[ "$observed" == "$expected" || "$observed" == "$expected.0" ]]
}

verify_plist_minimum() {
  local plist_path=$1 label=$2 observed
  [[ -f "$plist_path" && ! -L "$plist_path" ]] ||
    fail "$label is missing a regular Info.plist"
  observed=$(
    /usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' \
      "$plist_path" 2>/dev/null
  ) || fail "$label has no LSMinimumSystemVersion"
  version_matches "$observed" "$plist_minimum" ||
    fail "$label LSMinimumSystemVersion must be $plist_minimum; found $observed"
}

binary_architectures() {
  /usr/bin/lipo -archs "$1" 2>/dev/null | /usr/bin/awk '{$1=$1; print}'
}

parse_macho_deployment_target() {
  /usr/bin/awk '
    function finish_deployment_command() {
      if (!active) return
      if (value_count != 1) invalid = 1
      if (kind == "build" && (platform_count != 1 || platform != "1")) {
        invalid = 1
      }
      if (value_count == 1) observed = value
      active = 0
    }

    $1 == "Load" && $2 == "command" {
      finish_deployment_command()
      next
    }

    $1 == "cmd" && $2 == "LC_BUILD_VERSION" {
      finish_deployment_command()
      if (NF != 2) invalid = 1
      commands += 1
      active = 1
      kind = "build"
      platform = ""
      platform_count = 0
      value = ""
      value_count = 0
      next
    }

    $1 == "cmd" && $2 == "LC_VERSION_MIN_MACOSX" {
      finish_deployment_command()
      if (NF != 2) invalid = 1
      commands += 1
      active = 1
      kind = "legacy"
      platform = ""
      platform_count = 0
      value = ""
      value_count = 0
      next
    }

    active && kind == "build" && $1 == "platform" {
      if (NF != 2) invalid = 1
      platform = $2
      platform_count += 1
      next
    }

    active && kind == "build" && $1 == "minos" {
      if (NF != 2) invalid = 1
      value = $2
      value_count += 1
      next
    }

    active && kind == "legacy" && $1 == "version" {
      if (NF != 2) invalid = 1
      value = $2
      value_count += 1
      next
    }

    END {
      finish_deployment_command()
      if (invalid || commands != 1) exit 64
      print observed
    }
  '
}

macho_minimums() {
  /usr/bin/otool -l "$1" 2>/dev/null | parse_macho_deployment_target
}

parse_otool_install_names() {
  /usr/bin/awk '
    NR == 1 { next }
    {
      line = $0
      sub(/^[[:space:]]+/, "", line)
      metadata = " \\(compatibility version [^(),]+, current version [^(),]+(, [^(),]+)*\\)$"
      if (!match(line, metadata) || RSTART <= 1) {
        invalid = 1
        next
      }
      print substr(line, 1, RSTART - 1)
      names += 1
    }
    END {
      if (invalid || NR < 2 || names == 0) exit 64
    }
  '
}

parse_otool_rpaths() {
  /usr/bin/awk '
    function finish_rpath_command() {
      if (!active) return
      if (path_count != 1) invalid = 1
      active = 0
    }

    $1 == "Load" && $2 == "command" {
      finish_rpath_command()
      next
    }

    $1 == "cmd" && $2 == "LC_RPATH" {
      finish_rpath_command()
      if (NF != 2) invalid = 1
      active = 1
      path_count = 0
      next
    }

    active && $1 == "path" {
      line = $0
      sub(/^[[:space:]]*path[[:space:]]+/, "", line)
      if (!match(line, / \(offset [0-9]+\)$/) || RSTART <= 1) {
        invalid = 1
        next
      }
      print substr(line, 1, RSTART - 1)
      path_count += 1
      next
    }

    END {
      finish_rpath_command()
      if (invalid) exit 64
    }
  '
}

system_swift_rpath_in_load_commands() {
  parse_otool_rpaths | /usr/bin/awk '
    $0 == "/usr/lib/swift" { found = 1 }
    END { exit(found ? 0 : 1) }
  '
}

swift_dependency_kind() {
  local dependency=$1
  if [[ "$dependency" =~ ^/usr/lib/swift/libswift[[:alnum:]_.+-]*\.dylib$ ]]; then
    printf '%s\n' system
  elif [[ "$dependency" =~ ^@rpath/libswift[[:alnum:]_.+-]*\.dylib$ ]]; then
    printf '%s\n' rpath
  elif [[ "$dependency" == *libswift* ]]; then
    printf '%s\n' unsupported
  else
    printf '%s\n' other
  fi
}

system_swift_rpath_present() {
  /usr/bin/otool -l "$1" 2>/dev/null | system_swift_rpath_in_load_commands
}

verify_swift_loads() {
  local binary_path=$1 label=$2 dependencies dependency dependency_kind swift_count=0
  dependencies=$(
    /usr/bin/otool -L "$binary_path" 2>/dev/null | parse_otool_install_names
  ) || fail "could not inspect $label dynamic libraries"

  while IFS= read -r dependency; do
    [[ -n "$dependency" ]] || continue
    dependency_kind=$(swift_dependency_kind "$dependency")
    case "$dependency_kind" in
      system)
        swift_count=$((swift_count + 1))
        ;;
      rpath)
        swift_count=$((swift_count + 1))
        system_swift_rpath_present "$binary_path" ||
          fail "$label has an unresolved $dependency without LC_RPATH /usr/lib/swift"
        ;;
      unsupported)
        fail "$label loads Swift from an unsupported path: $dependency"
        ;;
    esac
  done <<EOF
$dependencies
EOF

  [[ "$swift_count" -gt 0 ]] || fail "$label has no observable Swift runtime dependency"
}

verify_binary() {
  local binary_path=$1 label=$2 architectures minimums minimum_count observed_minimum
  [[ -f "$binary_path" && ! -L "$binary_path" && -x "$binary_path" ]] ||
    fail "$label is missing, linked, or not executable"

  architectures=$(binary_architectures "$binary_path") ||
    fail "could not inspect $label architecture"
  [[ "$architectures" == "$expected_arch" ]] ||
    fail "$label must be thin $expected_arch; found ${architectures:-unknown}"

  minimums=$(macho_minimums "$binary_path") ||
    fail "could not inspect $label deployment target"
  minimum_count=$(
    printf '%s\n' "$minimums" | /usr/bin/awk 'NF { count++ } END { print count + 0 }'
  )
  [[ "$minimum_count" -eq 1 ]] ||
    fail "$label must contain exactly one macOS deployment target; found $minimum_count"
  observed_minimum=$(printf '%s\n' "$minimums" | /usr/bin/awk 'NF { print; exit }')
  version_matches "$observed_minimum" "$macho_minimum" ||
    fail "$label deployment target must be $macho_minimum; found $observed_minimum"

  verify_swift_loads "$binary_path" "$label"
}

main() {
  local app_path app_info app_executable_name xpc_path
  if [[ $# -ne 2 ]]; then
    echo "Usage: $0 <SchoolX.app> <arm64|x86_64>" >&2
    exit 2
  fi

  app_path=$1
  expected_arch=$2
  case "$expected_arch" in
    arm64) macho_minimum=11.0 ;;
    x86_64) macho_minimum=10.15 ;;
    *)
      echo "unsupported expected macOS architecture: $expected_arch" >&2
      exit 2
      ;;
  esac

  [[ -d "$app_path" && ! -L "$app_path" ]] ||
    fail "app must be a non-symlink bundle: $app_path"

  app_info="$app_path/Contents/Info.plist"
  app_executable_name=$(
    /usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app_info" 2>/dev/null
  ) || fail "app has no CFBundleExecutable"
  [[ "$app_executable_name" =~ ^[0-9A-Za-z._+-]+$ ]] ||
    fail "app has an unsafe CFBundleExecutable"

  xpc_path="$app_path/Contents/XPCServices/${xpc_identifier}.xpc"
  [[ -d "$xpc_path" && ! -L "$xpc_path" ]] ||
    fail "app is missing its regular SchoolX Code Git XPC bundle"

  verify_plist_minimum "$app_info" "app"
  verify_plist_minimum "$xpc_path/Contents/Info.plist" "SchoolX Code Git XPC"
  verify_binary "$app_path/Contents/MacOS/$app_executable_name" "app executable"
  verify_binary "$xpc_path/Contents/MacOS/schoolx-code-git" "SchoolX Code Git XPC executable"

  echo "Verified $expected_arch macOS deployment target and Swift runtime contract"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
