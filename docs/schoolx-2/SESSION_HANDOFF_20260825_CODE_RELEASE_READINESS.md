# SchoolX Code macOS notarization, Intel, and Linux release readiness

검증 기준일: **2026-08-25**

Repository 기준:

- branch: `codex/schoolx-release-readiness-20260825`
- base HEAD: `82e963be68943b8091bde45cdc12e04afa6e806d`
- 대상: 현재 checkout의 tracked working snapshot과 이 문서에 기록한 release-gate 보강.
  시작 전부터 존재한 사용자 수정은 보존하고 release-readiness 변경과 분리했다.

문서 상태: **repository-side gate와 승인된 local macOS app/DMG artifact closure 완료 — updater/canonical Linux closure 전에는 전체 release ready 아님**.

이 문서는 다음 문서를 먼저 읽고 수행한 artifact/runtime 검증 기록이다.

- [`AGENTS.md`](../../AGENTS.md)
- [`RELEASING.md`](../../RELEASING.md)
- [`SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY_DECISION.md`](SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY_DECISION.md)

서명 인증서 이름, notarization credential, updater signing key, token 값은 조회 결과에 포함하거나
repository에 기록하지 않았다. Canonical workflow와 release tag는 실행하지 않았지만, 사용자 승인 하에 local
arm64/x86_64 app과 final DMG를 Apple에 제출하고 결과를 검증했다.

## 1. 판정 요약

| Gate | 결과 | 판정 |
|---|---|---|
| 기존 arm64 app/XPC architecture, identifier, Team ID | app/XPC 모두 thin `arm64`, 고정 identifier, 동일 Team `3WPS7QNZV5` | 통과 |
| 기존 arm64 nested signature/entitlements | app deep/strict, XPC strict, repository verifier와 entitlement verifier 통과 | 통과 |
| 기존 arm64 runtime compatibility | 설치 app의 `LSMinimumSystemVersion`이 `10.13`; 새 runtime verifier가 negative control로 거부 | 기존 artifact 실패; corrected app 별도 통과 |
| corrected arm64 signed app/XPC | 7개 Mach-O thin `arm64`; plist `10.15`, platform `1`, minos `11.0`, system Swift, 고정 ID/Team, nested signature/entitlements/runtime/timestamp | 통과 |
| 기존 arm64 app/DMG Gatekeeper·ticket | 최초 설치 artifact는 `Unnotarized Developer ID`, ticket 없음 | historical negative control |
| x86_64 sidecar compilation | 다섯 sidecar가 Mach-O `x86_64`로 release build되고 signed app의 7-file closure에서도 재검증됨 | 통과 |
| x86_64 app/XPC package | Xcode 27 beta에서 app/XPC와 bundled Mach-O 7개 모두 thin `x86_64`; plist `10.15`, platform `1`, minos `10.15`, system Swift load | 통과 |
| x86_64 signed app/XPC | 고정 ID, 동일 Team `3WPS7QNZV5`, nested signature, entitlements, hardened runtime, timestamp, runtime verifier | 통과 |
| x86_64 Rosetta | clean Rosetta Git, translated app launch, exact embedded XPC task creation과 direct `/usr/bin/git` child 관측 | 통과 |
| arm64/x86_64 app notarization | 양 app `Accepted`, log `statusCode=0`, `issues: null`(0건), app staple/stapler/Gatekeeper/runtime/signature | 통과 |
| arm64/x86_64 final DMG | stapled app으로 재조립·서명한 exact DMG를 각각 제출; `Accepted`, `statusCode=0`, `issues: null`(0건), staple/stapler/Gatekeeper와 mounted-app 통합 verifier | 통과 |
| x86_64 pre-notary DMG | 서명/structure/mounted app은 통과했지만 ticket과 Gatekeeper는 실패했던 별도 byte | historical negative control |
| pinned Ubuntu 24.04 release build | 과거 emulated updater-disabled/cached 진단에서 `.deb`를 만들었지만, 최신 pinned-digest QEMU 재시도는 GCC crash로 release-profile compile을 완료하지 못함 | 부분 통과; 새 package와 native clean build/runtime 미검증 |
| Linux AppImage | 과거 emulated build는 GTK plugin에서 실패했고, 최신 QEMU 재시도는 packaging 전에 중단되어 새 AppImage 없음 | 미검증; native runner 재실행 필요 |
| Linux descriptor launcher runtime tests | 과거 emulated non-root debug test 3/3 통과; 최신 release-profile gate는 compile 전에 GCC가 crash하여 test/trace 없음 | 부분 통과; canonical x64 release-profile evidence 대기 |
| Linux no-silent-fork trace | emulation 밖 arm64 tracer에서 translated x86_64 child의 host-side sequence를 관측 | 진단용 부분 증거; native x86_64 미검증 |
| macOS post-sign artifact gate | thin arch, fixed app/XPC ID와 Team, nested signature/entitlements, app·updater archive에서 추출한 app·DMG의 stapling/Gatekeeper를 release/canary에 연결 | 저장소 계약 통과; 현재 signing action의 final-DMG 계약 때문에 의도적으로 block |
| AppImage build-tool pinning | 최신 QEMU 진단에서도 pinned appimagetool/type2 runtime과 Tauri helper 다섯 개의 install/verify 통과 | 입력 무결성 계약 통과; package/AppImage build 대기 |
| Linux runtime fail-closed gate | reported Ubuntu/x86_64 tuple, workflow `runner.arch=X64`, release test binary, non-root, child별 vfork/descriptor order parser와 evidence upload를 CI/release/canary에 연결 | 저장소 계약 통과; canonical x64 run 대기 |
| updater-enabled release | 필요한 local env unset; repository secret은 조회하지 않음 | 미검증 |

따라서 Downloads에 보존한 local macOS DMG는 artifact-level positive evidence지만, 현재 source를 canonical 공개
release로 승인하면 안 된다. macOS architecture/signature/notarization과 Rosetta XPC probe는 닫혔고, 남은 최소
해소 조건은 다음 external/canonical 실행이다.

1. Canonical macOS lane에서 final DMG byte 자체의 sign/notarize/staple 계약을 고치고, updater signing secret으로
   만든 updater archive에서 추출한 app까지 최종 verifier를 통과시킨다. Signed x86_64 provenance는 정상
   `desktop-v*` release 절차에서 다시 확인한다.
2. Canonical native x86_64 Ubuntu 24.04 gate에서 launcher evidence를 통과시키고, pinned helper로 AppImage
   build/post-process 및 `.deb`/AppImage executable smoke를 완료한다.

### 1.1 이번 보강에서 저장소에 추가한 fail-closed 경계

- macOS release/canary는 최종 signed app, embedded XPC, updater archive, mounted DMG 내부 app을 다시 검사한다.
  app/XPC는 lane별 exact thin architecture, fixed identifier, Team `3WPS7QNZV5`, nested signature와 entitlement를
  모두 만족해야 하며 app, updater archive에서 추출한 app, DMG의 stapled ticket/Gatekeeper 판정이 없으면 upload
  전에 실패한다. Updater archive 파일 자체에 ticket이 붙는다는 뜻은 아니다.
- 고정된 `block/apple-codesign-action` v1.1.0은 DMG에서 app을 추출해 signing service로 보낸 뒤 signed app을 로컬에서
  다시 만든 DMG에 교체한다. 그 final DMG 자체는 codesign/notary submission/staple하지 않는다. 새 gate는 이 출력을
  의도적으로 거부하며, action contract 또는 canonical release process에 통합된 authorized final-DMG signing lane이
  고쳐지기 전에는 macOS release가 ready가 아니다.
- base Tauri config는 app plist floor를 `10.15`로 고정한다. XPC staging과 최종 signed-artifact verifier는 thin
  architecture뿐 아니라 app/XPC plist, 정확히 하나인 macOS deployment command, modern command의 platform `1`,
  lane별 Mach-O minos(`x86_64=10.15`, `arm64=11.0`)를 검사한다. Swift install-name은 공백을 포함한 전체 문자열을
  파싱하며 한 path component의 `/usr/lib/swift/libswift*.dylib` 또는 system Swift rpath가 있는
  `@rpath/libswift*.dylib`만 허용한다. host Xcode 절대경로, 하위경로 우회, 모호하거나 상충하는 load command는
  fail-closed한다.
- Linux AppImage helper 다섯 개는 Tauri CLI 2.11.2/tauri-bundler 2.9.2와 결합된 lock의 SHA-256을 만족한 뒤에만
  Cargo target-local `.tauri` cache에 설치된다. 전체 download를 먼저 검증하고, bundling 뒤 Tauri가 만드는 정확한
  deterministic cache 형태까지 다시 검증한다.
- Linux launcher gate는 pinned Ubuntu 24.04 reference, guest x86_64/release profile/Rust 1.95/glibc 2.39와 workflow가
  제공한 `runner.arch=X64`를 확인하고
  UID/GID 10001로 권한을 낮춘다. 전체 trace에서 thread clone을 제외한 process child는 예상한 Git 두 개만
  허용하고, 각 `/usr/bin/git` child가 `vfork()` 또는
  `clone[3](CLONE_VM|CLONE_VFORK)`를 사용하고 `setpgid -> /proc/self/fd/<fd> chdir -> execve` 순서를 지키는지
  raw syscall evidence로 판정한다. `strace -v`는 CI environment 값 노출을 막기 위해 사용하지 않는다.
- release-profile test compile을 막던 debug-only migration helper는 `test` cfg에서만 추가 포함되도록 고쳤다.
- `RELEASING.md`의 Linux 설명은 emulated Ubuntu 24.04 amd64 userspace에서 관측한 exact-byte ABI requirement
  `glibc >= 2.39`로 정정했다. 이를 native clean-install 또는 distro support floor로 표현하지 않는다.

## 2. 검증 환경과 source 상태

### 2.1 macOS host

- macOS 27.0 (`26A5416b`), Apple Silicon host
- global `xcode-select`: `/Users/kim-yonghun/Downloads/Xcode-beta.app/Contents/Developer`
- full Xcode 27.0, build `27A5252f`
- Swift 6.4
- Rust/Cargo 1.95.0
- `arch -x86_64 uname -m`이 `x86_64`을 반환하여 Rosetta 자체는 사용 가능
- Xcode license와 first-launch setup은 완료됐다. 환경을 비운 Rosetta 실행의 `/usr/bin/git`과 §4.2의 네 Swift
  compatibility archive `lipo` assertion이 모두 통과했다.
- 로컬 signing identity의 표시명, 개수, private-key material은 기록하거나 출력하지 않았다. 승인된 로컬 서명은
  아래 corrected arm64/x86_64 app과 양 architecture final DMG artifact 생성에만 사용했다.

### 2.2 Linux build environment

release workflow와 같은 OCI reference의 amd64 userspace를 Apple Silicon의 Colima/QEMU에서 사용했다. 최신 closure
시도 중 Colima VM은 임시로 `4 CPU / 8 GiB`를 사용했다. Guest가 `x86_64`을 보고하더라도 native x64 runner나
workflow-equivalent provenance는 아니다.

```text
ubuntu:24.04@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90
Ubuntu 24.04.4 LTS
amd64 container/userspace under QEMU on an arm64 Colima VM
glibc 2.39
Rust/Cargo 1.95.0
Node 24.15.0
pnpm 11.4.0
just 1.46.0
```

Repository Hermit 경로는 QEMU에서 Go 1.26 runtime crash로 실행할 수 없었다. 진단을 계속하기 위해 Rust/Cargo
1.95.0, Node 24.15.0, pnpm 11.4.0, just 1.46.0을 exact version으로 수동 설치했다. Version tuple은 의도한
toolchain과 맞지만 Hermit activation과 binary provenance가 다르므로 workflow-equivalent build라고 주장하지 않는다.
`pnpm install --frozen-lockfile`은 통과했다.

검증한 repository-pinned AppImage 후처리 도구:

| Tool | Version/input | SHA-256 |
|---|---|---|
| appimagetool | 1.9.1 | `ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0` |
| type2 runtime | 20251108 | `2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d` |

두 input의 SHA-256 검증과 설치가 통과했고, Tauri가 사용하는 pinned AppImage helper 다섯 개도 target-local cache에
설치한 뒤 `--verify-only` 재검증을 통과했다. 이는 input/cache integrity 증거이며 package 또는 AppImage 생성 증거는
아니다.

Base image는 digest-pinned지만 workflow의 `apt-get update`와 package install에는 package version pin이 없다.
따라서 전체 build dependency closure까지 재현 가능하게 고정된 상태는 아니다.

최신 실행의 credential-free build log와 provisioning evidence는 다음 임시 경로로 복사했다.

```text
/tmp/schoolx-linux-emulation-diagnostic.Aor2im
```

검증에 사용한 exact container는 제거했다. Colima는 원래 상태인 stopped `aarch64`, `2 CPU / 2 GiB`로 복원했다.
위 evidence와 과거 `.deb`/trace 경로는 모두 `/tmp`의 transient local diagnostic이며 canonical 보관소가 아니다.

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

과거 `.deb` build에는 tracked working snapshot이 들어갔다. 최신 QEMU 재시도는 clean clone의
`f5b514808`을 base로 사용한 뒤 현재 launcher gate script만 별도 복사했으므로 이 역시 단일 immutable commit의
workflow-equivalent build가 아니다. 두 실행의 결과 모두 canonical release artifact가 아니라 로컬 진단 증거다.

## 3. macOS arm64 artifact 검증

기존 `/Applications/SchoolX.app`과 그 signed DMG는 app/XPC의 thin `arm64`, 고정 identifier, 동일 Team
`3WPS7QNZV5`, deep/nested signature와 entitlement 계약을 통과했다. 그러나 app plist가 `10.13`이어서 새
`desktop/scripts/verify-macos-runtime-compatibility.sh`가 `LSMinimumSystemVersion=10.15` negative control로
거부한다. 기존 DMG도 이 app을 포함하므로 corrected runtime/notarization positive evidence가 아니다.

새 base config와 Xcode 27을 적용해 다섯 sidecar와 Tauri app/XPC를 다시 release build했다. 서명 전 runtime gate를
통과한 bundle을 별도 임시 경로로 복사한 뒤, 기존에 검증된 Intel app과 동일한 leaf certificate/private identity를
출력하지 않는 방식으로 5개 sidecar -> XPC -> app 순서로 명시적 서명했다. App만 committed
`desktop/src-tauri/Entitlements.plist`를 사용했고 XPC/sidecar에는 entitlement를 추가하지 않았다.

Corrected signed app은 `/tmp/schoolx-arm64-release.7EL2CB/SchoolX.app`이다.

| 항목 | 결과 |
|---|---|
| app/XPC architecture | 둘 다 thin `arm64` |
| bundled Mach-O closure | app executable, XPC executable, bundled sidecar 5개로 구성된 7개 모두 thin `arm64` |
| identifiers | app `io.github.schoolx520.app`, XPC `io.github.schoolx520.app.schoolx-code-git` |
| Team/signing identity | app/XPC/sidecar 모두 Team `3WPS7QNZV5`; 기준 Intel app과 leaf DER byte equality 확인 |
| nested signing | app deep/strict, XPC와 sidecar strict 검증 통과 |
| entitlement/signing options | repository entitlement contract, hardened runtime, secure timestamp 통과 |
| plist floor | app/XPC 모두 `10.15` |
| Mach-O runtime contract | app/XPC 모두 platform `1`, minos `11.0`; Swift dependency는 system path |
| repository runtime verifier | `desktop/scripts/verify-macos-runtime-compatibility.sh ... arm64` 통과 |
| historical pre-notary state | app `stapler validate` exit `65`, `spctl` exit `3` |
| final notarization state | Apple `Accepted`, log `statusCode=0`, `issues: null`(0건); staple/stapler/Gatekeeper와 signature/runtime 재검증 통과 |

App submission 전에 credential-free notarization transport archive를 만들었다. 다음 hash는 pre-staple ZIP input의
hash이며 최종 DMG hash가 아니다.

| 항목 | 값 |
|---|---|
| artifact | `/tmp/schoolx-arm64-release.7EL2CB/SchoolX_0.5.3_arm64-notary-input.zip` |
| SHA-256 | `835eef3a7bf3a885fe410ff65d0327494628de07483101cb829f524a1ee03ffa` |

이 exact app은 Apple `Accepted` 뒤 ticket을 staple했고 `stapler validate`, `spctl --assess --type execute`, nested
signature/entitlement/runtime verifier를 다시 통과했다. 이 stapled app으로 만든 final arm64 DMG 결과는 §5.1에
기록한다. Debug bundle은 arm64 ad-hoc/no-Team이며 signed release의 negative control로만 취급한다.

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

### 4.2 historical app build blocker와 해소

최초에는 global `xcode-select`가 가리키는 Command Line Tools로 다음 workflow-equivalent build를 updater artifact
없이 시도했다.

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
실패했다. 당시 선택된 Command Line Tools의 다음 archive는 `arm64`/`arm64e`만 포함했다.

```text
libswiftCompatibilityConcurrency.a
libswiftCompatibility51.a
libswiftCompatibility56.a
libswiftCompatibilityPacks.a
```

이 historical failure는 SchoolX source compile error가 아니라 당시 선택된 toolchain의 host blocker였다. 이후
`/Users/kim-yonghun/Downloads/Xcode-beta.app`을 설치하고, 먼저 검증 명령에만 `DEVELOPER_DIR`를 지정해 archive
내용을 확인했다.

```sh
export DEVELOPER_DIR=/Users/kim-yonghun/Downloads/Xcode-beta.app/Contents/Developer
xcodebuild -version

SWIFT_RUNTIME_DIR="$(
  xcrun swiftc -print-target-info -target x86_64-apple-macosx10.15 |
    jq -r '.paths.runtimeLibraryPaths[] | select(endswith("/macosx"))' |
    head -n 1
)"
test -n "$SWIFT_RUNTIME_DIR"
for archive in \
  libswiftCompatibilityConcurrency.a \
  libswiftCompatibility51.a \
  libswiftCompatibility56.a \
  libswiftCompatibilityPacks.a; do
  test -f "$SWIFT_RUNTIME_DIR/$archive"
  [[ "$(lipo -archs "$SWIFT_RUNTIME_DIR/$archive")" == "x86_64 arm64 arm64e" ]]
done
```

선택한 toolchain은 Xcode 27.0 build `27A5252f`였고, 위 네 archive가 모두 `x86_64 arm64 arm64e`여서 이 blocker는
해소됐다. 동일 target의 app/XPC build와 후속 서명도 완료했다. 이후 사용자 승인으로 global developer directory도
`/Users/kim-yonghun/Downloads/Xcode-beta.app/Contents/Developer`로 전환했다. 다음처럼 환경을 비운 Rosetta
프로세스에서도 system Git이 정상 실행되어 XPC와 동일한 global toolchain 전제를 확인했다.

```sh
env -i PATH=/usr/bin:/bin arch -x86_64 /usr/bin/git --version
```

첫 Intel link 성공 뒤에는 별도의 repository config 결함도 발견됐다. base Tauri config가 minimum을 생략해 app/XPC
Mach-O와 app plist가 `10.13`으로 만들어졌고 Swift가 `@rpath/libswiftCore.dylib`를 참조하면서 system Swift rpath가
없어 Rosetta launch가 dyld 오류로 종료됐다. base `minimumSystemVersion=10.15`와 pre-bundle/final runtime gate를
추가해 이를 닫았다. corrected build는 platform `1`, minos `10.15`, `/usr/lib/swift` install-name 계약을 통과한다.

### 4.3 signed artifact와 Rosetta task

검증한 ephemeral signed app은 `/tmp/schoolx-x86-release.JxyLzN/SchoolX.app`이다. 서명 인증서 표시명이나 credential은
출력하지 않았으며 결과는 다음과 같다.

| 항목 | 결과 |
|---|---|
| app/XPC architecture | 둘 다 thin `x86_64` |
| bundled Mach-O closure | app executable, XPC executable, bundled sidecar 5개로 구성된 7개 모두 thin `x86_64` |
| identifiers | app `io.github.schoolx520.app`, XPC `io.github.schoolx520.app.schoolx-code-git` |
| Team | app/XPC 모두 `3WPS7QNZV5` |
| nested signing | app deep/strict 및 XPC strict 검증 통과 |
| entitlement/signing options | repository entitlement contract, hardened runtime, timestamp 확인 통과 |
| plist floor | app/XPC 모두 `10.15` |
| Mach-O runtime contract | app/XPC 모두 platform `1`, minos `10.15`; Swift dependency는 system path |
| repository runtime verifier | `desktop/scripts/verify-macos-runtime-compatibility.sh ... x86_64` 통과 |

이 thin Intel app을 Apple Silicon host에서 실행해 translated execution과 exact app executable의 bounded liveness를
확인했다. 이어 기존 사용자 checkout과 분리한 disposable empty committed repository에서 SchoolX Code task를
생성했다. 관측 결과는 다음과 같다.

| 항목 | 결과 |
|---|---|
| translated app launch | 통과 |
| exact embedded XPC | `/Applications/SchoolX.app/Contents/XPCServices/io.github.schoolx520.app.schoolx-code-git.xpc/Contents/MacOS/schoolx-code-git` 관측 |
| fixed Git child | XPC의 direct child로 `/usr/bin/git` 관측 |
| task materialization | 파일을 읽거나 변경하지 않는 no-op prompt에 `OK` 응답 후 완료 |
| task lifecycle | task archive 완료; 사용자 확인 뒤 UI native removal 완료, transcript 보존 receipt 확인 |

Probe용 repository에는 사용자 파일을 넣지 않았고 remote는 credential/userinfo/query/fragment가 없는 검증된
HTTP(S) origin만 복제했다. 사용자 checkout은 probe 대상으로 사용하지 않았다. 사용자 확인 직후 SchoolX UI의
native removal만 사용해 exact managed worktree를 제거했고, UI에서 execution root 제거와 transcript 보존을
확인했다. 이후 원래 signed arm64 `/Applications/SchoolX.app`, 사용자 `~/.schoolx/REPOS` inode, 사전 백업한 WebKit
localStorage를 복원했다. LocalStorage는 backup byte와 SHA-256 equality 및 SQLite integrity `ok`를 확인했고 app/XPC는
원래처럼 종료 상태로 두었다.

Canonical upstream에서 signed x86_64를 만드는 공개 경로는 immutable `desktop-v*` release의
`release-macos-x64` job이다. `signed-macos-canary.yml`은 arm64/main-only이고, fork의 Team Build x86_64는
명시적으로 unsigned이므로 대체 증거가 아니다. 위 local signed artifact도 canonical release/notarization 증거를
대체하지 않는다.

## 5. notarization과 stapling

### 5.1 이번 실행 결과

초기 probe에서 다음 environment variable은 unset이었다. 값이나 다른 credential source는 읽거나 출력하지 않았다.

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

이후 사용자가 App Store Connect API private key를 local Keychain profile `SCHOOLX_NOTARY`에 저장했고
`notarytool store-credentials --validate`가 성공했다. Profile 이름 외의 key ID, issuer, private-key path/content와
signing identity 표시명은 출력하거나 문서에 기록하지 않았다. 이 승인된 local profile과 기존 Developer ID private
identity를 사용해 arm64/x86_64 app과 final DMG를 각각 제출했다. Canonical workflow나 외부 signing service는
실행하지 않았다.

App submission input은 다음과 같다. ZIP은 pre-staple transport byte이며 배포 artifact가 아니다.

| Architecture | ZIP | SHA-256 |
|---|---|---|
| arm64 | `/tmp/schoolx-arm64-release.7EL2CB/SchoolX_0.5.3_arm64-notary-input.zip` | `835eef3a7bf3a885fe410ff65d0327494628de07483101cb829f524a1ee03ffa` |
| x86_64 | `/tmp/schoolx-x86-release.JxyLzN/SchoolX_0.5.3_x64-notary-input.zip` | `e922340d56aa54e46ef0b00ed88f33a773132143893cc2406e89b9c4fac113e5` |

네 submission의 Apple log는 모두 `status=Accepted`, `statusCode=0`, `issues: null`(0건)이었다. App 두 개는
ticket을 staple한 뒤 `stapler validate`,
`spctl --assess --type execute`, strict nested signature/entitlement/runtime verifier를 다시 통과했다.

Stapled app을 DMG template에 다시 넣어 별도 exact final DMG를 만들고, DMG 자체를 Developer ID로 서명한 다음
제출·staple했다. 최종 배포 파일은 다음 Downloads 경로에 보존했다.

| Architecture | Final DMG | post-staple SHA-256 |
|---|---|---|
| arm64 | `/Users/kim-yonghun/Downloads/SchoolX-release-readiness-20260825/SchoolX_0.5.3_arm64.dmg` | `83bcc9bf7f8af041bca3a09082ef3c96047340011cf2d26847909c218e6f03c5` |
| x86_64 | `/Users/kim-yonghun/Downloads/SchoolX-release-readiness-20260825/SchoolX_0.5.3_x64.dmg` | `f6c853a1fed7dd161c4df8c86b3ff1ea35dc4d63617fd4989bac8e34429ade9a` |

두 DMG 모두 `hdiutil verify`, strict DMG signature, `stapler validate`, Gatekeeper open assessment를 통과했고,
`desktop/scripts/verify-signed-macos-release.sh`가 mounted app의 exact thin architecture, fixed app/XPC identifier와
Team, nested signature/entitlement/runtime/ticket까지 재검증했다. Downloads copy는 `/tmp`의 verified source와
SHA-256이 일치한다.

Historical negative control인
`/tmp/schoolx-x86-final.K9SHE4/SchoolX_0.5.3_x64-signed-unnotarized.dmg`
(`35f8ccac85f592af9ae74bd72f2498bd78af4215eb29c29514146525e9b15717`)은 final DMG와 다른 byte다. 이 파일은
서명/structure/mounted app은 통과했지만 app/DMG ticket이 없고 Gatekeeper가 거부했다. 최종 positive evidence와
혼동하면 안 된다.

Credential-safe raw evidence는 mode `0700`인 `/tmp/schoolx-notary-evidence.e7tg7M`에만 두었고 repository에
복사하지 않았다. Temporary log는 canonical evidence storage가 아니며 updater archive는 이번 local closure에
포함되지 않았다.

### 5.2 필요한 authority/secret

로컬 app/DMG closure의 재실행에는 다음이 필요하다. 이번 실행에서는 사용자 승인으로 앞의 두 항목을 사용할 수
있었고, 실제 값은 출력하지 않았다.

- Team `3WPS7QNZV5`의 유효한 Developer ID Application signing identity와 private key
- Apple notarization용 preconfigured keychain profile, 또는 App Store Connect API key ID/issuer/private-key file
- Apple ID 방식을 쓰는 경우 Apple ID, app-specific password, Team ID
- credential 파일/값을 repository나 shell trace에 남기지 않는 실행 환경

Updater archive 생성·서명에 필요한 `TAURI_SIGNING_PRIVATE_KEY`와 password는 제공되지 않았으므로 updater-enabled
positive evidence는 만들지 않았다.

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

Canonical workflow가 사용하는 Apple credential은 `block/apple-codesign-action` 뒤의 private signing service가
소유하며 public repository에 복사하면 안 된다.

### 5.3 authorized manual closure 명령

아래는 새 release byte를 위한 재현용 runbook이다. 이번에 Accepted/stapled된 exact app/DMG를 다시 제출하는 명령이
아니다. `$NOTARY_PROFILE`은 credential 값이 아니라 미리 안전하게 저장한 keychain profile 이름이고,
`$EXPECTED_ARCH`는 각 lane에서 `arm64` 또는 `x86_64`로 설정한다. App과 DMG는 각각 제출하고 최종 배포 파일에
ticket을 staple해야 한다.

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
  desktop/scripts/verify-macos-runtime-compatibility.sh \
    "$TARGET_APP" "$EXPECTED_ARCH" >/dev/null

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

다른 authorized operator에게 API-key profile이 없다면 secure shell에서 다음 placeholder 형태로 한 번 생성한다.
이번 host의 profile은 이미 검증됐으므로 재실행할 필요가 없다. 실제 값이나 key file은 문서, logs, shell history,
Git에 넣지 않는다.

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

이번 승인된 local lane은 stapled app으로 양 architecture DMG를 재조립하고 final byte 자체를
sign/notarize/staple해 verifier를 통과시켰다. 이는 artifact-level contract가 실행 가능함을 증명하지만, pinned
canonical action의 구현이나 provenance를 바꾸지 않으므로 canonical blocker를 대신하지 않는다.

해결은 다음 중 하나가 필요하다.

- `block/apple-codesign-action` owner가 final rebuilt DMG 자체를 private service에서 Developer ID sign하고 Apple에
  submit한 뒤 staple한 exact byte를 반환하도록 action/service contract를 확장하고 SchoolX workflow가 그 immutable
  commit을 pin한다.
- 또는 Team `3WPS7QNZV5`의 Developer ID identity와 approved notary credential을 가진, canonical release process에
  통합된 authorized lane이 final DMG에 대해 이 문서 5.3의 codesign/submission/staple/verification 순서를 수행한다.

현재 계정은 `block/apple-codesign-action`과 `block/buzz` 모두 `pull: true`, `push: false`라 이 외부 변경이나
canonical run을 임의로 수행하지 않았다. 필요한 owner 권한은 action repository write/PR merge 권한, private signing
service 변경 권한, 그리고 SchoolX workflow에서 새 immutable action commit을 승인할 권한이다. Secret 값은 이
repository나 요청 응답에 전달하면 안 된다.

## 6. pinned Ubuntu 24.04 desktop/release package

### 6.1 historical emulated diagnostic package

이전 pinned Ubuntu 24.04 OCI 진단에서는 amd64 userspace를 Apple Silicon host에서 emulation하고 다음 sidecar
release build를 성공시켰다.

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

따라서 이 exact bytes에서 관측한 필요 symbol floor는 glibc 2.39다. 이는 emulated Ubuntu 24.04 build userspace의
정적 ABI 증거일 뿐, Ubuntu 24.04의 native clean-install 검증이나 일반적인 distro support floor가 아니다. 기존
`RELEASING.md`의 “Ubuntu 22.04 container”와 “Ubuntu 22.04 or newer” 서술은 실제 workflow와 맞지 않았으며 이번
보강에서 정정했다. 특정 distro 지원을 주장하려면 명시적으로 pinned된 sysroot/build floor를 사용하고, 각 native
target에서 최종 package의 desktop과 다섯 sidecar dependency/runtime acceptance를 모두 통과해야 한다.

이 `.deb`는 updater artifact 생성을 끈 과거 로컬 release-profile 증거이며 공식 updater-enabled release가 아니다.
최신 QEMU 재시도는 이 artifact를 재현하거나 대체하지 못했다.

### 6.2 latest pinned-digest QEMU attempt

최신 시도에서는 §2.2의 pinned image/input과 수동 exact-version toolchain으로 dependency install 및 AppImage helper
provisioning까지 완료했다. Launcher gate가 요구하는 release-profile `buzz_lib` test compilation은 다음 두 지점에서
QEMU 아래 GCC가 crash했다.

1. 최초 병렬 build에서 `aws-lc-sys` compilation 중 GCC가 `SIGSEGV`로 종료됐다.
2. `CARGO_BUILD_JOBS=1` 재시도에서는 `schemars_derive` link 중 GCC internal compiler error와 `SIGSEGV`가 발생했다.

Release-profile test executable이 만들어지기 전에 compiler가 종료됐으므로 launcher test와 `strace`는 시작되지
않았다. Sidecar/Tauri packaging에도 도달하지 못해 이 실행에서 새 `.deb`, AppImage, launcher trace는 생성되지 않았다.
서로 다른 compile/link 지점에서 발생한 crash와 QEMU 실행 조건을 고려해 이 결과는 emulator/toolchain execution
limitation으로 분류한다. 이를 SchoolX source, launcher 또는 packaging의 product failure로 주장하지 않는다. 관련
credential-free log는 `/tmp/schoolx-linux-emulation-diagnostic.Aor2im`에 있으며 positive closure에는 사용하지 않는다.

### 6.3 historical AppImage blocker and current state

과거 Tauri `--bundles deb,appimage` 진단은 Tauri가 가져온 linuxdeploy의 GTK plugin 단계에서 exit 2로 실패했다.
plugin이 library mode의 linuxdeploy를 재귀 호출하는 지점이며, library를 하나로 줄인 재시도도 같은 실패였다. 최신
시도는 §6.2의 compiler crash로 packaging에 도달하지 못했다. 둘 다 Apple Silicon 위 x86_64 emulation에서 수행했으므로
native x86_64 runner에서도 재현되는 product bug라고 단정하지 않는다. 성공한 AppImage가 없으므로
`desktop/scripts/fix-appimage.sh` 후처리와 final AppImage runtime은 미검증이다. 불완전한 AppDir를 임의로 repack하지
않았다.

과거 packaging 실행에서 추가 supply-chain gap도 확인됐다. 당시 Tauri bundler는 package 시점에 다음을 runtime
download했다.

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
대신 증명하지는 않는다. 최신 QEMU 진단에서 build 전 install과 `--verify-only`가 실제로 통과했지만 packaging 후
재검증은 실행할 package build 자체가 없어 수행되지 않았다.

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

최신 pinned-digest QEMU 시도는 release-profile test binary를 만들려 했지만 §6.2의 GCC crash로 compilation을
완료하지 못했다. 따라서 이번 시도에는 새 launcher test result나 raw/structured trace가 없으며, 과거 debug test를
release-profile positive evidence로 승격하지 않는다.

### 7.2 historical syscall evidence

아래 syscall 관측은 이전 진단의 historical evidence이며 최신 §6.2 실행에서는 새 trace가 생성되지 않았다. x86_64
strace를 Rosetta-emulated guest에서 직접 실행한 trace는 syscall number를 올바르게 decode하지 못해 증거에서
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

- `desktop/scripts/verify-linux-git-launcher-runtime.sh`가 exact Ubuntu image reference, guest `x86_64`, dpkg `amd64`,
  glibc 2.39, Rust 1.95.0 x86_64 host, procfs와 workflow-provided `runner.arch=X64`를 먼저 hard-assert한다.
  `runner.arch` binding은 ARM64 runner의 우발적 emulation을 차단하지만 물리 hardware나 비에뮬레이션을 attestation하지
  않는다. Native provenance는 canonical runner/job metadata와 함께 판단한다.
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

Evidence upload는 `if: always()`이고 파일 부재도 warning이므로 artifact 존재 자체가 pass 증거는 아니다. Canonical
run의 exact SHA/ref와 workflow/job conclusion `success`, `trace-verdict.json`의 `verdict=pass`, runner metadata와
environment/hash/raw trace를 함께 확인해야 한다.

남은 gap은 구현 부재가 아니라 positive native 실행 증거 부재다. 과거 local translated trace는 `CLONE_VFORK`만 있고
`CLONE_VM`이 없어서 새 parser가 의도대로 거부하며, 최신 QEMU 재시도는 compiler crash로 trace 자체를 만들지 못했다.
Source가 canonical upstream `main`의 native X64 runner에서 실행된 뒤 evidence artifact의 `verdict=pass`, non-root
identity, exact tuple과 hash를 검토해야 이 gate를 닫을 수 있다. 제품 runtime probe 자체는 여전히 Rust/glibc tuple
또는 spawn backend를 매 task마다 검사하지 않으며 `std::process::Command` public contract도 no-fork를 보장하지
않는다. 따라서 toolchain/stdlib/launcher setup 변경 시 canonical gate 재실행이 필수다.

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

Local arm64/x86_64 app/DMG는 notarization까지 완료됐지만 canonical provenance와 updater-enabled release를 대신하지
않는다. 최종 canonical x86_64 closure는 source가 release 승인된 뒤 정상 release candidate/tag 절차 안에서
수행하거나, release owner가 동일 private signing service를 사용하는 별도 authorized Intel canary lane을 제공해야
한다.

## 9. remediation 상태와 남은 follow-up

저장소에서 완료:

1. macOS arm64/x86_64 release와 signed canary에 exact app/XPC architecture, identifier, Team, nested signature,
   entitlements, stapling, Gatekeeper gate를 연결했다.
2. 최종 DMG를 read-only mount하고 updater archive를 별도 extract해 내부 app/XPC까지 같은 계약으로 재검증한다.
   base plist `10.15`, Mach-O platform/minos, strict Swift install-name/system rpath gate도 XPC staging과 signed app,
   updater-extracted app, mounted app에 연결했다.
3. Linux non-root release-profile launcher trace/parser와 evidence upload를 CI/release/canary에 연결하고, workflow
   `runner.arch=X64` binding으로 ARM64 runner의 우발적 emulation을 차단했다.
4. release test compile의 debug-only migration symbol cfg를 test-only로 고쳤다.
5. Tauri AppImage helper 다섯 개를 SHA-256 lock으로 고정하고 build 전후 검증을 workflow에 연결했다.
6. `RELEASING.md`를 emulated Ubuntu 24.04 userspace의 glibc 2.39 정적 관측과 native/distro support 경계를 정확히
   구분하도록 정정했다.

최신 로컬 QEMU 진단에서 dependency install, pinned AppImage input 검증, helper install/verify는 통과했다. 그러나
release-profile compile은 `aws-lc-sys` GCC `SIGSEGV`와 단일-job 재시도의 `schemars_derive` GCC internal compiler
error/`SIGSEGV`로 완료되지 않았고, 새 package/AppImage/launcher trace는 없다. 이 로컬 결과는 emulator limitation이며
product failure나 native release evidence로 사용하지 않는다.

승인된 local macOS closure에서 추가 완료:

1. Full Xcode를 global developer directory로 선택하고 clean Rosetta `/usr/bin/git`과 네 Swift compatibility archive
   slice를 확인했다.
2. Signed x86_64 app을 Rosetta로 실행해 exact embedded XPC를 통한 disposable Code task와 fixed `/usr/bin/git`
   direct child를 확인하고 task를 archive했다.
3. Corrected arm64/x86_64 app과 stapled app으로 재조립한 final DMG를 각각 Apple에 제출했다. 네 log 모두
   `Accepted`, `statusCode=0`, `issues: null`(0건)이고, app/DMG staple, Gatekeeper, mounted-content 통합 verifier를
   통과했다.

외부/canonical 실행 또는 후속 강화가 필요:

1. `block/apple-codesign-action` 또는 canonical release process에 통합된 authorized lane이 final DMG byte 자체를
   sign/notarize/staple하도록 외부 계약을 고친다. 현재 v1.1.0 output은 새 hard gate를 구조적으로 통과할 수 없다.
   Updater signing secret으로 만든 archive와 그 안의 stapled app도 canonical verifier를 통과시켜야 한다.
2. Source를 canonical upstream `main`에 반영한 뒤 native X64 runner의 pinned Ubuntu 24.04에서 새 launcher gate와
   pinned-helper AppImage build를 실제 실행한다. 로컬 QEMU 결과로 이를 대체하지 않는다.
3. 최종 `.deb`와 AppImage를 clean install/extract해 desktop과 다섯 sidecar 전체의 ELF/GLIBC/dependency/launch smoke를
   수행한다.
4. apt dependency closure는 여전히 package version까지 고정되지 않았다. 더 강한 reproducibility claim에는 snapshot
   repository 또는 resolved package manifest가 필요하다.

Release gate와 별개인 local cleanup도 완료했다. Archived probe task의 exact managed worktree는 사용자 확인 뒤
SchoolX UI의 native removal로만 제거했고 transcript는 보존했다. 원래 arm64 app, 사용자 REPOS와 localStorage를
검증해 복원했으며 probe용 temporary copy/evidence는 복원 확인 후 제거했다.

## 10. 실행한 관련 regression

macOS host에서 다음 scoped tests가 모두 통과했다.

```text
code_workspace::git_launch_contract_tests                         13 passed
code_workspace::macos_git_xpc::session_lifecycle::tests            5 passed
code_workspace::macos_git_xpc::tests                               9 passed
desktop/scripts/stage-code-git-xpc.test.mjs                        14 passed
desktop/scripts/verify-macos-runtime-compatibility.test.mjs         8 passed
desktop/scripts/build-release-config.test.mjs                       4 passed
release config/runtime-gate focused Node total                     26 passed
root-trusted platform Git exact test                                1 passed
desktop/src/features/code/lib/codeTaskCreation.test.mjs             3 passed
uncertain start reload/recovery exact test                          1 passed
```

위 focused Node 26건은 `14 + 8 + 4`이다. 특히 non-macOS/missing/duplicate platform, 상충하거나 중복된 deployment
command, 공백이 있는 전체 install-name, host Xcode 절대경로, 하위경로 우회와 exact system Swift rpath를 각각
positive/negative fixture로 검사한다.

Ubuntu 결과는 §§6–7에 기록했다.

보강 후 추가 통과:

```text
macOS release/canary + XPC packaging contract                    pass
existing arm64 10.13 fixture runtime negative control            expected fail at plist floor
full-Xcode global selection + clean Rosetta system Git            pass
four Swift compatibility archive lipo assertions                  pass
corrected signed arm64 app signature/runtime verifier            pass
corrected signed x86_64 app runtime verifier                     pass
Rosetta exact embedded-XPC task + direct /usr/bin/git child       pass
arm64/x86_64 app notary logs                                     pass: Accepted/statusCode 0/issues null (0)
arm64/x86_64 stapled app ticket/Gatekeeper/runtime/signature      pass
arm64/x86_64 final DMG notary logs                                pass: Accepted/statusCode 0/issues null (0)
arm64/x86_64 final DMG ticket/Gatekeeper/integrated gate          pass
historical pre-notary x86_64 app/DMG gate                         expected fail: stapler 65, spctl 3
updater archive signing/extracted-app gate                        not run: updater signing secret unavailable
Linux launcher trace/parser/workflow unit tests                 10 passed
latest Linux QEMU release compile                                diagnostic fail: repeated GCC SIGSEGV
desktop Tauri release-profile lib test compile (macOS host)      pass
storage migration release-profile tests                          6 passed
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
| `f5b514808` | 최초 platform readiness handoff와 release 문서 정정 |
| `aa2ec7fb9` | macOS bundle/Mach-O/Swift runtime compatibility fail-closed gate |
| `9017e7172` | Linux launcher gate의 `pipefail` false negative 제거 |
| `67e47becf` | launcher gate를 workflow-provided `runner.arch=X64`에 결합 |

일곱 commit 모두 `Signed-off-by` trailer를 포함한다. 새 origin branch
`codex/schoolx-release-readiness-20260825`는 `67e47becf`까지 게시됐다. 이 handoff와 최신 `RELEASING.md` 정확성
보강은 최종 local closure를 반영한 후속 signed-off documentation commit으로 기록한다.

정상 pre-push hook은 이번 범위와 무관하게 fork `main`과 현재 source 사이에 이미 존재하던 desktop file-size
ratchet 초과 19건에서 실패했다. 이번 범위의 전용 regression과 syntax/contract gate를 별도로 통과시킨 후
`LEFTHOOK=0`으로 새 branch를 게시했다. Hook이나 ratchet 기준은 수정하지 않았다.

Canonical upstream workflow와 release tag는 권한 없이 trigger하지 않았다. Apple 제출은 사용자 승인된 local
artifact 네 건에만 수행했고, credential과 submission 식별자는 repository에 기록하지 않았다. 기존 working-tree
변경도 stage/commit하지 않고 사용자 소유 상태로 보존했다.
