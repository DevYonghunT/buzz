# SchoolX Code native Git helper launch authority 착수 인계

기준일: **2026-08-21**

문서 상태: **다음 구현 세션용 task brief**. 이 문서를 작성한 세션에서는 제품 코드를 변경하지 않았다.
Phase 3 Git write의 crash/CAS/owned-lock/startup P0는
[`SESSION_HANDOFF_20260821_CODE_PHASE3_GIT_WRITE_IMPLEMENTATION.md`](SESSION_HANDOFF_20260821_CODE_PHASE3_GIT_WRITE_IMPLEMENTATION.md)의
app-process launch 신뢰 경계 아래 완료됐다. 다음 세션은 그 신뢰 가정을 제거하거나, 안전한 구현 primitive가
없음을 확인해 명시적으로 fail closed하는 데만 집중한다.

## 1. 다음 세션의 목표

SchoolX Code의 세 production Git helper가 현재 앱을 pathname으로 다시 실행하는 authority를 제거한다.

```text
trusted running parent
  -> current_exe() pathname resolve
  -> Command::new(pathname)
  -> private argument/environment + inherited directory FD
  -> re-executed app dispatch
  -> typed Git exec
```

현재 parent는 root/repository/Git evidence를 엄격하게 검증하지만 재실행할 app executable이나 그 ancestor를
pin/trust-verify하지 않는다. 앱 경로를 바꿀 수 있는 same-UID actor는 typed Git validation보다 먼저 다른 child
bytes를 실행시키고 pinned directory FD와 private request를 받을 수 있다. Path의 metadata/digest를 검사한 뒤
같은 path로 spawn하는 방식도 검사와 사용 사이 교체를 막지 못한다.

완료 목표는 다음과 같다.

1. 세 production Code Git helper 모두에서 `current_exe()` pathname self-reexec authority를 제거한다.
2. 이미 열린 exact directory FD를 cwd로 사용하는 OS-enforced descriptor-bound spawn을 사용한다.
3. Child executable은 closed typed enum이 선택한 root-trusted Git만 허용한다.
4. macOS/Linux에서 같은 계약을 증명하고, 안전한 primitive가 없는 platform/build는 journal claim이나
   filesystem/Git mutation 전에 deterministic unsupported로 종료한다.
5. 기존 Phase 2 removal/pinned Git 및 Phase 3 Git write public contract와 crash/recovery 보장을 유지한다.

Frontend, public Tauri API 또는 새 Git 기능을 추가하는 세션이 아니다.

## 2. 반드시 먼저 읽을 문서

다음 순서로 읽는다.

1. 이 문서
2. [`SESSION_HANDOFF_20260821_CODE_PHASE3_GIT_WRITE_IMPLEMENTATION.md`](SESSION_HANDOFF_20260821_CODE_PHASE3_GIT_WRITE_IMPLEMENTATION.md)
3. [`SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md`](SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md)
4. [`SCHOOLX_CODE_DESIGN.md`](SCHOOLX_CODE_DESIGN.md)
5. [`SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md`](SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md)
6. [`SESSION_HANDOFF_20260819_CODE_PHASE2_PUBLIC_REMOVAL.md`](SESSION_HANDOFF_20260819_CODE_PHASE2_PUBLIC_REMOVAL.md)
7. [`SECURITY_CONTRACT.md`](SECURITY_CONTRACT.md)
8. Repository root의 [`AGENTS.md`](../../AGENTS.md)

첫 명령은 반드시 다음과 같이 실행한다.

```bash
. ./bin/activate-hermit && git status --short
```

현재 checkout은 Phase 0~3 구현과 사용자 변경이 함께 있는 큰 dirty worktree다. 기존 tracked/untracked 파일을
보존하고 이 checkout 자체에 `git add`, `git commit`, `git reset`, `git checkout` 또는 `git clean`을 실행하지
않는다. 제품 테스트가 만든 격리된 임시 Git repository 안의 mutation만 예외다.

## 3. 현재 production launch 경계

### 3.1 세 helper를 하나의 범위로 다루는 이유

Production `current_exe()` self-reexec는 세 곳에 있다.

| 경로 | 현재 역할 | private argument | request environment |
|---|---|---|---|
| `desktop/src-tauri/src/code_workspace/git_write/git_command.rs` | Phase 3 status/stage/unstage/commit용 typed Git | `--schoolx-code-git-write-helper` | `SCHOOLX_CODE_GIT_WRITE_REQUEST` |
| `desktop/src-tauri/src/code_workspace/worktrees.rs` | pinned Git read와 worktree operation | `--schoolx-code-pinned-git-helper-v1` | `SCHOOLX_CODE_PINNED_GIT_REQUEST_V1` |
| `desktop/src-tauri/src/code_workspace/bindings/removal/physical/unix.rs` | safe-remove proof/recovery용 typed Git | `--schoolx-code-removal-git-helper-v1` | `SCHOOLX_CODE_REMOVAL_GIT_REQUEST_V1` |

`desktop/src-tauri/src/main.rs`는 Tauri 시작 전에
`desktop/src-tauri/src/lib.rs::run_code_pinned_git_helper_if_requested()`를 호출하고, removal → write → pinned Git
순서로 private argument를 dispatch한다. 세 helper 모두 target/root directory handle을 stdin으로 넘기고 child가
`fchdir`한 뒤 대부분 Git으로 `exec`한다. Removal `BlobTypes` 요청만 pipe가 필요해 helper가 Git을
spawn/write/wait한 뒤 종료한다.

Git write helper만 바꾸고 나머지 두 helper를 남기면 app-wide process-launch authority는 닫히지 않는다.
반대로 공통화하면서 Phase 2 helper의 typed request나 removal durability를 약화해서도 안 된다.

### 3.2 이미 닫힌 경계

다음은 다시 설계할 대상이 아니라 보존할 기반이다.

- Git write는 macOS `/usr/bin/git` system candidate와 Linux root-trusted candidate만 허용한다. Pinned Git과
  removal helper는 아직 PATH-resolved canonical regular file 검증만 사용하므로 migration 때 같은 root-trusted
  policy로 올려야 한다.
- Canonical Git executable과 ancestor의 root ownership, non-writable namespace, set-id 부재와 pinned
  device/inode/owner/mode/link-count/size/digest를 exec 직전 검증한다.
- Git write request는 closed `GitWriteCommand`이며 public Tauri caller가 literal path/ref/OID/argv/identity를
  공급하지 않는다. Native-derived typed command 내부 evidence와 혼동하지 않는다.
- Helper envelope는 versioned, bounded, `deny_unknown_fields`이고 directory FD identity와 exact repository
  authority를 child에서 재검증한다.
- 각 최종 Git command에는 environment clear/allowlist, hook/filter/signing/network 차단, literal pathspec,
  timeout, process-group cleanup과 bounded stdout/stderr가 구현돼 있다. 단 app self-reexec hop은 Git write만
  `env_clear()`하고 pinned/removal은 ambient parent environment를 상속하므로 이번 launch-authority 경계에
  포함한다.
- Git journal, owned artifacts/standard locks, startup recovery와 acknowledge tombstone은 이미 전체 회귀를
  통과했다.

Git write는 helper가 `fchdir`한 뒤에도 `GIT_WORK_TREE`, `GIT_DIR`, `GIT_COMMON_DIR`에 absolute pathname을
전달한다. 새 descriptor-cwd launcher만으로 이 repository namespace까지 atomic하게 닫힌다고 주장하지 않는다.
이번 완료 정의는 app executable launch authority 제거이며, Git/repository pathname에는 기존 exact
revalidation/CAS와 문서화된 residual이 계속 적용된다. Relative/descriptor 기반 Git authority로 더 바꾸려면
별도 설계와 fault matrix가 필요하다.

### 3.3 이번 범위가 아닌 `current_exe()` 사용

다음은 별도 lifecycle/packaging 의미를 가지므로 이 세션에 섞지 않는다.

- managed agent discovery/runtime/readiness
- bundled `buzz-acp`/CLI 위치 계산
- 일반 Tauri startup 또는 reset/migration 코드
- Git/removal crash matrix가 libtest child를 다시 실행하는 `#[cfg(test)]` harness

Test-only child entry는 production app trust 검사에 묶지 않는다. 다만 production launcher와 test launcher가
같은 함수에 섞여 완료 sentinel을 우회하지 않도록 명확히 분리한다.

## 4. 위협 모델과 완료 정의

### 4.1 신뢰 경계

- 현재 실행 중인 parent process의 bytes와 OS kernel은 신뢰한다.
- Root, root 권한의 OS/package update와 macOS xcode-select tool resolution은 TCB다.
- User-owned package manager/custom Git과 app install path를 바꿀 수 있지만 running parent의 memory/process를
  제어할 권한은 없는 same-UID actor를 신뢰하지 않는다.
- Workspace/repository content, Git config/hook/filter, webview payload와 agent output은 권한이 아니다.

Homebrew Git이나 user-writable AppImage/bundle 지원을 위해 이 경계를 완화해야 한다면 구현 중 암묵적으로
허용하지 말고 별도 product/security decision으로 올린다.

### 4.2 다음은 완료로 인정하지 않는다

- `current_exe()` path를 stat/hash/canonicalize한 뒤 같은 pathname으로 `Command::new` 호출
- Parent가 보낸 nonce/digest를 child가 그대로 echo하는 self-attestation
- Symlink만 거부하고 writable ancestor 또는 stable-path replacement를 허용
- App 시작 직후 helper를 미리 pathname spawn해 race window만 줄이는 방식
- macOS/Linux 중 한 platform에서만 성립하는 동작을 silent pathname fallback으로 대체
- Git write 한 helper만 교체한 뒤 app-wide helper authority 완료로 표시
- Root-trusted app helper를 만들었지만 child가 user-writable Git executable을 실행하도록 허용

### 4.3 완료 조건

다음이 모두 증명돼야 한다.

- 세 production helper 경로에 `Command::new(current_exe())`가 0개다.
- Production private helper argument/environment dispatch가 제거되거나 non-authoritative test-only code로
  격리된다.
- Root directory는 pathname 재해석 없이 이미 열린 descriptor의 spawn cwd action으로 사용된다.
- stdin payload, stdout/stderr capture, process group과 timeout이 기존과 동등하다.
- Child executable은 공통 root-trusted Git policy와 exact identity 재검증을 사용한다.
- 전달한 target/root cwd pathname을 validation 직후 rename/replace해도 spawn cwd action은 opened
  device/inode를 선택한다. 이는 child cwd 증명이지 Git이 별도로 받는 absolute `GIT_WORK_TREE`/`GIT_DIR`/
  `GIT_COMMON_DIR`의 atomic closure 주장이 아니다. App pathname self-reexec는 없고 Git executable은
  root-trusted namespace와 exact identity로 검증한다. Repository pathname은 기존 verification/CAS 계약을
  유지한다.
- 새 mutation admission의 unsupported capability는 journal claim, standard lock, worktree removal claim과 Git
  object write 전에 zero-mutation으로 거부된다. 이미 존재하는 durable Git/removal record는 기존 strict
  load/cross-preflight/recovery 또는 fail-closed 계약을 그대로 따른다.
- 세 helper migration과 전체 Phase 2/3 회귀가 모두 끝나기 전에는 app-wide 완료로 표시하지 않는다.

## 5. 구현 전 decision gate

큰 refactor 전에 macOS deployment target 10.15의 arm64/x86_64 build와 Linux release/package target에서
**safe descriptor-bound direct spawn** 가능성을 작은 격리 테스트로 먼저 증명한다.

필요한 primitive는 이미 열린 directory FD를 child cwd로 설정하는 spawn file action, stdin/stdout/stderr
dup, process-group 설정과 root-trusted executable exec다.
[POSIX.1-2024 spawn contract](https://pubs.opengroup.org/onlinepubs/9799919799/functions/posix_spawn.html)는
`posix_spawn_file_actions_addfchdir()`와 file-action 순서를 정의하지만 target OS의 실제 symbol/deployment
availability를 별도로 확인해야 한다. 현재
[`std::process::Command::current_dir`](https://doc.rust-lang.org/std/process/struct.Command.html#method.current_dir)는
path만 받고,
[`CommandExt::pre_exec`](https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html#tymethod.pre_exec)는
`unsafe`다. 현재 dependency graph의
[`nix` 0.31 spawn surface](https://docs.rs/nix/0.31.3/nix/spawn/index.html)는 safe `addfchdir` action을 노출하지
않고, 직접 사용하는
[`rustix` 1.1 process surface](https://docs.rs/rustix/1.1.4/rustix/process/index.html)는 process-wide
`fchdir`만 제공한다. 따라서 repo production code에서 no-`unsafe`로 사용할 수 있는 spawn wrapper가 있는지
먼저 확인해야 한다. `nix`는 현재 lockfile의 transitive dependency일 뿐 `desktop/src-tauri/Cargo.toml`의 direct
dependency가 아니므로, 사용하려면 dependency/maintenance 검토부터 명시적으로 거친다.

특히 macOS 10.15 deployment target에서는 POSIX.1-2024의 표준 symbol을 가정하지 않는다. 해당 deployment에서
사용 가능한 legacy `_np` extension 또는 safe wrapper가 있더라도 arm64/x86_64 compile, link와 runtime을 모두
검증한다. Linux도 libc/package target별 standard/extension symbol availability를 같은 방식으로 검증한다.

결정 순서는 다음과 같다.

1. 기존 또는 추가 가능한 audited safe Rust dependency/API가 descriptor cwd spawn을 제공하는지 확인한다.
2. Safe API가 있으면 실제 macOS/Linux child에서 directory rename/replacement 뒤에도 opened inode를 cwd로
   사용하고, stdin과 capture/process-group 계약이 유지됨을 증명한다.
3. Ambient inheritable FD나 global `FD_CLOEXEC` 변경이 필요하면 multi-threaded Tauri의 concurrent spawn에
   descriptor leak/race가 없는지 먼저 증명한다. 증명할 수 없으면 사용하지 않는다.
4. Safe direct spawn이 불가능하면 OS가 launch 시점에 executable identity를 강제하는 별도 signed/XPC 또는
   platform helper 설계를 제안하고, packaging/entitlement/privilege 변경 전 사용자 결정을 받는다.
5. `unsafe`, `pre_exec`, post-thread `fork` 또는 check-then-path-exec만 남는다면 구현을 강행하지 않는다.
   해당 platform은 mutation 전에 unsupported로 두고 blocker와 필요한 외부 결정을 문서화한다.

`/proc/self/exe`, `/dev/fd/*` 또는 유사 경로는 이름만으로 descriptor execution을 보장한다고 가정하지 않는다.
각 release target의 실제 spawn 순서, close-on-exec와 filesystem semantics를 테스트로 증명한 경우에만 사용한다.

## 6. 권장 구현 순서

1. 현재 세 launcher와 Git executable resolution을 호출 그래프로 고정하는 source/contract test를 추가한다.
2. macOS/Linux descriptor-cwd direct-spawn capability spike를 독립 test module에서 수행한다.
3. 결과가 안전하면 `code_workspace` 내부에 공통 private descriptor-spawn abstraction을 만든다.
4. Git write runner를 먼저 옮기고 기존 helper/real-Git 및 crash matrix를 통과시킨다.
5. `worktrees.rs` pinned Git helper를 옮겨 read/worktree mutation tests를 통과시킨다.
6. safe-remove physical helper를 옮겨 durable-boundary crash/recovery tests를 통과시킨다.
7. 세 migration이 끝난 뒤에만 pre-Tauri production dispatcher, private arguments와 request env를 제거한다.
8. Repo-wide `current_exe()` inventory에서 Code production helper가 0개이고 test-only/범위 밖 사용만 남았는지
   sentinel로 검증한다.
9. Targeted tests, 전체 Tauri/frontend/fresh-build E2E와 마지막 `just ci`를 순서대로 실행한다.

공통 모듈을 추가한다면 새 파일을 1,000줄 미만으로 유지한다. 기존 `git_command.rs`, `worktrees.rs`와 removal
`unix.rs`를 더 키우지 말고 platform spawn, trust policy와 tests를 작은 sibling module로 분리한다.

## 7. 필수 fault/security tests

### 7.1 공통 launcher

- App executable stable path replacement/symlink/writable-ancestor가 foreign child 실행으로 이어지지 않음
- Directory path rename/replacement 후 child cwd가 opened device/inode와 일치
- Closed, wrong-type 또는 reused descriptor가 mutation 전에 거부됨
- Root-trusted Git executable/ancestor replacement와 permission/ACL drift 거부
- Child에 auth/Nostr/global Git config/environment가 전달되지 않음
- stdin payload가 root FD와 혼동되지 않고 exact command에만 전달됨
- stdout/stderr cap, timeout과 entire process-group cleanup 유지
- Unsupported platform/capability의 journal/filesystem/Git zero mutation
- 세 production `current_exe()` helper launch 부재 source sentinel

Barrier 기반 app-executable replacement test는 validation 완료와 spawn/exec 사이에 교체를 강제해야 한다.
단순히 spawn 전이나 종료 후 path를 바꾸는 테스트로 TOCTOU closure를 주장하지 않는다. Foreign app-child
fixture는 marker를 남기게 하고 그 marker가 생성되지 않았음을 확인한다.

### 7.2 Git write 보존 회귀

다음 전체 묶음을 유지한다.

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::git_write --lib -- --nocapture
```

현재 기준은 **66 passed, 2 ignored**다. 특히 다음을 삭제하거나 약화하지 않는다.

- Stage 15, commit 17 durable-boundary subprocess crash matrix
- Unstage 4-boundary parity crash smoke
- `AcknowledgementPersisted` response-loss와 durable tombstone
- Published 이후 external index/HEAD drift에도 same receipt 수렴
- Foreign/replaced lock과 planned artifact 보존
- Before/expected-after/third-live-state recovery와 sticky uncertain
- Completed receipt의 restartable cleanup
- Startup malformed/oversized zero mutation과 safe-remove/Git cross-preflight 순서

### 7.3 Phase 2 helper 회귀

공통 launcher를 적용하면 다음을 함께 실행한다.

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::worktrees::tests --lib -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::bindings::removal::physical::tests --lib -- --nocapture
```

Pinned target rename/swap, strict helper envelope, removal durable-boundary crash/startup ordering과 sibling/source
preservation을 유지한다. Test harness의 ignored child entry를 직접 실행하지 말고 parent test가 exact env와
fixture를 설치해 호출하게 한다.

## 8. 보존해야 할 계약

- 여섯 Phase 3 Git command 이름과 top-level `{input}` envelope를 변경하지 않는다.
- Phase 2 binding v4, fork wire, model selector와 safe-remove public input/9-field receipt/journal을 변경하지 않는다.
- Caller path/ref/OID/argv/identity/operationId authority를 추가하지 않는다.
- Generic shell/public argv runner를 새 launcher로 노출하지 않는다.
- Git journal phase/evidence, owned-lock provenance, cleanup/ack/reconcile semantics를 변경하지 않는다.
- Absolute repository namespace에 남은 기존 revalidation/CAS residual을 helper-launch 완료에 포함된 것처럼
  표현하지 않는다.
- Runtime/approval/PTY/fork/archive/remove 대 Git write admission XOR를 유지한다.
- Windows 또는 capability-missing build에 pathname fallback을 추가하지 않는다.
- Production `unsafe`를 추가하지 않는다. 새 public API에는 doc comment를 단다.
- Frontend schema/state/UI/E2E 동작은 launcher 내부 변경 때문에 수정하지 않는다.
- Local/archive write, stage-all/hunk, branch/push/PR/Talk 공유와 hook/signing을 열지 않는다.

## 9. 현재 기준선

직전 완료 세션의 검증 결과는 다음과 같다.

- `cargo fmt`, `cargo check --lib`, `cargo clippy --lib --tests -- -D warnings`
- Git write: **66 passed, 2 ignored, 0 failed**
- Native admission gate: **7 passed, 0 failed**
- Native contract: **8 passed, 1 ignored, 0 failed**
- Tauri 전체: **2429 passed, 21 ignored, 0 failed**
- Frontend 전체: **4037 passed, 0 failed**
- Fresh-build `schoolx-code.spec.ts --project=smoke`: **26 passed, 0 failed**
- `git diff --check`

Ignored Tauri tests에는 환경/수동/feature-flag sentinel과 private subprocess entrypoint가 포함된다.

## 10. 검증 명령

Crash matrix가 현재 libtest binary를 child로 다시 실행하므로 동일 target에 다른 Cargo build/test를 병렬로
실행하지 않는다. 다른 build가 binary를 교체하면 잘못된 child entry를 실행할 수 있다.

```bash
. ./bin/activate-hermit && git status --short

cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check
cargo check --manifest-path desktop/src-tauri/Cargo.toml --lib
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml \
  --lib --tests -- -D warnings

cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::git_write::git_command::tests --lib -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::git_write --lib -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::worktrees::tests --lib -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::bindings::removal::physical::tests --lib -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  commands::code_git_handoff::gate_tests --lib -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::contract_tests --lib -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib

pnpm --dir desktop typecheck
pnpm --dir desktop check:px-text
pnpm --dir desktop test
pnpm --dir desktop exec biome check \
  src/features/code src/testing/e2eBridge.ts tests/e2e/schoolx-code.spec.ts

pnpm --dir desktop build:e2e
pnpm --dir desktop exec playwright test \
  tests/e2e/schoolx-code.spec.ts --project=smoke

just ci
git diff --check
```

Git/hook 명령을 실행하는 shell에서는 repo Hermit environment를 유지한다. Fresh E2E 전에는 `AGENTS.md`의 stale
preview 지침대로 port 4173 listener를 확인하고, stale Playwright preview임이 확인된 경우에만 종료한다.

## 11. 다음 세션 복사용 시작 요청

```text
SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY.md를 먼저 읽고,
SESSION_HANDOFF_20260821_CODE_PHASE3_GIT_WRITE_IMPLEMENTATION.md,
SCHOOLX_CODE_DESIGN.md, SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md와
SESSION_HANDOFF_20260819_CODE_PHASE2_PUBLIC_REMOVAL.md를 대조해줘.

첫 명령은 `. ./bin/activate-hermit && git status --short`로 실행해줘. 현재 shared dirty worktree의
tracked/untracked 사용자 변경을 보존하고 이 checkout 자체를 stage/commit/reset/checkout/clean하지 마.

SchoolX Code production의 세 current_exe Git helper(write, pinned Git, safe-remove)를 공통 scope로 잡아줘.
먼저 macOS/Linux에서 no-unsafe safe descriptor-bound direct spawn을 제공하는 API/dependency와 release-target
동작을 작은 fault test로 증명해줘. Root directory는 열린 FD의 spawn cwd action으로만 사용하고, child는
closed typed enum이 선택한 root-trusted Git만 실행해야 해. Stat/hash 뒤 pathname spawn, self-attestation,
ambient inheritable-FD race 또는 unsupported-platform pathname fallback으로 완료를 주장하지 마.

Safe primitive가 증명되면 Git write, pinned Git, safe-remove 순으로 migration하고 세 production self-reexec와
pre-Tauri private dispatch를 제거해줘. 증명할 수 없거나 packaging/entitlement/privileged helper가 필요하면
제품 변경을 임의로 선택하지 말고 mutation 전 deterministic unsupported 상태와 선택지를 보고해줘.

여섯 Git public command/{input}, binding v4, safe-remove 9-field receipt/journal, crash/CAS/ack/startup recovery,
runtime gate, no-unsafe와 신규 파일 1,000줄 제한을 보존해줘. Local/archive write, stage-all/hunk,
branch/push/PR/Talk 공유와 hook/signing은 열지 마. Targeted fault tests부터 전체 Tauri/frontend/fresh-build E2E와
just ci까지 통과할 때까지 이어서 진행해줘.
```

## 12. 2026-08-22 결과 부록

사용자는 decision gate의 **선택 B**를 승인했다. 상세 결정과 현재 검증 표는
[`SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY_DECISION.md`](SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY_DECISION.md)에
기록했다. 이 문서 머리말의 “제품 미변경/다음 구현 세션” 상태와 3장의 production self-reexec 현황은 아래
결과로 superseded됐다.

- 세 production Code Git `current_exe()` self-reexec와 private argument/environment dispatcher를 제거했다.
  기존 `current_exe()` crash entry는 `#[cfg(test)]` harness로만 보존하고 source contract로 감시한다.
- macOS Code Git mutation은 macOS 12+ Developer ID signed app에서만 지원한다. Embedded unprivileged XPC
  service와 app은 fixed identifier/valid signature/same Team ID를 서로 확인하고, helper는 열린 directory를
  `posix_spawn_file_actions_addfchdir_np`로 cwd에 묶어 fixed `/usr/bin/git`만 실행한다. Unsigned/dev/macOS 12
  미만은 신규 mutation 전에 unsupported다.
- Rust/Swift 경계는 exact-pinned `swift-bridge = 0.1.59` FFI TCB다. Protocol v3 persistent session은 shared
  `/usr/bin/git` flock OFD, exact session/request nonce, helper incarnation, PID owner CAS와 XPC transaction을 묶는다.
  Client는 numeric signal을 보내지 않으며, one-side death/reply-loss/cancel race도 exact cleanup proof 없이는
  authority를 해제하지 않는다.
- Linux는 pinned release tuple에서 runtime-probed `/proc/self/fd/<N>` direct spawn을 사용한다. 이는 Rust
  public std backend guarantee가 아니다. aarch64 독립 launcher runtime 3/3과 isolated Clippy는 통과했지만
  전체 Linux desktop build는 CI 대기다.
- XPC staging과 nested signature/identifier/same-Team verifier를 release/canary packaging에 연결했다. Local
  unsigned bundle 구조 검사와 exact `TAURI_ENV_DEBUG="false"` release staging 계약은 통과했지만 실제 Developer ID
  signed artifact 검증은 CI가 맡는다.

전체 Git-write/worktree/removal target, Tauri library (**2454 passed**), frontend (**4037 passed**)와 fresh-build
SchoolX Code E2E (**26 passed**)까지 통과했다. Source contract는 9/9, XPC Rust session은 10/10이며 Swift
arm64/x86_64 typecheck와 native/Rosetta CAS·OFD·race spike도 통과했다. `just ci`는 root fmt/workspace Clippy 뒤
현재 base가 Phase 0~3 누적 변경을 모두 신규/증가로 보는 desktop file-size ratchet 19개에서 중단됐다. 이번
migration으로 새로 1,000줄을 넘겼던 `git_write/git_command.rs`의 bounded capture를 201줄 sibling으로 분리해
본체를 959줄로 되돌렸고 guard를 완화하지 않았다. Dedicated launch-authority 파일도 모두 1,000줄 미만이다.
독립 desktop/web build와 mobile test 1,022개는 통과했고, `just test-unit`은 7개 구성요소 통과 뒤 로컬 native
static `onnxruntime` 부재로 `buzz-voice` compile만 막혔다. 실제 signed dual-architecture artifact와 Linux desktop
build는 CI가 남았다. Simultaneous app+helper death와 Git에 전달되는 absolute repository pathname은 이번 범위의
명시적 residual이다.
