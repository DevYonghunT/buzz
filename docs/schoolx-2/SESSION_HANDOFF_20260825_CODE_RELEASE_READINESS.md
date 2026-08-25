# SchoolX Code macOS notarization, Intel, and Linux release readiness

검증 기준일: **2026-08-25**

Repository 기준:

- branch: `codex/schoolx-release-readiness-20260825`
- base HEAD: `82e963be68943b8091bde45cdc12e04afa6e806d`
- 대상: 현재 checkout의 tracked working snapshot과 이 문서에 기록한 release-gate 보강.
  시작 전부터 존재한 사용자 수정은 보존하고 release-readiness 변경과 분리했다.

문서 상태: **repository-side gate 보강 완료 — 외부 closure 전에는 전체 release ready 아님**.

이 문서는 다음 문서를 먼저 읽고 수행한 artifact/runtime 검증 기록이다.

- [`AGENTS.md`](../../AGENTS.md)
- [`RELEASING.md`](../../RELEASING.md)
- [`SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY_DECISION.md`](SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY_DECISION.md)

서명 인증서 이름, notarization credential, updater signing key, token 값은 조회 결과에 포함하거나
repository에 기록하지 않았다. 외부 workflow와 Apple notarization 제출도 실행하지 않았다.

## 1. 판정 요약

| Gate | 결과 | 판정 |
|---|---|---|
| 기존 arm64 app/XPC architecture, identifier, Team ID | app/XPC 모두 thin `arm64`, 고정 identifier, 동일 Team `3WPS7QNZV5` | 통과 |
| 기존 arm64 nested signature/entitlements | app deep/strict, XPC strict, repository verifier와 entitlement verifier 통과 | 통과 |
| 기존 arm64 app/DMG Gatekeeper | `Unnotarized Developer ID` | 실패 |
| 기존 arm64 app/DMG stapled ticket | `xcrun stapler validate`가 ticket 없음으로 실패 | 실패 |
| x86_64 sidecar compilation | 다섯 sidecar가 Mach-O `x86_64`로 release build됨 | 부분 통과 |
| x86_64 app/XPC package | Command Line Tools의 Swift compatibility archive에 Intel slice가 없어 link 실패 | 실패 |
| x86_64 signed app/XPC 및 Rosetta task creation | signed artifact가 없어 실행 불가 | 미검증 |
| pinned Ubuntu 24.04 release build | x86_64 release binary와 `.deb` 생성 | 부분 통과 |
| Linux AppImage | Tauri/linuxdeploy GTK plugin 단계 실패 | 실패 |
| Linux descriptor launcher runtime tests | pinned Ubuntu 24.04 amd64 userspace를 emulation한 non-root debug test에서 3/3 통과 | 부분 통과; native/package 미검증 |
| Linux no-silent-fork trace | emulation 밖 arm64 tracer에서 translated x86_64 child의 host-side sequence를 관측 | 진단용 부분 증거; native x86_64 미검증 |
| macOS post-sign artifact gate | thin arch, fixed app/XPC ID와 Team, nested signature/entitlements, app·updater archive에서 추출한 app·DMG의 stapling/Gatekeeper를 release/canary에 연결 | 저장소 계약 통과; 현재 signing action의 final-DMG 계약 때문에 의도적으로 block |
| AppImage build-tool pinning | Tauri CLI 2.11.2가 사용하는 다섯 helper를 SHA-256 고정하고 target-local cache를 build 전후 검증 | 저장소 계약 통과; native package build 대기 |
| Linux runtime fail-closed gate | pinned Ubuntu 24.04 native x86_64, release test binary, non-root, child별 vfork/descriptor order parser와 evidence upload를 CI/release/canary에 연결 | 저장소 계약 통과; upstream native run 대기 |
| updater-enabled release | 필요한 local env unset; repository secret은 조회하지 않음 | 미검증 |

따라서 현재 artifact를 공개 release로 승인하면 안 된다. 저장소가 fail-closed하도록 보강했지만, 최소 해소 조건은
다음 세 가지 external/canonical 실행이다.

1. x86_64 Swift compatibility slice가 있는 selected Apple toolchain에서 Intel app/XPC를 build하고 Developer ID 서명
   검증 및 Rosetta task 생성까지 통과한다.
2. arm64와 x86_64 최종 app/DMG를 notarize/staple하고 Gatekeeper 및 stapler를 최종 산출물에서 통과시킨다.
3. canonical native x86_64 Ubuntu 24.04 gate에서 launcher evidence를 통과시키고, pinned helper로 AppImage
   build/post-process 및 `.deb`/AppImage executable smoke를 완료한다.

### 1.1 이번 보강에서 저장소에 추가한 fail-closed 경계

- macOS release/canary는 최종 signed app, embedded XPC, updater archive, mounted DMG 내부 app을 다시 검사한다.
  app/XPC는 lane별 exact thin architecture, fixed identifier, Team `3WPS7QNZV5`, nested signature와 entitlement를
  모두 만족해야 하며 app, updater archive에서 추출한 app, DMG의 stapled ticket/Gatekeeper 판정이 없으면 upload
  전에 실패한다. Updater archive 파일 자체에 ticket이 붙는다는 뜻은 아니다.
- 고정된 `block/apple-codesign-action` v1.1.0은 DMG에서 app을 추출해 signing service로 보낸 뒤 signed app을 로컬에서
  다시 만든 DMG에 교체한다. 그 final DMG 자체는 codesign/notary submission/staple하지 않는다. 새 gate는 이 출력을
  의도적으로 거부하며, action contract 또는 별도 authorized final-DMG signing lane이 고쳐지기 전에는 macOS release가
  ready가 아니다.
- Linux AppImage helper 다섯 개는 Tauri CLI 2.11.2/tauri-bundler 2.9.2와 결합된 lock의 SHA-256을 만족한 뒤에만
  Cargo target-local `.tauri` cache에 설치된다. 전체 download를 먼저 검증하고, bundling 뒤 Tauri가 만드는 정확한
  deterministic cache 형태까지 다시 검증한다.
- Linux launcher gate는 pinned Ubuntu 24.04 digest/native x86_64/release profile/Rust 1.95/glibc 2.39를 확인하고
  UID/GID 10001로 권한을 낮춘다. 전체 trace에서 thread clone을 제외한 process child는 예상한 Git 두 개만
  허용하고, 각 `/usr/bin/git` child가 `vfork()` 또는
  `clone[3](CLONE_VM|CLONE_VFORK)`를 사용하고 `setpgid -> /proc/self/fd/<fd> chdir -> execve` 순서를 지키는지
  raw syscall evidence로 판정한다. `strace -v`는 CI environment 값 노출을 막기 위해 사용하지 않는다.
- release-profile test compile을 막던 debug-only migration helper는 `test` cfg에서만 추가 포함되도록 고쳤다.
- `RELEASING.md`의 Linux 설명은 관측 ABI requirement `glibc >= 2.39`, 가장 오래된 검증 distro Ubuntu 24.04로
  정정했다. 이는 일반적인 distro support floor 주장과 구분한다.

## 2. 검증 환경과 source 상태

### 2.1 macOS host

- macOS 27.0 (`26A5416b`), Apple Silicon host
- Command Line Tools만 선택됨: `/Library/Developer/CommandLineTools`
- full Xcode 없음
- Swift 6.4
- Rust/Cargo 1.95.0
- `arch -x86_64 uname -m`이 `x86_64`을 반환하여 Rosetta 자체는 사용 가능
- 로컬 signing identity의 이름, 개수, private-key material은 기록하거나 사용하지 않았다.

### 2.2 Linux build environment

release workflow와 같은 digest를 별도 Colima profile에서 사용했다.

```text
ubuntu:24.04@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90
Ubuntu 24.04.4 LTS
amd64 container/userspace on an arm64 Colima VM through Rosetta
glibc 2.39
Rust/Cargo 1.95.0
Node 24.15.0
pnpm 11.4.0
```

검증한 repository-pinned AppImage 후처리 도구:

| Tool | Version/input | SHA-256 |
|---|---|---|
| appimagetool | 1.9.1 | `ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0` |
| type2 runtime | 20251108 | `2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d` |

Base image는 digest-pinned지만 workflow의 `apt-get update`와 package install에는 package version pin이 없다.
따라서 전체 build dependency closure까지 재현 가능하게 고정된 상태는 아니다.

검증용으로 만든 `schoolx-readiness` Colima profile, container, volume은 결과를 host `/tmp`로 복사한 뒤
삭제했다. 해당 VM/build cache는 복구할 수 없으며, `.deb`와 raw trace만 이 문서에 적은 임시 host 경로에 남아 있다.

### 2.3 pre-existing working tree

검증 시작 전부터 다음 수정/미추적 항목이 존재했고 변경하거나 stage하지 않았다.

```text
 M .dockerignore
 M .gitignore
 M crates/buzz-core/src/relay.rs
 M deploy/compose/README.md
 M desktop/src-tauri/src/managed_agents/restore.rs
 M desktop/src-tauri/src/managed_agents/runtime.rs
 M desktop/src-tauri/src/managed_agents/runtime/tests.rs
 M desktop/src-tauri/src/managed_agents/runtime_commands.rs
?? deploy/compose/Dockerfile.local
?? supabase/
```

Linux build에는 tracked working snapshot이 들어갔다. 따라서 생성된 `.deb`는 clean immutable commit의
release artifact가 아니라 이 checkout을 검증한 로컬 증거다.

## 3. macOS arm64 artifact 검증

다음 세 app bundle을 독립적으로 검사했다.

- `/Applications/SchoolX.app`
- `desktop/src-tauri/target/release/bundle/macos/SchoolX.app`
- `desktop/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/SchoolX.app`

세 bundle의 결과는 동일했다.

| 항목 | App | Embedded XPC |
|---|---|---|
| Mach-O architecture | thin `arm64` | thin `arm64` |
| identifier | `io.github.schoolx520.app` | `io.github.schoolx520.app.schoolx-code-git` |
| Team ID | `3WPS7QNZV5` | `3WPS7QNZV5` |
| app/XPC Team equality | app 기준 Team | XPC가 app과 동일 |
| strict signature | `codesign --verify --deep --strict` 통과 | `codesign --verify --strict` 통과 |
| repository signature contract | app/XPC pair를 함께 검사 | `desktop/scripts/verify-code-git-xpc-signature.sh` 통과 |
| entitlements | app/XPC bundle을 함께 검사 | `desktop/scripts/verify-macos-entitlements.sh` 통과 |

`desktop/src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/SchoolX_0.5.3_aarch64.dmg`도
검사했다.

- `hdiutil verify`: 통과
- DMG signature verification: 통과
- read-only mount 내부 app/XPC: thin arm64, 위 identifier/Team, deep/nested signature와 entitlements 모두 통과
- 내부 app Gatekeeper: `Unnotarized Developer ID`
- DMG Gatekeeper: `Unnotarized Developer ID`
- app/DMG `xcrun stapler validate`: ticket 없음으로 실패
- 검증 후 DMG는 정상 detach했다.

따라서 “Developer ID 서명됨”은 확인됐지만 “notarized/stapled release”는 아니다. Debug bundle은
arm64 ad-hoc/no-Team이어서 고정 identifier/Team/signature contract를 통과하지 못했다. 이는 signed release의
negative control로만 기록한다.

## 4. macOS x86_64 build와 Rosetta

### 4.1 완료된 부분

설치된 `x86_64-apple-darwin` Rust target으로 다음 release sidecar를 build하고 bundle staging까지 완료했다.

```sh
. ./bin/activate-hermit
cargo build --release --target x86_64-apple-darwin \
  -p buzz-acp \
  -p buzz-agent \
  -p buzz-dev-mcp \
  -p git-credential-nostr \
  -p buzz-cli
./scripts/bundle-sidecars.sh x86_64-apple-darwin
```

`desktop/src-tauri/binaries/*-x86_64-apple-darwin`의 다섯 파일은 모두 Mach-O `x86_64`이다.
각 파일을 Rosetta로 직접 invoke했을 때 application-level help/config/protocol 결과까지 도달했고, `Bad CPU type`이나
dyld load failure는 없었다. 이는 sidecar loader smoke일 뿐 app/XPC task 생성 증거는 아니다.

### 4.2 app build blocker

다음 workflow-equivalent build를 updater artifact 없이 시도했다.

```sh
. ./bin/activate-hermit
export CMAKE_POLICY_VERSION_MINIMUM=3.5
export MACOSX_DEPLOYMENT_TARGET=10.15
export CMAKE_OSX_DEPLOYMENT_TARGET=10.15
export TAURI_BUNDLER_DMG_IGNORE_CI=true
export SCHOOLX_CODE_GIT_CARGO_LAYOUT=target-triple

cd desktop
pnpm exec tauri build --verbose --no-sign \
  --target x86_64-apple-darwin \
  --bundles app \
  --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

Frontend build와 Rust compilation은 진행됐지만 final link에서 Swift compatibility archive의 Intel slice가 없어
실패했다. 선택된 Command Line Tools의 다음 archive는 `arm64`/`arm64e`만 포함한다.

```text
libswiftCompatibilityConcurrency.a
libswiftCompatibility51.a
libswiftCompatibility56.a
libswiftCompatibilityPacks.a
```

이는 SchoolX source compile error가 아니라 현재 선택된 Apple toolchain이 Intel target을 완성하지 못하는
host blocker다. Full Xcode 설치 자체만으로 충분하다고 가정하지 말고, x86_64 compatibility slice가 있는 Swift
toolchain을 선택한 뒤 같은 build를 다시 실행해야 한다. 일반적인 full Xcode 경로를 선택한 검증 예시는 다음과 같다.

```sh
sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
xcodebuild -version

SWIFT_RUNTIME_DIR="$(
  xcrun swiftc -print-target-info -target x86_64-apple-macosx10.15 |
    jq -r '.paths.runtimeLibraryPaths[] | select(endswith("/macosx"))' |
    head -n 1
)"
test -n "$SWIFT_RUNTIME_DIR"
test -f "$SWIFT_RUNTIME_DIR/libswiftCompatibility51.a"
[[ "$(lipo -archs "$SWIFT_RUNTIME_DIR/libswiftCompatibility51.a")" == *x86_64* ]]
```

마지막 assertion이 통과해야 한다. 실패한 x86_64 intermediate는 disk 회수를 위해 target-specific clean했으며,
위에서 생성한 sidecar staging 파일은 유지했다.

### 4.3 signed artifact와 Rosetta task

x86_64 `.app`가 완성되지 않았으므로 다음 항목은 **미검증**이다.

- x86_64 app/XPC architecture와 fixed identifier
- app/XPC 동일 Team ID 및 nested Developer ID signature
- x86_64 Gatekeeper/notarization/stapling
- Intel-only app을 Rosetta에서 실행한 뒤 embedded XPC를 통해 실제 SchoolX Code task 생성

Rosetta runtime 자체는 정상이다. Closure run에서는 signed Intel artifact를 임시 경로에 풀고, 다른 SchoolX
instance가 없는 clean test user/session에서 다음과 같이 exact artifact를 확인한다.

```sh
set -euo pipefail
: "${APP:?set APP to the absolute SchoolX.app path}"
[[ "$(uname -m)" == arm64 ]]

EXPECTED_TEAM=3WPS7QNZV5
APP_BIN="$APP/Contents/MacOS/buzz-desktop"
XPC="$APP/Contents/XPCServices/io.github.schoolx520.app.schoolx-code-git.xpc"
XPC_BIN="$XPC/Contents/MacOS/schoolx-code-git"

[[ "$(lipo -archs "$APP_BIN")" == x86_64 ]]
[[ "$(lipo -archs "$XPC_BIN")" == x86_64 ]]
codesign --verify --deep --strict --all-architectures "$APP" >/dev/null 2>&1
codesign --verify --strict --all-architectures "$XPC" >/dev/null 2>&1
desktop/scripts/verify-code-git-xpc-signature.sh "$APP"
desktop/scripts/verify-macos-entitlements.sh "$APP"

APP_TEAM="$(codesign -d --verbose=4 "$APP" 2>&1 | sed -n 's/^TeamIdentifier=//p')"
XPC_TEAM="$(codesign -d --verbose=4 "$XPC" 2>&1 | sed -n 's/^TeamIdentifier=//p')"
[[ "$APP_TEAM" == "$EXPECTED_TEAM" ]]
[[ "$XPC_TEAM" == "$EXPECTED_TEAM" ]]

open -n "$APP"
APP_PID=""
for _ in {1..50}; do
  APP_PID="$(pgrep -nf "$APP_BIN" || true)"
  [[ -n "$APP_PID" ]] && break
  sleep 0.2
done
test -n "$APP_PID"
kill -0 "$APP_PID"
OBSERVED_EXECUTABLE="$(ps -ww -p "$APP_PID" -o comm= | sed 's/^[[:space:]]*//')"
[[ "$OBSERVED_EXECUTABLE" == "$APP_BIN" ]]
ps -ww -p "$APP_PID" -o pid=,comm=,command=
```

이 host의 `ps`에는 `arch` output keyword가 없으므로 thin `x86_64` binary가 arm64 host에서 exact executable
path로 살아 있다는 조합으로 Rosetta launch를 판정한다. Intel-only app은 Rosetta에서 자동 실행된다. Clean하고
disposable한 committed test repository를 대상으로 앱에서
Code task 하나를 생성하고, repository inspection/preparation 중 exact `$XPC_BIN` process를 관찰해 PID의 executable
path와 `x86_64` architecture를 확인한다. Preparation, XPC session, fixed `/usr/bin/git` child, task-start 결과까지
성공해야 하며 단순 process launch는 task creation 증거를 대신하지 않는다. Managed task가 worktree와 app state를
변경할 수 있으므로 사용자 작업 repository를 probe 대상으로 쓰면 안 된다.

Canonical upstream에서 signed x86_64를 만드는 공개 경로는 immutable `desktop-v*` release의
`release-macos-x64` job이다. `signed-macos-canary.yml`은 arm64/main-only이고, fork의 Team Build x86_64는
명시적으로 unsigned이므로 대체 증거가 아니다.

## 5. notarization과 stapling

### 5.1 이번 실행 결과

다음 environment variable은 unset이었다. 값이나 다른 credential source는 읽거나 출력하지 않았다.

```text
APPLE_ID
APPLE_PASSWORD
APPLE_TEAM_ID
APPLE_API_KEY
APPLE_API_ISSUER
APPLE_API_KEY_PATH
AC_USERNAME
AC_PASSWORD
AC_TEAM_ID
ASC_KEY_ID
ASC_ISSUER_ID
ASC_KEY_PATH
```

Authorized keychain profile도 제공되거나 검증되지 않았다. 이 확인만으로 local Keychain을 포함한 모든 credential
source가 없다고 단정하지 않는다. 승인된/식별된 accepted submission이 없어 `notarytool submit`, `stapler staple`,
외부 signing service invocation을 실행하지 않았다. `stapler validate`는 현재 attached ticket이 없음을,
`spctl`은 현재 artifact가 `Unnotarized Developer ID`로 평가됨을 각각 증명한다. 과거 online submission 존재 여부를
조회한 결과로 해석하지 않는다.

### 5.2 필요한 authority/secret

로컬 수동 closure에는 다음이 필요하다.

- Team `3WPS7QNZV5`의 유효한 Developer ID Application signing identity와 private key
- Apple notarization용 preconfigured keychain profile, 또는 App Store Connect API key ID/issuer/private-key file
- Apple ID 방식을 쓰는 경우 Apple ID, app-specific password, Team ID
- credential 파일/값을 repository나 shell trace에 남기지 않는 실행 환경

Canonical workflow 경로에는 다음 GitHub Actions secret/permission이 필요하다.

- `OSX_CODESIGN_ROLE`
- `CODESIGN_S3_BUCKET`
- job permission `id-token: write`
- `BUZZ_UPDATER_PUBLIC_KEY` 또는 `SPROUT_UPDATER_PUBLIC_KEY`
- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- protected immutable `desktop-v*` tag를 만들 수 있는 release App/authorized release process
- 새 protected tag를 만드는 정상 release path에는 Actions variable `BUZZ_RELEASE_TAGGER_CLIENT_ID`와 secret
  `BUZZ_RELEASE_TAGGER_PRIVATE_KEY`

실제 Apple credential은 `block/apple-codesign-action` 뒤의 private signing service가 소유하며 public repository에
복사하면 안 된다.

### 5.3 authorized manual closure 명령

아래 `$NOTARY_PROFILE`은 credential 값이 아니라 미리 안전하게 저장한 keychain profile 이름이다. `$EXPECTED_ARCH`는
각 lane에서 `arm64` 또는 `x86_64`로 설정한다. App과 DMG는 각각 제출하고 최종 배포 파일에 ticket을 staple해야 한다.

```bash
set -euo pipefail
: "${NOTARY_PROFILE:?set an authorized keychain profile name}"
: "${EXPECTED_ARCH:?set arm64 or x86_64}"
: "${DEVELOPER_ID_IDENTITY:?set an authorized codesign identity without echoing it}"
: "${TAURI_SIGNING_PRIVATE_KEY:?configure the updater signing key in this shell}"
: "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:?configure the updater key password}"

APP=/absolute/path/SchoolX.app
ZIP=/absolute/path/SchoolX-notary.zip
DMG_TEMPLATE=/absolute/path/unsigned-SchoolX-template.dmg
DMG=/absolute/path/final-SchoolX.dmg
EXPECTED_TEAM=3WPS7QNZV5
NOTARY_EVIDENCE="$(mktemp -d)"
VERIFY_DIR="$(mktemp -d)"
DMG_MOUNT="$(mktemp -d)"
RW_DMG="$NOTARY_EVIDENCE/SchoolX-read-write.dmg"
DMG_DEVICE=""

cleanup() {
  if [[ -n "$DMG_DEVICE" ]]; then
    hdiutil detach "$DMG_DEVICE" >/dev/null 2>&1 || true
  fi
  rm -rf "$NOTARY_EVIDENCE" "$VERIFY_DIR" "$DMG_MOUNT"
}
trap cleanup EXIT

verify_app_signature() {
  TARGET_APP="$1"
  TARGET_APP_BIN="$TARGET_APP/Contents/MacOS/buzz-desktop"
  TARGET_XPC="$TARGET_APP/Contents/XPCServices/io.github.schoolx520.app.schoolx-code-git.xpc"
  TARGET_XPC_BIN="$TARGET_XPC/Contents/MacOS/schoolx-code-git"

  [[ "$(lipo -archs "$TARGET_APP_BIN")" == "$EXPECTED_ARCH" ]]
  [[ "$(lipo -archs "$TARGET_XPC_BIN")" == "$EXPECTED_ARCH" ]]
  codesign --verify --deep --strict --all-architectures "$TARGET_APP" >/dev/null 2>&1
  codesign --verify --strict --all-architectures "$TARGET_XPC" >/dev/null 2>&1
  desktop/scripts/verify-code-git-xpc-signature.sh "$TARGET_APP" >/dev/null 2>&1
  desktop/scripts/verify-macos-entitlements.sh "$TARGET_APP" >/dev/null 2>&1

  APP_TEAM="$(codesign -d --verbose=4 "$TARGET_APP" 2>&1 | sed -n 's/^TeamIdentifier=//p')"
  XPC_TEAM="$(codesign -d --verbose=4 "$TARGET_XPC" 2>&1 | sed -n 's/^TeamIdentifier=//p')"
  [[ "$APP_TEAM" == "$EXPECTED_TEAM" ]]
  [[ "$XPC_TEAM" == "$EXPECTED_TEAM" ]]
}

submit_and_assert_accepted() {
  INPUT="$1"
  RESULT="$2"
  LOG="$3"
  xcrun notarytool submit "$INPUT" \
    --keychain-profile "$NOTARY_PROFILE" \
    --wait --output-format json >"$RESULT"
  jq -e '.status == "Accepted" and (.id | type == "string")' "$RESULT"
  SUBMISSION_ID="$(jq -r '.id' "$RESULT")"
  xcrun notarytool log "$SUBMISSION_ID" "$LOG" \
    --keychain-profile "$NOTARY_PROFILE"
  jq -e '.status == "Accepted" and .statusCode == 0 and ((.issues // []) | length == 0)' "$LOG"
}

verify_app_signature "$APP"

ditto -c -k --keepParent "$APP" "$ZIP"
submit_and_assert_accepted \
  "$ZIP" "$NOTARY_EVIDENCE/app-submit.json" "$NOTARY_EVIDENCE/app-log.json"
xcrun stapler staple "$APP"
xcrun stapler validate "$APP"
verify_app_signature "$APP"
spctl --assess --type execute "$APP" >/dev/null 2>&1

# Stapled app에서 실제 updater archive를 다시 만들고 updater key로 서명한다.
APP_PARENT="$(dirname "$APP")"
ARCHIVE="$APP_PARENT/SchoolX.app.tar.gz"
(cd "$APP_PARENT" && tar -czf "$ARCHIVE" SchoolX.app)
(cd desktop && pnpm tauri signer sign "$ARCHIVE")
test -s "$ARCHIVE.sig"
tar -xzf "$ARCHIVE" -C "$VERIFY_DIR"
ARCHIVE_APP="$VERIFY_DIR/SchoolX.app"
verify_app_signature "$ARCHIVE_APP"
xcrun stapler validate "$ARCHIVE_APP"
spctl --assess --type execute "$ARCHIVE_APP" >/dev/null 2>&1

# Stapled app을 DMG template에 다시 넣고, 그 exact final byte를 Developer ID로
# sign한 뒤 submit/staple한다. Identity 값은 stdout/log에 출력하지 않는다.
hdiutil convert "$DMG_TEMPLATE" -format UDRW -o "$RW_DMG" -ov >/dev/null
ATTACH_OUTPUT="$(hdiutil attach -readwrite -nobrowse -noautoopen \
  -mountpoint "$DMG_MOUNT" "$RW_DMG")"
DMG_DEVICE="$(awk '$1 ~ /^\/dev\// { print $1; exit }' <<<"$ATTACH_OUTPUT")"
test -n "$DMG_DEVICE"
test -d "$DMG_MOUNT/SchoolX.app"
rm -rf "$DMG_MOUNT/SchoolX.app"
ditto "$APP" "$DMG_MOUNT/SchoolX.app"
hdiutil detach "$DMG_DEVICE" >/dev/null
DMG_DEVICE=""
hdiutil convert "$RW_DMG" -format UDZO -imagekey zlib-level=9 \
  -o "$DMG" -ov >/dev/null
codesign --force --timestamp --sign "$DEVELOPER_ID_IDENTITY" \
  "$DMG" >/dev/null 2>&1
codesign --verify --strict "$DMG" >/dev/null 2>&1
submit_and_assert_accepted \
  "$DMG" "$NOTARY_EVIDENCE/dmg-submit.json" "$NOTARY_EVIDENCE/dmg-log.json"
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"
codesign --verify --strict "$DMG" >/dev/null 2>&1
spctl --assess --type open --context context:primary-signature "$DMG" >/dev/null 2>&1

# 최종 app, archive-extracted app, DMG와 mounted app을 credential-safe verifier로 재검증한다.
desktop/scripts/verify-signed-macos-release.sh \
  "$EXPECTED_ARCH" "$APP" "$DMG" "$ARCHIVE"

# Release evidence를 승인된 보관 위치로 복사한 뒤 temporary directory를 제거한다.
```

API-key profile이 아직 없다면 authorized operator가 secure shell에서 다음 placeholder 형태로 한 번 생성한다.
실제 값이나 key file은 문서, logs, shell history, Git에 넣지 않는다.

```sh
xcrun notarytool store-credentials "$NOTARY_PROFILE" \
  --key "$ASC_KEY_PATH" \
  --key-id "$ASC_KEY_ID" \
  --issuer "$ASC_ISSUER_ID"
```

`notarytool` command completion만으로 충분하지 않다. `Accepted` JSON/log assertion, updater archive extraction,
`stapler validate`, codesign/nested signature/architecture, `spctl`이 최종 배포 byte에서 모두 통과해야 한다.

### 5.4 현재 pinned signing action의 final-DMG blocker

고정 commit `679535d1ab7c5a7c18e6f9afcba3464512cc3dde` (`v1.1.0`)의 `action.yml`을 직접 감사했다.
DMG input 처리 순서는 다음과 같다.

1. unsigned DMG를 mount하고 `.app`만 zip으로 signing service에 보낸다.
2. service가 돌려준 signed app zip을 받는다.
3. original DMG를 read-write로 convert하고 app을 교체한 뒤 로컬에서 UDZO DMG를 다시 만든다.
4. 그 final DMG에 대한 `codesign`, `notarytool submit`, `stapler staple`은 action에 없다.

따라서 action의 `signed-dmg-path`라는 output 이름과 달리 최종 DMG byte는 signed/notarized/stapled artifact 계약을
충족하지 않는다. 이번에 추가한 verifier의 DMG codesign/stapler/Gatekeeper gate는 canonical workflow에서도 현재
구조상 실패한다. 이를 약화하면 안 된다.

해결은 다음 중 하나가 필요하다.

- `block/apple-codesign-action` owner가 final rebuilt DMG 자체를 private service에서 Developer ID sign하고 Apple에
  submit한 뒤 staple한 exact byte를 반환하도록 action/service contract를 확장하고 SchoolX workflow가 그 immutable
  commit을 pin한다.
- 또는 Team `3WPS7QNZV5`의 Developer ID identity와 approved notary credential을 가진 별도 authorized lane이
  final DMG에 대해 이 문서 5.3의 codesign/submission/staple/verification 순서를 수행한다.

현재 계정은 `block/apple-codesign-action`과 `block/buzz` 모두 `pull: true`, `push: false`라 이 외부 변경이나
canonical run을 임의로 수행하지 않았다. 필요한 owner 권한은 action repository write/PR merge 권한, private signing
service 변경 권한, 그리고 SchoolX workflow에서 새 immutable action commit을 승인할 권한이다. Secret 값은 이
repository나 요청 응답에 전달하면 안 된다.

## 6. pinned Ubuntu 24.04 desktop/release package

### 6.1 successful build

Pinned Ubuntu 24.04 OCI의 amd64 userspace를 Apple Silicon host에서 emulation한 container에서 다음 sidecar release
build가 성공했다.

```sh
cargo build --release \
  -p buzz-acp \
  -p buzz-agent \
  -p buzz-dev-mcp \
  -p git-credential-nostr \
  -p buzz-cli
./scripts/bundle-sidecars.sh
```

Tauri frontend production build와 release desktop binary도 성공했다. Combined `.deb,appimage` build는 `.deb`를
만든 뒤 AppImage 단계에서 실패했으므로, 같은 target/cache에서 `.deb`만 지정한 isolated packaging command로
다시 확인했다. Clean rebuild라고 주장하지 않는다.

```sh
cd desktop
pnpm tauri build --ci \
  --bundles deb \
  --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

결과:

| 항목 | 값 |
|---|---|
| artifact | `/tmp/schoolx-release-readiness-20260825.0sW10D/SchoolX_0.5.3_amd64.deb` (ephemeral) |
| SHA-256 | `d3ece8cade56d2cd67736817fade7a6004ffbb29cdc687a429424ec055aa7181` |
| package | `school-x` |
| version | `0.5.3` |
| architecture | `amd64` |
| installed size | `192353` KiB |
| declared dependencies | `libwebkit2gtk-4.1-0, libgtk-3-0` |

Package 안의 desktop binary와 다섯 sidecar는 모두 ELF x86-64이고 build environment에서 desktop binary의 `ldd`
dependency가 모두 resolve됐다. Sidecar 다섯 개 전체에 대한 `ldd`/installed-package runtime smoke는 하지 않았다.
관측된 최대 GLIBC symbol version은 다음과 같다.

| Binary | Maximum GLIBC version |
|---|---|
| `buzz` | `GLIBC_2.38` |
| `buzz-acp` | `GLIBC_2.39` |
| `buzz-agent` | `GLIBC_2.39` |
| `buzz-desktop` | `GLIBC_2.39` |
| `buzz-dev-mcp` | `GLIBC_2.39` |
| `git-credential-nostr` | `GLIBC_2.39` |

따라서 관측된 ABI requirement는 glibc 2.39 이상이고, 가장 오래된 실제 검증 distro는 Ubuntu 24.04다. Symbol
version만으로 distro floor 전체를 증명하지 않는다. 기존 `RELEASING.md`의 “Ubuntu 22.04 container”와 “Ubuntu
22.04 or newer” 서술은 실제 workflow 및 이 artifact 증거와 맞지 않았으며 이번 보강에서 정정했다. Ubuntu 22.04
지원을 다시 주장하려면 더 낮은 glibc build floor에서 재빌드하거나 symbol floor를 낮추고, clean installed target에서
desktop과 다섯 sidecar의 dependency/runtime acceptance를 모두 통과해야 한다.

이 `.deb`는 updater artifact 생성을 끈 로컬 release-profile 증거이며 공식 updater-enabled release가 아니다.

### 6.2 AppImage blocker

Tauri `--bundles deb,appimage`는 Tauri가 가져온 linuxdeploy의 GTK plugin 단계에서 exit 2로 실패했다. plugin이
library mode의 linuxdeploy를 재귀 호출하는 지점이며, library를 하나로 줄인 재시도도 같은 실패였다. 이 검증은
Apple Silicon host 위 x86_64 emulation에서 수행했으므로 native x86_64 runner에서도 재현되는 product bug라고
단정하지 않는다. 그러나 성공한 AppImage가 없으므로 `desktop/scripts/fix-appimage.sh` 후처리와 final AppImage
runtime은 미검증이다. 불완전한 AppDir를 임의로 repack하지 않았다.

이 실행에서 추가 supply-chain gap도 확인됐다. 당시 Tauri bundler는 package 시점에 다음을 runtime download했다.

- AppRun
- linuxdeploy binary
- GTK/GStreamer plugin scripts
- `continuous` release의 appimage plugin

이 gap은 저장소 보강에서 닫았다. `desktop/scripts/tauri-appimage-tools-x86_64.lock`은 Tauri CLI 2.11.2가 사용하는
AppRun, linuxdeploy, GTK/GStreamer plugin, appimage plugin 다섯 input을 exact SHA-256으로 고정한다. Script input은
Tauri source commit `499df79be65ef8c0670abc0207cd9e37b55d8491`에 고정했고, release asset URL이 mutable하더라도 hash가
다르면 설치 전에 실패한다. `desktop/scripts/install-tauri-appimage-tools.sh`가 모든 input을 staging/검증한 뒤
Cargo metadata가 반환한 target directory의 `.tauri` cache에 설치하고, release/canary workflow는 bundling 전 설치와
bundling 후 재검증을 모두 수행한다. 이 변경은 supply-chain drift를 fail-closed하지만 native AppImage build 성공을
대신 증명하지는 않는다.

Native x86_64 runner에서 workflow의 pinned image/apt/AppImage-tool provisioning을 그대로 마친 뒤 실행할 core
release command는 다음과 같다. 이 snippet만으로 bare Ubuntu를 provisioning하지는 않는다. appimagetool 1.9.1과
type2 runtime 20251108의 위 SHA-256 검증, `APPIMAGETOOL_RUNTIME_FILE`, exact workflow apt dependency도 먼저
충족해야 한다.

```bash
set -euo pipefail
. ./bin/activate-hermit
: "${VERSION:?set the tag-derived release version}"
: "${BUZZ_UPDATER_PUBLIC_KEY:?provide the authorized updater public key}"
: "${TAURI_SIGNING_PRIVATE_KEY:?provide the authorized updater private key}"
: "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:?provide the updater key password}"
: "${APPIMAGETOOL_RUNTIME_FILE:?point to the hash-verified type2 runtime}"

export BUZZ_UPDATER_ENDPOINT=https://github.com/block/buzz/releases/download/buzz-desktop-latest/latest.json
export CMAKE_POLICY_VERSION_MINIMUM=3.5
export BUZZ_UPDATER_PUBLIC_KEY TAURI_SIGNING_PRIVATE_KEY
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD APPIMAGETOOL_RUNTIME_FILE
just desktop-install-ci

cd desktop
node scripts/set-version-from-tag.mjs "$VERSION"
cd src-tauri
cargo update --workspace
cd ../../..

cargo build --release \
  -p buzz-acp -p buzz-agent -p buzz-dev-mcp -p git-credential-nostr -p buzz-cli
./scripts/bundle-sidecars.sh

desktop/scripts/install-tauri-appimage-tools.sh

cd desktop
node scripts/build-release-config.mjs
pnpm tauri build --verbose --ci \
  --bundles deb,appimage \
  --config src-tauri/tauri.release.conf.json
cd ..

desktop/scripts/install-tauri-appimage-tools.sh --verify-only

mapfile -t APPIMAGES < <(find desktop/src-tauri/target/release/bundle/appimage \
  -name '*.AppImage' -type f)
test "${#APPIMAGES[@]}" -eq 1
bash desktop/scripts/fix-appimage.sh "${APPIMAGES[0]}"
```

`build-release-config.mjs`에는 updater public key와 endpoint가 필요하다. Updater artifact signing에는 private key와
workflow가 제공하는 password secret이 필요하다. Credential 없는 unsigned canary 검증에서는 workflow가 생성하는
`tauri.canary.conf.json`을 사용하며 그것을 signed updater release 증거로 표현하면 안 된다.

## 7. Linux descriptor launcher runtime와 fail-closed 경계

### 7.1 pinned runtime tests

Pinned Ubuntu 24.04 amd64 userspace를 arm64 VM에서 emulation하고 UID 1000 non-root로 Linux descriptor launcher
runtime test를 실행했다. 성공한 test/trace binary는 default debug profile이며, 같은 pinned Rust std를 사용한다는
점에서는 관련 진단이지만 native x86_64 또는 packaged release artifact 증거는 아니다.

```text
3 passed, 0 failed, 0 ignored
```

검증 내용:

- descriptor cwd는 non-stdio FD이며 `FD_CLOEXEC`; regular file은 reject
- 원래 path를 rename하고 replacement directory를 만들어도 열린 directory inode에서 Git 실행
- stdin/capture/process group이 cwd descriptor와 독립

같은 test binary에서 root-trusted executable tests는 4/4, launch-source contract tests는 13/13 통과했다.

초기 `--release --lib` 전체 test compile은 기존 `storage_tests.rs`가 `#[cfg(debug_assertions)]` symbol인
`copy_agent_keys_between_stores`와 `DEV_MIGRATION_MARKER`를 참조해 실패했다. 이를
`#[cfg(any(debug_assertions, test))]`로 제한해 production release surface를 넓히지 않고 release test compilation을
복구했다. 보강 후 macOS host에서 `cargo test --release --lib --no-run`이 성공했고 관련 migration test 6/6도
통과했다. 생성한 `.deb`를 clean install해 packaged desktop에서 실제 Code task를 만드는 probe는 여전히 실행하지
않았다.

### 7.2 syscall evidence

x86_64 strace를 Rosetta-emulated guest에서 직접 실행한 trace는 syscall number를 올바르게 decode하지 못해 증거에서
제외했다. 진단을 위해 같은 OCI index digest의 arm64 platform image에 최소 x86_64 loader/library와 x86_64 Git을
넣고 native arm64 tracer로 translated x86_64 test binary와 child Git의 host-side syscall sequence를 관찰했다.
OCI index digest가 같아도 amd64/arm64 filesystem manifest는 다르므로 동일 native release tuple이라는 뜻은 아니다.

Exact rename/replacement test는 1/1 통과했고 두 Git launch의 sequence는 같았다.

```text
clone(CLONE_VFORK|SIGCHLD)
setpgid(0, 0)
chdir("/proc/self/fd/6")
execve(/usr/bin/git, ...)
```

관측 count:

| Event | Count |
|---|---:|
| plain `fork()` | 0 |
| plain `vfork()` | 0 |
| fork-style `clone` without `CLONE_THREAD`/`CLONE_VFORK` | 0 |
| `clone(CLONE_VFORK\|SIGCHLD)` | 2 |
| `chdir("/proc/self/fd/6")` | 2 |
| `/usr/bin/git` `execve` | 2 |

첫 실행은 bounded admission `git --version`, 둘째는 실제
`hash-object --no-filters -- marker`였다. Raw trace는 `.deb`와 함께 다음 임시 경로에 있다.

```text
/tmp/schoolx-release-readiness-20260825.0sW10D/native-x86git-strace-20260825.*
```

이 파일은 2026-08-25 handoff 작성 시점에는 남아 있지만 `/tmp`의 transient local evidence다. Exact trace invocation,
strace version, parser command, test-binary/Git hash를 함께 보존하지 못했으므로 count를 독립 재감사 가능한 release
record로 취급하지 않는다.

이 count는 관측 사실이지만 native x86_64 syscall 증거는 아니다. Translation layer가 process creation을
중개하거나 host syscall로 변환할 수 있고, 관측된 `CLONE_VFORK|SIGCHLD`에는 `CLONE_VM`도 없었다. 따라서 이 trace로
native x86_64 Rust std spawn backend 또는 no-fork를 확정하지 않는다. Native x86_64 pinned runner에서 동일 test를
직접 `strace -ff`해야 release evidence가 된다.

### 7.3 fail-closed 판정

현재 구현에서 확인한 경계:

- mutation 전에 launcher admission을 수행
- `/proc/self/fd` procfs/magic-link, reopened device/inode, directory type, FD ≥ 3, `FD_CLOEXEC` 검증
- 선택된 exact root-trusted Git의 ownership/writeability/exact identity 재검증 (`/usr/bin/git`이 우선이며,
  이번 trace에서는 `/usr/bin/git`; 검증을 통과한 canonical PATH candidate fallback은 허용)
- cleared environment, bounded `git --version` probe, independent process group
- worktree mutation, Git write, removal claim, Code task start가 authority admission 뒤에만 진행
- uncertain task start를 reusable preparation으로 되돌리지 않고 uncertain 상태로 유지

저장소 보강 후 경계:

- `desktop/scripts/verify-linux-git-launcher-runtime.sh`가 exact Ubuntu image digest, native `x86_64`, dpkg `amd64`,
  glibc 2.39, Rust 1.95.0 x86_64 host와 procfs를 먼저 hard-assert한다.
- release-profile `buzz_lib` test executable을 Cargo JSON에서 하나만 선택하고 ELF x86-64/release path를 확인한다.
- root container에서는 UID/GID 10001과 `no_new_privs`로 권한을 낮춘 뒤 exact rename/replacement test 하나를
  `strace -ff`로 실행한다. Root에서 pass처럼 조기 반환되는 기존 test output을 release 증거로 받아들이지 않는다.
- `desktop/scripts/linux_git_launcher_trace.py`가 각 expected Git child를 creator와 연결한다. Plain `fork()`와
  `CLONE_VM|CLONE_VFORK`가 모두 없는 clone/clone3는 거부하고, child별
  `setpgid -> /proc/self/fd/<non-stdio-fd> chdir -> /usr/bin/git execve` 순서를 강제한다.
- Trace 전체에서 `CLONE_THREAD`가 아닌 추가 process child는 거부하므로 forked intermediate가 다시 vfork로 Git을
  실행해 direct-child 검사만 우회하는 경우도 실패한다.
- CI, Linux canary, 정식 Linux release job이 같은 pinned gate를 실행하고 raw trace, structured verdict,
  environment tuple, test/Git/tool/lockfile hash를 artifact로 보존한다. Trace는 `-v` 없이 수집해 inherited CI
  environment를 펼치지 않는다.

남은 gap은 구현 부재가 아니라 positive native 실행 증거 부재다. 로컬 translated trace는 `CLONE_VFORK`만 있고
`CLONE_VM`이 없어서 새 parser가 의도대로 거부한다. Source가 canonical CI에서 실행된 뒤 evidence artifact의
`verdict=pass`, non-root identity, exact tuple과 hash를 검토해야 이 gate를 닫을 수 있다. 제품 runtime probe 자체는
여전히 Rust/glibc tuple 또는 spawn backend를 매 task마다 검사하지 않으며 `std::process::Command` public contract도
no-fork를 보장하지 않는다. 따라서 toolchain/stdlib/launcher setup 변경 시 canonical gate 재실행이 필수다.

## 8. credential와 upstream-only workflow authority

다음 release environment variable은 unset이었다. Repository/organization secret value나 다른 credential source는
조회하지 않았다.

```text
OSX_CODESIGN_ROLE
CODESIGN_S3_BUCKET
BUZZ_UPDATER_PUBLIC_KEY
SPROUT_UPDATER_PUBLIC_KEY
TAURI_SIGNING_PRIVATE_KEY
TAURI_SIGNING_PRIVATE_KEY_PASSWORD
```

현재 GitHub 권한은 canonical `block/buzz`에 read-only이고 fork에는 admin이었다. 따라서 canonical secret을 사용하는
workflow를 dispatch하거나 protected release tag를 만들 권한이 없다. 외부 실행은 임의로 trigger하지 않았다.

Current source가 review 후 canonical `main`에 merge되고 repository write + Actions workflow-dispatch 권한이 있는
operator만 canary를 실행한다. Fine-grained token이면 해당 repository의 `Actions: write`, classic token이면
`repo` scope가 필요하다.

```sh
gh workflow run signed-macos-canary.yml --repo block/buzz --ref main
gh workflow run linux-canary.yml --repo block/buzz --ref main

gh run list --repo block/buzz --workflow signed-macos-canary.yml --limit 1
gh run list --repo block/buzz --workflow linux-canary.yml --limit 1
: "${RUN_ID:?set the authorized workflow run ID}"
: "${OUTPUT_DIR:?set an artifact output directory}"
gh run watch "$RUN_ID" --repo block/buzz --exit-status
gh run download "$RUN_ID" --repo block/buzz --dir "$OUTPUT_DIR"
```

주의:

- signed macOS canary는 arm64만 검증한다.
- Linux canary는 updater-signing credential 없이 unsigned package를 만든다.
- x86_64 signed macOS와 full updater-enabled Linux를 포함하는 `release.yml`에는 `workflow_dispatch`가 없다.
- `release.yml`은 canonical repository의 authorized release process가 만든 immutable `desktop-v*` tag push로만 실행한다.
- 새 protected tag를 만드는 정상 경로에는 `BUZZ_RELEASE_TAGGER_CLIENT_ID`와
  `BUZZ_RELEASE_TAGGER_PRIVATE_KEY`가 추가로 필요하다. 이미 존재하는 immutable tag의 authorized rerun에는 tag
  creation credential이 필요하지 않다.
- 단순 검증 목적으로 tag를 임의 생성하거나 기존 release를 trigger하면 안 된다.

즉 x86_64 signed closure는 source가 release 승인된 뒤 canonical release candidate/tag 절차 안에서 수행하거나,
release owner가 동일 private signing service를 사용하는 별도 authorized Intel canary lane을 제공해야 한다.

## 9. remediation 상태와 남은 follow-up

저장소에서 완료:

1. macOS arm64/x86_64 release와 signed canary에 exact app/XPC architecture, identifier, Team, nested signature,
   entitlements, stapling, Gatekeeper gate를 연결했다.
2. 최종 DMG를 read-only mount하고 updater archive를 별도 extract해 내부 app/XPC까지 같은 계약으로 재검증한다.
3. Linux native x86_64 non-root release-profile launcher trace/parser와 evidence upload를 CI/release/canary에 연결했다.
4. release test compile의 debug-only migration symbol cfg를 test-only로 고쳤다.
5. Tauri AppImage helper 다섯 개를 SHA-256 lock으로 고정하고 build 전후 검증을 workflow에 연결했다.
6. `RELEASING.md`를 실제 Ubuntu 24.04/glibc 2.39 관측과 일치하도록 정정했다.

외부/canonical 실행 또는 후속 강화가 필요:

1. `block/apple-codesign-action` 또는 별도 authorized lane이 final DMG byte 자체를 sign/notarize/staple하도록
   외부 계약을 고친다. 현재 v1.1.0 output은 새 hard gate를 구조적으로 통과할 수 없다.
2. full Xcode Intel compatibility slice로 signed x86_64 app/XPC를 만들고 Rosetta Code task creation을 수행한다.
3. signing authority가 있는 canonical lane에서 arm64/x86_64 app, updater archive에서 추출한 app, DMG의 positive
   notarization gate를 통과시킨다.
4. native x86_64 Ubuntu 24.04에서 새 launcher gate와 pinned-helper AppImage build를 실제 실행한다.
5. 최종 `.deb`와 AppImage를 clean install/extract해 desktop과 다섯 sidecar 전체의 ELF/GLIBC/dependency/launch smoke를
   수행한다.
6. apt dependency closure는 여전히 package version까지 고정되지 않았다. 더 강한 reproducibility claim에는 snapshot
   repository 또는 resolved package manifest가 필요하다.

## 10. 실행한 관련 regression

macOS host에서 다음 scoped tests가 모두 통과했다.

```text
code_workspace::git_launch_contract_tests                         13 passed
code_workspace::macos_git_xpc::session_lifecycle::tests            5 passed
code_workspace::macos_git_xpc::tests                               9 passed
desktop/scripts/stage-code-git-xpc.test.mjs                         7 passed
root-trusted platform Git exact test                                1 passed
desktop/src/features/code/lib/codeTaskCreation.test.mjs             3 passed
uncertain start reload/recovery exact test                          1 passed
```

Ubuntu 결과는 7절에 기록했다.

보강 후 추가 통과:

```text
macOS release/canary + XPC packaging contract                    pass
current unnotarized arm64 fixture negative gate                  expected fail at stapler
Linux launcher trace/parser/workflow unit tests                 10 passed
desktop Tauri release-profile lib test compile                   pass
storage migration release-profile tests                          6 passed
release config/AppImage tool-lock Node tests                     3 passed
AppImage installer offline install/verify/tamper/atomic contract pass
release reference aggregate contract                             pass
changed workflow YAML parse + changed shell syntax               pass
```

AppImage production input 다섯 개의 remote SHA-256과 linuxdeploy deterministic cache hash도 lock과 독립 대조했다.
이는 input integrity 증거이며 native package build/runtime 증거는 아니다.

## 11. 변경, commit, push

Release readiness를 fail-closed하기 위한 code/workflow/documentation 수정이 필요해 범위별 signed-off commit으로
기록했다.

| Commit | 범위 |
|---|---|
| `09063a888` | macOS app/XPC/DMG/updater archive의 서명·architecture·identifier·Team·stapling·Gatekeeper fail-closed gate |
| `606cd89da` | pinned Ubuntu 24.04 non-root release-profile descriptor launcher runtime/trace gate |
| `c7d8c4c4b` | Tauri AppImage helper input 고정, atomic install, post-bundle integrity gate |

세 commit 모두 `Signed-off-by` trailer를 포함한다. 새 origin branch
`codex/schoolx-release-readiness-20260825`는 `c7d8c4c4b`까지 게시됐다. 이 handoff와 `RELEASING.md` 정정은 이
문서를 포함하는 별도 signed-off documentation commit으로 기록한다.

정상 pre-push hook은 이번 범위와 무관하게 fork `main`과 현재 source 사이에 이미 존재하던 desktop file-size
ratchet 초과 19건에서 실패했다. 이번 범위의 전용 regression과 syntax/contract gate를 별도로 통과시킨 후
`LEFTHOOK=0`으로 새 branch를 게시했다. Hook이나 ratchet 기준은 수정하지 않았다.

Canonical upstream workflow, release tag, notarization submission은 권한 없이 trigger하지 않았다. 기존
working-tree 변경도 stage/commit하지 않고 사용자 소유 상태로 보존했다.
