# SchoolX Code Phase 1A 인계

작성일: 2026-08-13
상태: Phase 1A native event/thread bridge 완료, Phase 1B 시작 전 세션 경계

## 1. 이번 단계에서 완료한 것

- bounded notification backlog를 raw JSON-RPC 대신 정규화된
  `schoolx-code-workspace-event` Tauri event로 내보낸다.
- runtime generation과 generation별 단조 증가 sequence를 부여한다.
- listener 재연결 시 generation/sequence 기준 backlog replay와 truncation 여부를
  반환한다.
- 다음 Codex app-server method를 좁은 native DTO로 연결했다.
  - `thread/list`
  - `thread/start`
  - `thread/resume`
  - `turn/start`
  - `turn/steer`
  - `turn/interrupt`
- thread/turn 시작 전 execution root를 native에서 canonicalize한다.
- thread는 `workspace-write` + `on-request`, turn은 network 비활성화와 단일
  writable root를 기본값으로 고정한다.
- command/file/permission server request만 bounded pending approval store에
  보관하며, 나머지 server request는 fail closed한다.
- approval response를 `(runtime generation, request ID, thread ID, turn ID)`에
  묶고, permission grant가 요청 범위를 넓히지 못하게 검사한다.
- turn 완료·interrupt·runtime 종료/교체 시 관련 pending approval을 무효화한다.
- frontend로 내보내는 protocol payload에 기존 secret redaction을 적용한다.

Phase 1A에는 React/TypeScript UI가 없으며 실제 사용자 prompt를 보내는 화면도
아직 없다.

## 2. 주요 파일

```text
desktop/src-tauri/src/code_workspace/
├── mod.rs
├── approvals.rs
├── discovery.rs
├── jsonrpc.rs
├── paths.rs
├── protocol.rs
└── runtime.rs

desktop/src-tauri/src/commands/code_workspace.rs
desktop/src-tauri/src/app_state.rs
desktop/src-tauri/src/commands/mod.rs
desktop/src-tauri/src/lib.rs
desktop/src-tauri/src/shutdown.rs
```

기준 설계는 [`SCHOOLX_CODE_DESIGN.md`](SCHOOLX_CODE_DESIGN.md)에 있다.

## 3. 현재 native command 계약

```text
code_runtime_probe
code_runtime_start
code_runtime_stop
code_runtime_status
code_runtime_events

code_threads_list
code_thread_start
code_thread_resume
code_turn_start
code_turn_steer
code_turn_interrupt
code_approval_respond
```

Phase 1A의 thread/turn command는 아직 caller가 `workspaceRoot`를 함께 보내는
과도기 계약이다. Phase 1B에서 저장된 binding을 권위 있는 execution root로
사용하도록 바꾼다. UI가 아직 없으므로 이 native 계약 변경은 frontend 호환성
문제를 만들지 않는다.

## 4. 검증 결과

2026-08-13 현재 작업 트리에서 다음을 실행했다.

```text
cargo test --manifest-path desktop/src-tauri/Cargo.toml code_workspace --lib
→ 18 passed, 0 failed
```

fake app-server fixture와 단위 테스트로 다음을 검증했다.

- Phase 0의 JSONL framing, size cap, initialize/stop/failure cleanup
- normalized delta의 thread/turn/item ID와 secret redaction
- unsupported notification 무시와 unknown server request fail-closed
- bounded backlog와 replay gap 표시
- thread start/resume/list 및 persisted turn snapshot 변환
- turn start/steer/interrupt
- command approval round-trip과 resolved event ordering
- stale generation approval 거부와 permission subset 제한
- runtime 재시작 후 generation 교체 및 thread resume

현재 app-server RPC error의 `error.message`는 command error 문자열로 전달되므로
일반 payload redaction과 같은 수준의 보장을 아직 하지 않는다. Phase 1B 이후
diagnostics hardening에서 별도로 고정해야 한다.

## 5. 유지해야 할 안전 결정

- `buzz-acp` 관리형 봇과 SchoolX Code runtime을 합치지 않는다.
- React/frontend가 raw child stdin, 임의 execution root 또는 Git command line을
  소유하지 않는다.
- 저장된 binding의 canonical execution root만 Codex `cwd`와 writable root로
  사용한다.
- 같은 managed worktree를 서로 다른 Code thread에 중복 binding하지 않는다.
- worktree root 밖으로 벗어나는 symlink/canonical path는 거부한다.
- worktree 생성 실패나 thread 시작 실패 때 사용자 checkout과 이미 존재하던
  worktree를 삭제하거나 reset하지 않는다.
- dirty worktree 자동 삭제는 Phase 1B에 넣지 않는다. 제거는 Phase 2의 별도
  사용자 action이다.
- 새 코드에 `unsafe`, production `unwrap()`, `expect()`를 추가하지 않는다.
- Code transcript를 자동으로 relay/Nostr에 게시하지 않는다.

## 6. 다음 범위: Phase 1B native worktree/thread binding persistence

Phase 1B도 frontend UI로 넓히지 않고 다음 native 범위에서 끊는다.

1. SchoolX app data 아래에 schema version이 있는 thread-binding index를 둔다.
2. binding은 설계의 다음 필드를 저장한다.
   - `community_id`
   - `project_dtag`
   - `repository_identity`
   - `codex_thread_id`
   - `execution_mode` (`worktree` 또는 `local`)
   - canonical `execution_root`
   - resolved `base_ref`
   - managed worktree에만 존재하는 `worktree_id`
3. index key를 community + project dtag + repository identity로 격리하고, 같은
   Codex thread ID가 다른 scope/root로 재결합되는 것을 거부한다.
4. `~/.schoolx/WORKTREES/<repository-hash>/<worktree-id>` 제품 경계 안에서만
   detached Git worktree를 준비한다. 실제 실행 환경에서는 현재 SchoolX nest
   root를 사용해 dev/release 데이터 경계를 지킨다.
5. Git common-dir에서 native repository identity를 만들고, base ref를 commit으로
   resolve한 뒤 worktree를 생성한다.
6. worktree 준비와 local-checkout 선택을 native command로 제공하되 destructive
   remove/reset/clean은 구현하지 않는다.
7. `thread/start` 성공 직후 binding을 atomic하게 기록한다. 기록 실패 시 생성된
   Codex thread/worktree를 자동 삭제하지 않고 오류를 반환해 복구 가능하게 둔다.
8. `thread/resume`, `thread/list`, `turn/start`는 caller가 보낸 임의 path가 아니라
   저장된 binding을 조회하고 다시 canonicalize한 execution root를 사용한다.
9. 앱 재시작을 모사한 store reload, scope 격리, 중복 worktree 거부, corrupt/unknown
   schema fail-closed, symlink/root containment, Git worktree 준비를 Rust 테스트로
   고정한다.

Phase 1B 완료 전에는 frontend adapter/reducer, Code workspace 화면, 승인 카드,
Changes inspector를 만들지 않는다.

## 7. 작업 트리 주의

SchoolX Code 이외의 기존 수정과 untracked 파일이 함께 있다. 특히
`managed_agents/restore.rs`, `managed_agents/runtime.rs`,
`managed_agents/runtime/tests.rs`, `managed_agents/runtime_commands.rs`,
`crates/buzz-core/src/relay.rs`, `brand/`, `deploy/`, `supabase/`는 사용자 작업으로
간주하고 되돌리거나 정리하지 않는다.

Phase 1B는 새 `code_workspace` native 모듈과 얇은 command/AppState wiring, 이
문서 범위만 수정한다.
