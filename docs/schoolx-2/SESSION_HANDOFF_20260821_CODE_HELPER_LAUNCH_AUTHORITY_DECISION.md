# SchoolX Code native Git descriptor-spawn decision gate

조사 기준일: **2026-08-21**

결정 및 구현 상태 갱신: **2026-08-22**

문서 상태: **사용자 승인 선택 B 구현 및 launch-authority 회귀 완료, release artifact/전역 size gate 잔여**.
[`SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY.md`](SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY.md)의
decision gate 조사에서 현재 repository dependency와 safe Rust API만으로 macOS/Linux 공통 direct spawn을
구현할 수 없음을 확인했고, 사용자는 **signed unprivileged macOS XPC/helper + Linux pinned direct spawn**인
선택 B를 승인했다. 이 문서의 원래 조사 증거는 보존하지만, 아래에 인용한 이전 상태인 “제품 미변경”, “결정
대기”, “세 production `current_exe()` helper가 남음”은 이 2026-08-22 갱신으로 **명시적으로 superseded**됐다.

전체 Git-write/worktree/removal 회귀, Tauri library, frontend test/typecheck/범위 lint와 fresh-build E2E는 통과했다.
`just ci`도 root fmt와 workspace Clippy까지 통과했지만, 이 launcher 작업 전에 누적된 Phase 0~3 대형 파일 19개를
현재 base와 비교하는 desktop file-size ratchet에서 중단됐다. 이번 migration으로 1,000줄을 새로 넘겼던
`git_write/git_command.rs`의 bounded capture를 201줄 sibling module로 분리해 본체를 959줄로 되돌렸고, guard를
완화하지 않았다. 실제 Developer ID signed dual-architecture artifact와
전체 Linux desktop/release image도 CI가 남아 있다. 따라서 launch-authority 구현 회귀는 완료됐지만 최종 release
readiness까지 완료됐다고 주장하지 않는다.

## 1. 결정과 지원 계약

원래 decision gate의 기술 결론은 그대로다.

- macOS 10.15용 OS primitive 자체는 존재하고 실제 arm64/x86_64 child에서 동작하지만, 조사한 safe Rust API에는
  `posix_spawn_file_actions_addfchdir_np` wrapper가 없었다.
- macOS `/dev/fd/<n>`은 directory descriptor cwd의 대안이 아니며 실제 `chdir`가 `ENOTDIR`로 실패한다.
- Linux `/proc/self/fd/<n>`과 `std::process::Command::current_dir` 조합은 고정 release tuple에서 사용할 수 있지만,
  Rust public API가 `posix_spawn` backend나 `fork/exec` fallback 금지를 보장하지 않는다.

이 근거에 따라 2026-08-22 사용자가 선택 B를 승인했고, 지원 계약은 다음과 같이 고정했다.

| 플랫폼/빌드 | SchoolX Code 신규 Git mutation | launch authority |
|---|---|---|
| macOS 12+ Developer ID signed 배포 | 조건 충족 시 지원 | app bundle에 내장된 unprivileged XPC service, app/helper 고정 identifier와 동일한 유효 Team ID 검증 |
| unsigned/ad-hoc/development macOS, macOS 12 미만 | **지원하지 않음** | journal/lock/removal claim/Git write 전 deterministic unsupported |
| 고정 Linux release tuple | runtime probe 통과 시 지원 | 열린 root directory FD를 `/proc/self/fd/<N>` cwd로 소비하는 direct root-trusted Git spawn |
| 그 밖의 platform/runtime | **지원하지 않음** | pathname/self-reexec fallback 없이 mutation 전 거부 |

macOS child executable은 fixed `/usr/bin/git`이다. Linux도 `/usr/bin/git`을 첫 후보로 삼고, 다른 후보는 canonical
executable과 모든 namespace ancestor가 root-owned/non-writable이며 exact identity 재검증을 통과한 경우에만
허용한다. Root 및 root package-manager/OS update는 이 정책의 TCB다.

세 production self-reexec와 production private argument/environment dispatcher는 제거됐다. 각 기존 파일의
`current_exe()`는 crash/fault subprocess용 `#[cfg(test)]` harness에만 남고 source contract test가 이 구분을
고정한다. 전체 gate가 끝나기 전까지는 이 상태를 release 완료로 표시하지 않는다.

## 2. macOS 증거

### 2.1 SDK/deployment availability

현재 macOS SDK의 `spawn.h`는 다음 availability를 선언한다.

- 표준 `posix_spawn_file_actions_addfchdir`: macOS 26.0+
- legacy `posix_spawn_file_actions_addfchdir_np`: macOS 10.15~26.0
- `posix_spawn`, stdio `adddup2`와 `addclose`: macOS 10.5+

Repository release/canary는 `MACOSX_DEPLOYMENT_TARGET=10.15`와
`minimumSystemVersion=10.15`를 유지한다. 따라서 이 범위에서는 표준 26.0 symbol이 아니라 `_np` symbol을
사용해야 한다.

이 내용은 2026-08-21 primitive 조사 당시의 compatibility 증거다. 현재 app/helper binary metadata가 더 낮은
OS floor를 유지하더라도, **SchoolX Code Git launch capability 자체는 macOS 12+와 유효한 signed peer로 더
좁게 제한**된다. 낮은 deployment metadata를 macOS 10.15~11의 mutation 지원 주장으로 해석하지 않는다.

### 2.2 Native descriptor-cwd spike

Repository 밖 임시 C spike에서 다음 순서를 실행했다.

1. directory를 열고 `O_CLOEXEC` FD를 유지한다.
2. 원래 path를 `moved`로 rename하고 원래 이름에 replacement directory를 만든다.
3. `posix_spawn_file_actions_addfchdir_np`와 stdout `adddup2` action을 구성한다.
4. `/bin/pwd`를 직접 `posix_spawn`한다.
5. child cwd, 열린 FD와 moved/replacement의 device/inode를 비교한다.

arm64 native와 x86_64 Rosetta runtime 모두 열린 FD와 moved directory의 device/inode가 일치했고 replacement
inode와 달랐다. 두 binary 모두 `_posix_spawn_file_actions_addfchdir_np`를 참조했다. x86_64 binary의
`LC_BUILD_VERSION` minimum은 10.15였고, arm64 binary는 해당 architecture의 OS floor에 따라 11.0이었다.

이는 필요한 OS primitive와 action ordering이 실제로 동작함을 증명한다. 막힌 부분은 kernel/API 부재가 아니라
repository가 no-`unsafe`로 호출할 수 있는 audited safe Rust surface의 부재다.

### 2.3 `/dev/fd` negative spike

열린 directory FD에 대해 다음 차이가 재현됐다.

```text
chdir("/dev/fd/3") -> ENOTDIR
fchdir(3)           -> success
```

`/dev/fd/3`의 `lstat` 결과도 원본 directory의 filesystem identity와 다른 devfs object였다. 따라서
`Command::current_dir("/dev/fd/<fd>")`는 macOS descriptor-cwd 구현으로 사용할 수 없다. `F_GETPATH`로 현재
pathname을 다시 얻는 방법도 validation과 spawn 사이 pathname replacement race를 되살리므로 완료 조건이
아니다.

## 3. Rust dependency/API 조사

### 3.1 현재 graph

- `rustix 1.1.x`: safe process-wide `fchdir`는 있지만 descriptor cwd spawn file action이 없다.
- `nix 0.31.3`: safe `posix_spawn`, `add_dup2`, `add_open`, `add_close`, process-group attribute는 있지만
  `add_fchdir`가 없다. 현재 lockfile에서는 transitive dependency이고 Tauri crate의 direct dependency도 아니다.
- `libc 0.2.186`: Linux GNU/musl용 raw `posix_spawn_file_actions_addfchdir_np` 선언은 있지만 Apple 선언이 없다.
  원래도 raw FFI 호출은 production `unsafe`를 요구한다.

### 3.2 확인했지만 채택할 수 없는 후보

- `cap-std-ext::CommandExt::cwd_dir`: public call은 safe지만 내부에서 `pre_exec(fchdir)`를 등록해 std를
  post-thread `fork/exec` 경로로 강제한다.
- `command-fds`: FD mapping을 위해 `pre_exec`를 사용하며 cwd action도 제공하지 않는다.
- `libc-spawn 0.0.1`: raw/unsafe binding이고 이 launch authority를 맡길 감사·유지보수 근거가 부족하다.
- `std::os::unix::process::CommandExt::pre_exec`: 직접 `unsafe`이며 handoff가 명시적으로 금지한다.
- parent process의 `fchdir` 후 spawn/restore: cwd가 process-global이므로 multi-threaded Tauri에서 race와
  cross-request 영향 없이 사용할 수 없다.

확인 시점에 macOS와 Linux 양쪽 `addfchdir[_np]` action을 제공하는 유지보수 중인 public safe Rust wrapper는
찾지 못했다.

### 3.3 승인 후 선택한 FFI 경계

선택 B 구현은 `swift-bridge`와 `swift-bridge-build`를 **정확히 `=0.1.59`**로 고정한다. Repository가 직접
작성한 production Rust에는 `unsafe`, `pre_exec` 또는 post-thread `fork` callback을 추가하지 않았다. 다만
`swift-bridge`가 생성하는 ABI glue와 Swift/native spawn 코드는 descriptor file action을 연결하는 FFI
TCB이므로, 일반 safe Rust 의존성과 같은 수준으로 표현하지 않고 exact-pinned audited boundary로 취급한다.
업데이트할 때는 generated ABI, 양 architecture compile/link, signing 및 fault matrix를 다시 검증해야 한다.

## 4. Linux의 승인된 조건부 지원 범위

Linux에서는 root FD를 `FD_CLOEXEC` 상태로 유지한 채 `/proc/self/fd/<n>`을 child cwd path로 주면, spawn file
action이 close-on-exec 처리보다 먼저 실행되므로 열린 inode가 선택된다. Rust 1.95 std 구현도 다음 조건에서
weak lookup한 `posix_spawn_file_actions_addchdir_np`와 `posix_spawnp`를 사용한다.

- absolute executable path
- uid/gid/groups/chroot/pre-exec closure 없음
- glibc 2.24+
- cwd action symbol 존재; glibc에서는 2.29+

현재 Linux canary/release container는 digest-pinned Ubuntu 24.04이므로 이 조건을 만족한다. 선택 B는 이를
Rust 1.95 std 구현, Ubuntu 24.04/glibc 2.39, procfs `/proc/self/fd` semantics로 이루어진 **고정 release
tuple**로 승인했다. 그러나
`Command::spawn`의 public contract는 실제 backend를 관찰하거나 fork fallback을 거부하는 기능을 제공하지
않는다. 따라서 구현은 다음 항목을 지원 계약으로 고정한다.

- Rust toolchain/std 구현과 glibc 2.29+ runtime floor 고정
- `/proc` mount와 `/proc/self/fd` semantics를 startup/admission에서 검증
- unsupported runtime에서 mutation 전 deterministic rejection
- CI source sentinel과 runtime tracing/test로 silent fork fallback 감시
- std/toolchain update마다 descriptor rename, stdio, process-group과 no-fork evidence 재검증

구현은 매 admission에서 `/proc/self/fd`가 procfs인지, FD entry가 magic link인지, 재개방한 대상의 device/inode가
열린 directory와 같은지 확인하고 실제 bounded `git --version` spawn probe를 수행한다. Root FD는
`FD_CLOEXEC`인 3 이상의 duplicate로 유지해 `Command::spawn`이 cwd를 소비할 때까지 살려 두고, Git exec에는
남기지 않는다. Directory rename/replacement, 독립 stdin, bounded output과 own process group 계약도 launcher
test에 포함된다.

이 계약은 현재 release matrix에서는 구현 가능하지만, 안정된 safe wrapper가 제공하는 OS-enforced API
계약보다 취약한 toolchain implementation dependency다. Public std backend 보장이라고 주장하지 않으며,
toolchain/libc 변화나 probe 실패는 지원 밖으로 처리한다. 현재 aarch64 Linux 독립 launcher runtime은 3/3과
isolated Clippy를 통과했지만, **전체 Linux desktop build는 CI 대기**다.

## 5. 결정 기록과 조사 당시 선택지

**결정: 선택 B, 2026-08-22 사용자 승인.** 선택 B의 signed unprivileged macOS XPC/helper와 Linux pinned
direct spawn을 구현했다. 아래 A/C/D는 결정 전에 비교한 역사적 대안이며 현재 선택되지 않았다. “결정 없이
A/B/C 중 하나를 구현하지 않는다”는 원래 gate는 사용자 승인으로 충족됐다.

### 선택 A — audited safe spawn wrapper를 먼저 확보 (미선택)

`libc`에 Apple `_np` binding을 추가하고 `nix::PosixSpawnFileActions`에 safe `add_fchdir`를 upstream한 뒤,
review/release된 version을 direct dependency로 채택한다. 또는 같은 수준으로 유지보수·감사되는 기존 crate가
나오면 채택한다.

- 장점: 세 helper를 직접 root-trusted Git child로 통일하고 handoff 구조를 가장 그대로 만족한다.
- 단점: upstream review/release 일정에 의존한다. repository 안의 임시 unsafe shim이나 forked raw binding으로
  대체해서는 안 된다.

### 선택 B — signed unprivileged macOS XPC/helper 설계 (승인·구현)

macOS에서는 launchd/code-signing이 executable identity를 강제하는 embedded XPC service 또는 동등한 signed
helper를 사용한다. Helper는 전달받은 directory handle을 단일-purpose process에서 `fchdir`한 뒤 closed typed
Git command만 실행한다. Linux는 별도 direct descriptor spawn을 사용한다.

- 장점: app bundle path를 바꿀 수 있는 same-UID actor에게 app self-reexec authority를 주지 않으면서 현재
  macOS 기능을 유지할 수 있다.
- 단점: Tauri packaging, signing, IPC handle transfer와 release/canary build 흐름을 별도 설계·검증해야 한다.
  구현은 root privilege를 추가하지 않았고, unsigned development build에서는 기능을 열지 않고 unsupported로
  둔다.

### 선택 C — macOS mutation을 deterministic unsupported로 전환 (미선택)

Linux만 위의 pinned toolchain/libc 계약 아래 direct migration하고, macOS의 신규 Git write/worktree/removal은
journal claim, standard lock, target reservation이나 Git object write 전에 거부한다. 기존 durable record는
strict recovery/fail-closed 순서를 유지한다.

- 장점: 추가 native helper나 production unsafe 없이 launch boundary를 닫는다.
- 단점: macOS에서 SchoolX Code 핵심 mutation 기능이 비활성화된다. 현재 macOS 중심 회귀와 제품 기대를 크게
  바꾸므로 명시적 product 승인이 필요하다.

### 선택 D — 현재 구현을 임시 유지하고 upstream을 기다림 (미선택)

현재 self-reexec risk를 문서화하고 migration을 보류한다.

- 장점: 당장 packaging과 macOS product 동작을 바꾸지 않는다.
- 단점: launch-authority 취약 경계를 그대로 유지하므로 이 handoff의 완료 선택지가 아니다. 시간 제한과 risk
  acceptance owner가 명시된 경우에만 임시 결정으로 기록해야 한다.

## 6. 2026-08-22 실제 구현 상태

### 6.1 macOS signed XPC launch authority

- App bundle에 고정 identifier `io.github.schoolx520.app.schoolx-code-git`의 unprivileged XPC service를 넣는다.
  Client와 service는 activation 전에 서로의 fixed code identifier, valid signature와 app과 동일한 non-empty
  Team ID를 검증한다. Runtime은 macOS 12+만 admit한다.
- Capability 검사는 새 Git write journal/standard lock/object write, 새 worktree mutation, 새 removal
  proof/manifest/claim 전에 실행된다. Unsigned/ad-hoc/dev build, missing/invalid helper와 macOS 12 미만은 이
  지점에서 unsupported다. 이미 존재하는 durable Git/removal record는 skip하지 않고 기존 strict
  recovery/fail-closed 경로를 따른다.
- Client가 열린 directory와 stdin handle을 전달하고 service가 device/inode/type를 다시 확인한다. Service의
  closed family/command parser와 Rust callback도 request envelope, repository authority와 root-trusted Git을
  다시 검증한다. Generic executable, shell 또는 public argv setter는 없다.
- Helper는 fixed `/usr/bin/git`을 `posix_spawn`하고
  `posix_spawn_file_actions_addfchdir_np`로 이미 열린 directory FD를 cwd로 사용한다. Stdio `dup2`, cleared
  environment, own process group과 suspended-before-resume ordering으로 child identity를 확인한 뒤 실행한다.
- Protocol v3는 한 authenticated XPC connection에 persistent session 하나와 active child 하나만 허용한다.
  `sessionBegin/sessionBegan`부터 `sessionEnd/sessionEnded`까지 random 128-bit session nonce, exact session ID와
  helper incarnation을 유지하고, launch/resume/poll/cancel/finished envelope는 여기에 exact request ID, helper PID와
  PGID를 추가로 결합한다.
- Client가 검증한 `/usr/bin/git` read-only FD의 flock OFD를 service가 duplicate해 session admission부터 clean end까지
  공유한다. Service만 global reservation, manual XPC transaction과 duplicate를 보유하며, client는 exact
  `sessionEnded` 뒤에만 자기 FD를 unlock/close한다. Git write/recovery, worktree read/mutation, removal proof/cleanup은
  각각의 고수준 작업 전체를 이 session과 exact end fence 안에서 수행한다.
- Unlinked `0600` PID slot의 aligned owner CAS는 0/1/2/3/4 상태를 사용한다. Service만 자신이 직접 소유하고 아직
  reap하지 않은 child를 signal하며 client는 numeric signal을 보내지 않는다. `finished`와 `cancelAck`는 reaping 및
  process-group `ESRCH` 증명 뒤 atomic reset한 terminal state에서 반환되고, reset 전/후 cancel race는 pending ACK
  detach 또는 terminal replay로 닫힌다.
- Client/helper 한쪽 death, begin/end reply loss와 helper-exit callback은 pre-armed exact peer exit와 one-shot FD claim을
  사용한다. Local unlock/close, Swift registry 제거, nonzero exact Rust session-ID CAS 순서가 모두 증명된 경우에만
  late cleanup한다. Helper나 child 종료를 증명할 수 없거나 live session clone이 남으면 authority를 poison하고
  **fail-closed로 계속 보유**한다.

### 6.2 Linux direct launcher

- Root-trusted Git과 live procfs descriptor capability를 admission 전에 pin/probe한다. Command executable은
  caller가 바꿀 수 없고, environment clear 뒤 typed caller의 제한된 args/env만 추가한다.
- Root directory FD는 `FD_CLOEXEC` duplicate로 pin하고 `/proc/self/fd/<N>`이 procfs magic link이며 같은
  device/inode directory로 resolve되는지 확인한다. Descriptor는 spawn이 cwd action을 소비할 때까지 유지되고
  Git exec 뒤 ambient FD로 남지 않는다.
- Stdin은 cwd FD와 독립적으로 설치하고 stdout/stderr는 기존 bounded capture를 사용한다. Child는 own process
  group으로 시작해 기존 timeout/descendant cleanup을 유지한다.
- 이 구현은 고정 release tuple의 관찰된 Rust std backend에 의존한다. Public std API가 no-fork backend를
  보장한다고 주장하지 않으며, 실제 runtime probe 또는 tuple이 맞지 않으면 mutation 전에 거부한다.

### 6.3 세 caller와 packaging

- Git write, pinned Git/worktree, safe-remove의 세 production caller와 그 production Git read가 새 platform authority를
  사용한다. Status/discovery/config/ancestry/proof를 포함한 관련 read와 mutation을 한 고수준 session으로 묶고,
  cleanup proof가 끝나기 전에는 session end를 허용하지 않는다.
  Production `current_exe()` self-reexec, private helper arguments/request environment와 pre-Tauri legacy
  dispatcher는 제거됐다. Test-only crash harness는 보존됐다.
- 여섯 public Git command와 top-level `{input}`, binding v4, safe-remove 9-field receipt/journal, crash/CAS/ack/
  startup recovery 및 runtime XOR는 변경하지 않았다.
- Build가 XPC bundle을 app의 고정 `Contents/XPCServices/...` 위치에 stage하고 Tauri packaging에 포함한다.
  Release/canary verifier는 nested executable의 존재/type/실행 bit, strict signature, fixed app/helper identifier와
  동일한 non-empty Team ID를 검사한다. 실제 Developer ID 서명 artifact 검증은 signing secret이 있는 CI에서
  수행해야 하며, local unsigned bundle 구조 검사를 signed artifact 검증으로 과장하지 않는다.
- Pinned Tauri CLI 2.11.2의 실제 hook 계약을 staging test로 고정했다. Debug는
  `TAURI_ENV_DEBUG="true"`, release는 변수가 생략되며 normalized caller의 명시적 `"false"`도 release로
  허용한다. 그 밖의 값은 fail closed한다.

## 7. 남은 residual과 검증 상태

### 7.1 의도적으로 남은 residual

- App와 XPC helper가 동시에 비정상 종료되면 어느 쪽도 살아 있는 Git process group을 정리할 주체가 없을 수
  있다. 현재 프로토콜은 한 프로세스가 살아 있는 crash/timeout/cancel과 PID reuse는 방어하지만, 별도 OS
  guardian 없이 simultaneous app+helper death까지 완전히 닫았다고 주장하지 않는다.
- Git write는 descriptor-bound cwd를 사용해도 `GIT_WORK_TREE`, `GIT_DIR`, `GIT_COMMON_DIR`에 absolute
  repository pathname을 넘긴다. 기존 exact revalidation/CAS가 이 residual을 완화하지만 repository namespace의
  atomic descriptor closure는 이번 launch-authority 완료 범위가 아니다.
- Linux는 public Rust std backend guarantee가 아니라 pinned release tuple + runtime probe 계약이다. 전체 Linux
  desktop build와 release image 검증은 CI에서 완료해야 한다.
- 종료를 증명할 수 없는 helper/child authority는 bounded 성공으로 위장하지 않고 fail-closed retention한다.
  이는 명시적인 availability residual이다.

### 7.2 현재 migration에 대해 통과한 표적 검증

| 검증 | 결과 |
|---|---|
| aarch64 Linux 독립 descriptor launcher runtime | **3/3 통과**, isolated Clippy 통과 |
| Production/test-only launch source contract | **9/9 통과** |
| macOS XPC Rust session/CAS contract | **10/10 통과**, fault 재감사 P0/P1/P2 0 |
| Swift compile/fault spikes | arm64 macOS 11 + x86_64 macOS 10.15 typecheck 통과; native/Rosetta CAS·OFD·session race spike 통과 |
| Git write 전체 | **67 passed, 2 ignored, 0 failed** |
| Pinned Git/worktree 전체 | **25 passed, 1 ignored, 0 failed** |
| Safe-remove physical 전체 | **24 passed, 2 ignored, 0 failed** |
| Native admission/contract | **7/7**, **8 passed, 1 ignored** |
| Tauri library 전체 | **2454 passed, 21 ignored, 0 failed** |
| Tauri Rust formatting/check/lint | `cargo fmt`, `cargo check --lib`, `cargo clippy --lib --tests -- -D warnings` 통과 |
| 독립 desktop/web build | `just desktop-build`, `just desktop-tauri-check`, `just web-build` 통과 |
| Mobile test | **1022 passed, 1 skipped, 0 failed** |
| Desktop frontend | typecheck 통과, **4037/4037 test 통과**, 범위 Biome/px-text 통과 |
| Fresh-build SchoolX Code Playwright smoke | **26/26 통과** |
| XPC staging unit | **7/7 통과** |
| Packaging/release contracts | XPC packaging, signed canary, release ref, shell syntax 통과 |
| Repository final whitespace | `git diff --check` 통과 |

이 표는 2026-08-22 migration 표적 결과다. 2026-08-21 이전 handoff의 전체 suite 기준선은 새 launcher에 대한
회귀 결과가 아니므로 재사용하지 않는다.

### 7.3 release readiness 전에 남은 gate

- aarch64/x86_64 signed release/canary artifact의 nested XPC signature verifier
- 전체 Linux desktop/release container build
- 최종 `just ci`: root fmt/workspace Clippy는 통과했으나 desktop file-size ratchet이 base 대비 누적 Phase 0~3 대형
  파일 19개에서 실패한다. Launch migration이 새로 만든 `git_command.rs` 초과는 `capture.rs`로 분리해 제거했고,
  dedicated launch-authority Rust/Swift 신규 파일도 모두 1,000줄 미만이다. Guard를 완화하거나 limit/allowlist를
  올리지 않았으며, 남은 누적 대형 파일 분할은 별도 구조화 작업으로 남긴다.
- 독립 `just test-unit`은 7개 구성요소가 통과했고 `buzz-voice`만 로컬 native static `onnxruntime` 부재로 compile
  단계에서 중단됐다. 동일 하위 명령에서도 재현되는 환경 blocker이므로 native library가 갖춰진 CI에서 완료해야 한다.

이 gate들이 끝나기 전에는 선택 B의 구현과 이번 범위 회귀 완료를 기록할 수는 있어도, signed/Linux artifact나
repository 전체 release readiness 완료를 주장하지 않는다.
