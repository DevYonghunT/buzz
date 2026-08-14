# SchoolX Code Phase 0 인계

작성일: 2026-08-13
상태: Phase 0 native runtime 완료, Phase 1A 시작 전 세션 경계

## 1. 이번 세션에서 완료한 것

- `codex` 실행 파일 탐지와 3초 제한 version probe
- `codex app-server --listen stdio://` child process lifecycle
- 공식 wire 계약에 맞춘 header 없는 JSON-RPC + JSONL framing
- 4 MiB line cap, 64 KiB stderr tail, 512개 notification backlog
- concurrent response dispatcher와 request ID correlation
- `initialize` 요청 후 `initialized` notification handshake
- 알 수 없는 server request의 fail-closed `-32601` 응답
- Unix process group 기반 전체 process tree 종료
- Tauri runtime state 및 다음 command 연결
  - `code_runtime_probe`
  - `code_runtime_start`
  - `code_runtime_stop`
  - `code_runtime_status`
- 일반 종료와 Unix signal 종료 경로에 app-server cleanup 연결

## 2. 주요 파일

```text
desktop/src-tauri/src/code_workspace/
├── mod.rs
├── discovery.rs
├── jsonrpc.rs
└── runtime.rs

desktop/src-tauri/src/commands/code_workspace.rs
desktop/src-tauri/src/app_state.rs
desktop/src-tauri/src/commands/mod.rs
desktop/src-tauri/src/lib.rs
desktop/src-tauri/src/shutdown.rs
```

기준 설계는 [`SCHOOLX_CODE_DESIGN.md`](SCHOOLX_CODE_DESIGN.md)에 있다.

## 3. 검증 결과

```text
cargo test --manifest-path desktop/src-tauri/Cargo.toml code_workspace --lib
→ 9 passed, 0 failed
```

fake app-server로 다음 경로를 검증했다.

- LF/CRLF/blank JSONL
- oversized message 거부
- request/notification/response 구분
- error response
- 정상 initialize + stop
- initialize 실패 후 `Failed` 상태와 process cleanup

실제 설치된 `codex-cli 0.145.0`에 다음 handshake도 성공했다.

```text
initialize
→ userAgent: schoolx-code-smoke/0.145.0 ...
→ codexHome: 현재 Orca Codex home
→ platformFamily: unix
→ platformOs: macos
initialized
```

실제 smoke test는 thread/turn/account 작업을 만들지 않고 handshake 직후 종료했다.

## 4. 유지해야 할 안전 결정

- `buzz-acp` 관리형 봇과 이 runtime을 합치지 않는다.
- Phase 0에는 approval UI가 없으므로 모든 server request를 fail closed한다.
- unknown server request를 자동 승인하지 않는다.
- stdio JSONL만 사용한다. WebSocket transport는 아직 사용하지 않는다.
- React에 raw `ChildStdin`, 절대 경로 권한, 인증 데이터를 넘기지 않는다.
- 새 코드에 `unsafe`, production `unwrap()`, `expect()`를 추가하지 않는다.
- Code transcript를 자동으로 relay/Nostr에 게시하지 않는다.

## 5. 다음 세션 범위: Phase 1A native event/thread bridge

다음 세션도 UI까지 넓히지 말고 아래에서 끊는다.

1. bounded notification backlog를 Tauri event로 내보내는 normalized envelope
2. runtime generation + sequence 부여
3. `thread/start`, `thread/resume`, `thread/list`
4. `turn/start`, `turn/steer`, `turn/interrupt`
5. command/file/permission server request를 pending approval store에 보관
6. `(generation, request id, thread id, turn id)`가 일치하는 approval response
7. fake app-server fixture로 delta, approval, interrupt, reconnect 테스트

Phase 1A 완료 전에는 실제 사용자 prompt를 Codex에 보내는 UI를 만들지 않는다.
승인과 event ordering 계약이 먼저 고정돼야 한다.

## 6. 다음 세션 시작 문구

새 세션에서 아래처럼 요청하면 된다.

> `docs/schoolx-2/SCHOOLX_CODE_DESIGN.md`와
> `docs/schoolx-2/SESSION_HANDOFF_20260813_CODE_PHASE0.md`를 읽고 SchoolX Code
> Phase 1A native event/thread bridge를 구현해줘. 기존 사용자 변경은 보존하고,
> 이 인계 문서의 다음 세션 범위에서 작업을 끊어줘.

## 7. 작업 트리 주의

이번 기능과 무관한 기존 수정 및 untracked 파일이 이미 있다. 특히
`managed_agents/restore.rs`, `runtime.rs`, `runtime/tests.rs`,
`runtime_commands.rs`, `brand/`, `deploy/`, `supabase/`는 사용자 작업이므로
되돌리거나 정리하지 않는다.
