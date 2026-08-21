#!/usr/bin/env bash
set -euo pipefail

# This is deliberately verification-only. If the external signing action did
# not sign the nested XPC with the app's team, the release must fail here.

app_path=${1:?usage: verify-code-git-xpc-signature.sh /path/to/SchoolX.app}
app_identifier=io.github.schoolx520.app
xpc_identifier=io.github.schoolx520.app.schoolx-code-git
xpc_path="$app_path/Contents/XPCServices/${xpc_identifier}.xpc"
xpc_executable="$xpc_path/Contents/MacOS/schoolx-code-git"

if [[ ! -d "$app_path" || -L "$app_path" ]]; then
  echo "expected a non-symlink app bundle at $app_path" >&2
  exit 1
fi
if [[ ! -d "$xpc_path" || -L "$xpc_path" ]]; then
  echo "signed app is missing the fixed Code Git XPC bundle at $xpc_path" >&2
  exit 1
fi
if [[ ! -f "$xpc_executable" || -L "$xpc_executable" || ! -x "$xpc_executable" ]]; then
  echo "Code Git XPC entrypoint is missing, linked, or not executable: $xpc_executable" >&2
  exit 1
fi

/usr/bin/codesign --verify --strict --verbose=2 "$xpc_path"

signature_metadata() {
  /usr/bin/codesign --display --verbose=4 "$1" 2>&1
}

metadata_value() {
  local metadata=$1 key=$2 value count
  value=$(printf '%s\n' "$metadata" | /usr/bin/awk -F= -v key="$key" '$1 == key { print substr($0, index($0, "=") + 1) }')
  count=$(printf '%s\n' "$value" | /usr/bin/awk 'NF { count += 1 } END { print count + 0 }')
  if [[ "$count" -ne 1 ]]; then
    echo "signature metadata must contain exactly one non-empty $key" >&2
    return 1
  fi
  printf '%s\n' "$value"
}

app_metadata=$(signature_metadata "$app_path")
xpc_metadata=$(signature_metadata "$xpc_path")
signed_app_identifier=$(metadata_value "$app_metadata" Identifier)
signed_xpc_identifier=$(metadata_value "$xpc_metadata" Identifier)
app_team=$(metadata_value "$app_metadata" TeamIdentifier)
xpc_team=$(metadata_value "$xpc_metadata" TeamIdentifier)

if [[ "$signed_app_identifier" != "$app_identifier" ]]; then
  echo "unexpected signed app identifier: $signed_app_identifier" >&2
  exit 1
fi
if [[ "$signed_xpc_identifier" != "$xpc_identifier" ]]; then
  echo "unexpected signed Code Git XPC identifier: $signed_xpc_identifier" >&2
  exit 1
fi
if [[ ! "$app_team" =~ ^[0-9A-Za-z]+$ || ! "$xpc_team" =~ ^[0-9A-Za-z]+$ || "$app_team" != "$xpc_team" ]]; then
  echo "app and Code Git XPC require the same non-empty TeamIdentifier" >&2
  exit 1
fi

developer_id_requirement() {
  local identifier=$1 team=$2
  printf '%s' \
    "anchor apple generic and identifier \"${identifier}\" and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"${team}\""
}

app_requirement=$(developer_id_requirement "$app_identifier" "$app_team")
xpc_requirement=$(developer_id_requirement "$xpc_identifier" "$xpc_team")
/usr/bin/codesign --verify --strict --verbose=2 -R="$app_requirement" "$app_path"
/usr/bin/codesign --verify --strict --verbose=2 -R="$xpc_requirement" "$xpc_path"

plist_identifier=$(
  /usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$xpc_path/Contents/Info.plist"
)
if [[ "$plist_identifier" != "$xpc_identifier" ]]; then
  echo "unexpected Code Git XPC Info.plist identifier: $plist_identifier" >&2
  exit 1
fi

echo "Verified signed Code Git XPC identity and TeamIdentifier in $app_path"
