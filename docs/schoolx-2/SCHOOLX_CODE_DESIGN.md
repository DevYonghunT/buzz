# SchoolX Code — Codex형 로컬 개발 환경 설계

상태: 구현 기준안 v0.1
작성일: 2026-08-13
대상: SchoolX 데스크톱(Tauri 2 + React 19)

구현 현황: Phase 0 runtime, Phase 1A native event/thread bridge, Phase 1B native
worktree/thread binding persistence, Phase 1C native contract/fixture freeze에 이어
Phase 1D React/TypeScript typed adapter와 pure reducer/query state까지 구현했다. UI는
아직 시작하지 않았다. 새 세션은
[`SESSION_HANDOFF_20260814_CODE_PHASE1D.md`](SESSION_HANDOFF_20260814_CODE_PHASE1D.md)를
먼저 읽는다. 이전 구현 경계는
[`SESSION_HANDOFF_20260814_CODE_PHASE1C.md`](SESSION_HANDOFF_20260814_CODE_PHASE1C.md),
[`SESSION_HANDOFF_20260814_CODE_PHASE1B.md`](SESSION_HANDOFF_20260814_CODE_PHASE1B.md),
[`SESSION_HANDOFF_20260813_CODE_PHASE1A.md`](SESSION_HANDOFF_20260813_CODE_PHASE1A.md),
[`SESSION_HANDOFF_20260813_CODE_PHASE0.md`](SESSION_HANDOFF_20260813_CODE_PHASE0.md)에
남아 있다.

## 1. 결론

SchoolX 안에 Codex와 유사한 개발 환경을 구현할 수 있다. 현재 저장소에는 이미
프로젝트/브랜치/파일 탐색, Git diff, PR 검토, 에이전트 실행 상태와 도구 호출
타임라인이 있어, 새 IDE를 처음부터 만드는 것보다 **로컬 Codex 실행 계층과
작업 중심 화면을 기존 기능 위에 얹는 방식**이 맞다.

첫 구현은 아래 구조로 고정한다.

- 제품군 이름은 `SchoolX`다.
- 협업·메신저 화면의 이름은 `SchoolX Talk`다.
- 로컬 개발 환경의 이름은 `SchoolX Code`다.
- 사용자와 대화하며 로컬 코드를 다루는 실행 엔진은 `codex app-server`다.
- 채널 멘션에 반응하는 상시 봇은 기존 `buzz-acp`를 계속 사용한다.
- React가 프로세스나 파일 권한을 직접 소유하지 않는다. Tauri/Rust가 Codex
  프로세스, JSON-RPC, 승인, 작업 디렉터리를 통제한다.
- 초기 권한은 `workspace-write` + `on-request`다. 기존 관리형 봇의
  `bypassPermissions`를 로컬 Code 화면에 재사용하지 않는다.
- 에이전트 작업은 기본적으로 전용 Git worktree에서 실행한다. 사용자의 현재
  체크아웃에서 실행하는 것은 명시적으로 선택한 경우뿐이다.

이 설계는 OpenAI의 `app-server`가 rich client를 위해 제공하는 스레드, 턴,
승인, 스트리밍 이벤트, 인증 계약을 사용한다. 현재 개발 노트북에서 확인한
설치본은 `codex-cli 0.145.0`이며 `codex app-server`, `generate-ts`,
`generate-json-schema`를 제공한다.

## 2. 제품 구조와 이름

### 2.1 사용자에게 보이는 구조

```text
SchoolX
├── SchoolX Talk   채널, DM, 회의, 사람·에이전트 협업
└── SchoolX Code   프로젝트, 로컬 작업, 에이전트 턴, 변경 검토
```

앱 전체를 `SchoolX Talk`로 바꾸고 그 안에 개발 환경을 넣으면 제품 의미가
메신저에 종속된다. 따라서 번들/제품군 표시는 당분간 `SchoolX`를 유지하고,
각 화면과 소개 문구에서 `SchoolX Talk`, `SchoolX Code`를 사용한다.

### 2.2 바꾸지 않는 이름

`buzz-*` 크레이트, Nostr kind, `buzz:` audience, relay wire 값,
localStorage 내부 키는 이 기능 때문에 이름을 바꾸지 않는다. 이 구분은
[`PRODUCT_IDENTITY.md`](PRODUCT_IDENTITY.md)의 제품 문자열/프로토콜 식별자/
내부 네임스페이스 계약을 그대로 따른다.

## 3. 목표와 범위

### 3.1 1차 목표

사용자가 프로젝트를 열고 다음 흐름을 한 화면에서 완료한다.

1. 새 작업을 만들거나 이전 작업을 이어서 연다.
2. 자연어로 작업을 요청한다.
3. 에이전트의 계획, 명령, 파일 변경, 진행 상태를 실시간으로 본다.
4. 추가 권한 요청을 정확한 대상과 범위로 검토한다.
5. 실행 중 방향을 바꾸거나 중지한다.
6. 변경 파일과 diff를 검토한다.
7. 테스트 결과를 확인하고 기존 Git/PR 기능으로 넘긴다.

### 3.2 1차 범위 밖

- VS Code 전체 기능을 복제하는 범용 IDE
- LSP, 디버거, 확장 프로그램 마켓플레이스
- 여러 사용자가 같은 로컬 터미널을 실시간 공동 조작하는 기능
- 모든 Codex 실험 API를 화면에 노출하는 것
- 로컬 전체 transcript를 자동으로 relay에 업로드하는 것
- `codex` 바이너리를 앱 번들에 즉시 포함하는 것

직접 코드 편집은 1차에 필수로 넣지 않는다. Codex형 제품의 중심은 에이전트
작업과 변경 검토다. 기존 Shiki 파일 뷰어와 diff를 먼저 사용하고, 실제 사용자
편집 수요를 확인한 뒤 Monaco/CodeMirror 도입을 별도 결정한다.

## 4. 현재 코드에서 재사용할 자산

| 현재 기능 | 위치 | SchoolX Code에서의 역할 |
|---|---|---|
| 프로젝트 상세 탭 | `features/projects/ui/ProjectWorkspaceTabs.tsx` | Code 진입점과 기존 Project 문맥 유지 |
| 로컬/원격 파일 탐색 | `ProjectRepositoryPanel.tsx` | 파일 트리와 읽기 전용 preview |
| Git snapshot/diff/branch/push | `commands/project_git*`, `shared/api/projectGit.ts` | 변경 검토와 Git handoff |
| PR files/inline comment | `ProjectPullRequestFilesChangedPanel.tsx` | 검토 UI 패턴 재사용 |
| 에이전트 세션 타임라인 | `ManagedAgentSessionPanel.tsx` 이하 | 메시지·계획·도구·권한 카드의 표시 계층 재사용 |
| OS 터미널 열기 | `commands/project_terminal.rs` | 고급 사용자를 위한 escape hatch 유지 |
| 관리형 에이전트 ACP | `crates/buzz-acp` | Talk 채널에 있는 상시 봇 전용 |
| 개발 MCP | `crates/buzz-dev-mcp` | 관리형 ACP 에이전트 전용, Code 터미널로 재사용하지 않음 |

`ManagedAgentSessionPanel`의 ACP raw event reducer에 Codex 이벤트를 억지로
주입하지 않는다. 대신 메시지/계획/도구/권한/결과라는 공통 표시 모델을
추출하고 ACP adapter와 Codex adapter가 각각 그 모델을 만든다.

## 5. 사용자 경험 설계

### 5.1 정보 구조

SchoolX Code는 프로젝트 상세에서 열리며 다음 네 영역을 사용한다.

```text
┌ 작업 사이드바 ┬──────── 대화·실행 타임라인 ────────┬ 변경/파일 검사기 ┐
│ 프로젝트       │ 작업 제목 · 모델 · worktree 상태   │ Changes / Files  │
│ 최근 작업      │ 사용자 요청                        │ 선택한 diff/file │
│ 새 작업        │ 계획 · 명령 · 결과 · 승인 카드     │                  │
│ 브랜치 상태    │                                     │                  │
│                │ 프롬프트 입력 · 실행/중지           │                  │
├────────────────┴─────────────────────────────────────┴──────────────────┤
│ 접을 수 있는 통합 터미널                                               │
└─────────────────────────────────────────────────────────────────────────┘
```

- 왼쪽: 현재 프로젝트의 Code 작업 스레드와 worktree 상태.
- 가운데: 한 스레드의 턴과 도구 활동. Codex와 가장 비슷해야 하는 핵심 영역.
- 오른쪽: `Changes`가 기본이며 `Files`로 전환 가능.
- 아래: `⌘J`로 여닫는 터미널. 초기 구현이 준비되기 전까지 기존
  `Open in Terminal`을 함께 제공한다.
- 창 너비가 좁으면 오른쪽 검사기는 별도 탭/드로어가 되고 가운데를 우선한다.

### 5.2 작업 생성

새 작업은 아래를 한 번에 결정한다.

- 프로젝트와 기준 branch
- 실행 위치: `Worktree`(기본) 또는 `Local checkout`
- 모델과 reasoning effort
- 권한 프로필

기본값은 별도 worktree, `workspace-write`, `on-request`다. 화면에는 기술 용어만
보이지 않게 다음처럼 설명한다.

- `프로젝트 안에서 작업` — 프로젝트 파일을 읽고 수정할 수 있음
- `추가 작업은 확인` — 프로젝트 밖 파일·네트워크·위험 명령은 먼저 물어봄

### 5.3 실행 타임라인

타임라인 항목은 다음 순서를 유지한다.

```text
User message
  → Plan / reasoning summary
  → Tool or command started
  → Streaming output (접힘, 마지막 줄 자동 추적)
  → File change / diff
  → Approval request (필요할 때)
  → Turn result
```

raw JSON은 기본 화면에 노출하지 않고 개발자 진단 메뉴에서만 연다. 긴 stdout은
가상화하고, 성공한 routine action은 toast 대신 inline 상태로 끝낸다.

### 5.4 승인 UX

승인 카드는 모달이 아니라 발생한 실행 위치에 inline으로 표시한다.

- 어떤 명령/파일/호스트인지 요약
- 요청 이유
- 허용되는 정확한 범위
- `이번만 허용`, 지원되는 경우 `이 세션에서 허용`, `거부`
- 키보드 포커스와 단축키

화면에 보인 request ID, thread ID, turn ID가 현재 native pending request와 모두
일치할 때만 응답을 보낸다. 오래된 카드나 이미 끝난 턴의 승인 버튼은 즉시
비활성화한다.

### 5.5 macOS 동작

- 사이드바와 검사기는 접을 수 있고 마지막 폭을 복원한다.
- 모든 row와 icon button은 hover, context menu, VoiceOver label을 가진다.
- `⌘P` 파일 빠른 열기, `⌘⇧P` 명령 팔레트, `⌘J` 터미널,
  `⌘B` 작업 사이드바, `Esc` popover/일시 입력 취소를 제공한다.
- 키보드만으로 작업 선택, 프롬프트 전송, 승인, 중지가 가능해야 한다.
- 파일을 Finder에서 작업 입력으로 drag & drop할 수 있어야 한다.
- Reduce Motion/Transparency, Increase Contrast를 따른다.

## 6. 브랜드와 시각 체계

브랜드 원본은 `brand/` 폴더를 단일 출처로 사용한다.

| 토큰 | 원본 | Light 역할 | Dark 역할 |
|---|---|---|---|
| Parchment | `#F4EDDD` | 전체 canvas의 가장 큰 비율 | 주요 text/밝은 강조 |
| Pine | `#355649` | navigation/selection/primary | 어두운 panel tint/selection |
| Terracotta | `#B85A3C` | 실행·주의 CTA에 제한 | `#D97958` 접근성 보정 action |
| Ink | `#1F2937` | text, 가장 적은 면적 | 전체 canvas의 가장 큰 비율 |
| Warm Gold | `#D7A94B` | pending/highlight | pending/highlight |
| Sage | `#7F967A` | secondary/success | secondary/success |

사용자 요구에 따라 Light mode에서는 Parchment 계열이 가장 큰 면적을 차지하고,
Ink/Pine 같은 어두운 색 면적은 최소화한다. Dark mode에서는 Ink를 canvas로
사용하되 순수 검정은 쓰지 않고, Terracotta는 action과 경고에만 제한한다.

컴포넌트에 hex를 직접 쓰지 않는다. `desktop/src/index.css`의 의미 토큰으로
매핑하고 기존 `bg-background`, `text-foreground`, `border-border`,
`bg-primary` 계열을 통해 사용한다. 코드/터미널만 SF Mono 또는 시스템
monospace를 사용하고 일반 UI는 현재 글꼴 체계를 유지한다.

## 7. 실행 아키텍처

### 7.1 경계

```text
React UI
  │ typed Tauri commands + Tauri events
  ▼
SchoolX Code frontend adapter/reducer
  │
  ▼
Tauri/Rust CodeRuntime
  ├── Codex 프로세스 lifecycle
  ├── JSONL JSON-RPC request correlation
  ├── server request/approval gate
  ├── canonical workspace root validation
  ├── Git worktree lifecycle
  └── redacted diagnostics
       │ stdin/stdout (JSONL)
       ▼
codex app-server --listen stdio://
```

WebSocket transport는 첫 버전에서 사용하지 않는다. 로컬 child stdio가 노출
면적이 가장 작고, 실험 WebSocket 인증과 port lifecycle이 필요 없다.

### 7.2 프로세스 단위

데스크톱 프로세스당 `app-server` 하나를 둔다. app-server 하나가 여러 thread를
관리하므로 프로젝트마다 프로세스를 만들 필요가 없다.

상태는 다음과 같다.

```text
NotInstalled → Stopped → Starting → Initializing → Ready
                              └──────────────→ Failed
Ready → Recovering → Ready
Ready → Stopping → Stopped
```

- 첫 Code 화면 진입 때 lazy start한다.
- `initialize` 응답 후 `initialized` notification을 보낸 뒤에만 Ready다.
- stdout 한 줄은 JSON-RPC 한 메시지다. stderr는 프로토콜로 파싱하지 않는다.
- 앱 종료 시 pending request를 취소하고 child process group을 정리한다.
- 예기치 않은 종료는 1회 자동 재시작하고 기존 thread ID로 resume한다.
- 반복 실패는 재시작 loop 대신 명확한 복구 화면을 보여준다.

### 7.3 Codex 프로토콜 사용 범위

1차 vertical slice에서 필요한 메서드만 허용한다.

| 목적 | app-server 메서드 |
|---|---|
| handshake | `initialize`, `initialized` |
| 작업 목록/복구 | `thread/list`, `thread/read`, `thread/start`, `thread/resume` |
| 실행 | `turn/start`, `turn/steer`, `turn/interrupt` |
| 검토 | `review/start` |
| 모델 | `model/list` |
| 터미널 | `command/exec`, `/write`, `/resize`, `/terminate` |
| 승인 | `item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, `item/permissions/requestApproval` |

알 수 없는 notification은 저장하지 않고 진단 로그에 이름만 기록한다. 알 수 없는
server request는 자동 허용하지 않고 `method not supported`로 fail closed한다.

### 7.4 프로토콜 버전 관리

`app-server`는 실험 인터페이스이므로 손으로 전체 타입을 복사하지 않는다.

1. 개발 시 설치된/pinned `codex`에서 `codex app-server generate-ts`와
   `generate-json-schema`를 실행한다.
2. 생성물 전체가 아니라 SchoolX가 사용하는 메서드의 schema snapshot과
   생성 버전/해시를 저장한다.
3. native transport는 JSON-RPC envelope를 파싱하고, method별 payload는 좁은
   SchoolX DTO로 변환한다.
4. CI fixture app-server가 handshake, delta, approval, crash/resume 계약을
   재생한다.
5. 새 Codex 버전은 compatibility test를 통과한 뒤 지원 목록에 추가한다.

현재 확인 버전 `0.145.0`을 최초 fixture 기준으로 쓰되, 이것을 영구 API로
가정하지 않는다.

## 8. 로컬 데이터와 relay 데이터

### 8.1 저장 책임

| 데이터 | 저장 위치 | 비고 |
|---|---|---|
| Codex thread/turn 원본 | Codex가 관리하는 `$CODEX_HOME` | SchoolX가 포맷을 복제하지 않음 |
| 프로젝트↔thread 연결 | SchoolX app data의 versioned index | thread ID, project ID, worktree ID만 |
| worktree | `~/.schoolx/WORKTREES/...` | 제품 경계 안에서 관리 |
| 사용자 설정 | SchoolX app data | panel 폭, 기본 권한, 최근 모델 |
| Talk 메시지 | relay/Nostr | 기존 계약 유지 |
| Code transcript | 기본은 로컬 | 명시적 공유만 Talk로 게시 |

커뮤니티를 전환해도 로컬 Codex 프로세스 자체를 무조건 죽이지 않는다. 다만
프로젝트↔thread index의 key는 `community + project dtag + repository identity`로
분리해 다른 커뮤니티의 작업이 섞이지 않게 한다.

### 8.2 Talk와 Code의 연결

자동 동기화하지 않는다. 사용자가 다음 action을 선택할 때만 relay로 보낸다.

- `Talk에 진행 상황 공유`
- `변경 요약을 채널에 게시`
- `PR 검토 요청`

공유되는 것은 요약과 사용자가 선택한 diff/링크뿐이다. 환경 변수, 절대 경로,
명령 전체 stdout, hidden reasoning은 기본적으로 제외한다.

## 9. Worktree와 Git 안전성

### 9.1 기본 정책

한 Code thread는 하나의 execution root에 고정된다.

```text
CodeThreadBinding {
  community_id
  project_dtag
  repository_identity
  codex_thread_id
  execution_mode: worktree | local
  execution_root
  base_ref
  worktree_id?
}
```

- 기본 worktree root: 활성 SchoolX nest의
  `WORKTREES/<repository-hash>/<worktree-id>`. dev/release nest 경계를 섞지 않는다.
- versioned binding index와 미완료 preparation journal은 Tauri app data의
  `code/thread-bindings.json`에 함께 저장한다.
- native가 canonicalize한 절대 경로만 Codex `cwd`와 turn
  `sandboxPolicy.writableRoots`에 전달한다. 현재 stable Codex app-server
  0.145 계약에 없는 top-level `runtimeWorkspaceRoots`는 보내지 않는다.
- symlink를 따라간 최종 경로가 허용 root 밖이면 거부한다.
- 같은 worktree를 두 active thread가 동시에 쓰지 못한다.
- 삭제는 clean/merged 여부를 확인한 별도 사용자 action으로만 수행한다.
- uncommitted 변경이 있는 worktree는 자동 삭제하지 않는다.
- Unix의 managed-worktree Git mutation은 검증된 디렉터리 handle을 helper stdin으로
  넘기고 `fchdir`한 뒤 실행한다. 따라서 검증 뒤 같은 pathname이 symlink나 다른
  디렉터리로 교체되어도 Git이 대체 경로로 redirect되지 않는다.
- 다만 portable한 비특권 POSIX API는 동일 UID 프로세스가 실행 중 pinned inode나
  그 조상을 rename하는 것까지 막지 못한다. native는 실행 후 전체 이름 경로와
  device/inode chain을 다시 검증해 binding persistence를 fail closed하지만, 그
  검사 전에 이미 pinned inode에 생긴 Git side effect를 자동 되돌리지는 않는다.

### 9.2 Local checkout 모드

사용자가 선택하면 현재 checkout에서 실행할 수 있으나, 시작 전에 dirty 상태와
현재 branch를 보여준다. 이 모드에서는 SchoolX가 branch 전환, reset, clean을
자동 실행하지 않는다.

## 10. 권한과 보안 계약

### 10.1 기본 권한

- `sandbox = workspace-write`
- `approvalPolicy = on-request`
- workspace root는 선택한 execution root 하나
- 프로젝트 밖 write는 승인 필요
- 네트워크는 필요한 작업에서 명시적 권한 요청
- `danger-full-access`는 기본 UI에 두지 않고 고급 설정 + 강한 경고로만 제공

### 10.2 반드시 지킬 불변식

1. frontend 문자열을 command line으로 이어 붙이지 않는다.
2. app-server 실행 파일은 discovery에서 얻은 canonical executable만 쓴다.
3. JSONL line 크기, pending request 수, stdout buffer 크기에 상한을 둔다.
4. pending approval은 `(connection generation, request id, thread id, turn id)`에
   귀속한다.
5. child 재시작 후 이전 generation의 승인 응답은 거부한다.
6. auth token, environment secret, private key를 Tauri event나 일반 로그에 싣지
   않는다.
7. stderr와 tool output을 저장하기 전 기존 redaction 정책을 적용한다.
8. 앱 종료/창 종료/프로젝트 분리 때 PTY와 child process를 orphan으로 남기지
   않는다.
9. Code transcript는 사용자 action 없이 Nostr event로 만들지 않는다.
10. `buzz-acp`의 auto-approve와 Codex Code approval은 별도 정책이다.

## 11. 코드 구조

### 11.1 Native

```text
desktop/src-tauri/src/
├── code_workspace/
│   ├── mod.rs
│   ├── runtime.rs          # child lifecycle/state machine
│   ├── jsonrpc.rs          # JSONL framing, ids, pending map
│   ├── protocol.rs         # 허용한 method의 DTO/normalization
│   ├── approvals.rs        # pending approval gate
│   ├── paths.rs            # executable/workspace canonicalization
│   ├── bindings.rs         # versioned binding/preparation persistence
│   ├── discovery.rs        # executable/version discovery와 0.145.x gate
│   └── worktrees.rs        # git identity와 descriptor-bound worktree 준비
└── commands/
    └── code_workspace.rs   # 얇은 Tauri command facade
```

`AppState`에는 `Arc<CodeRuntime>` 하나를 넣는다. command 함수가 child stdin/stdout
lock을 직접 만지지 않으며 모두 runtime actor로 요청한다.

### 11.2 Frontend

```text
desktop/src/features/code/
├── api/
│   ├── codeWorkspace.ts    # Tauri invoke/event adapter
│   └── types.ts            # normalized public DTO
├── state/
│   ├── codeSessionReducer.ts
│   ├── codeSessionStore.ts
│   └── codeSessionQueries.ts
├── ui/
│   ├── CodeWorkspaceScreen.tsx
│   ├── CodeThreadSidebar.tsx
│   ├── CodeTimeline.tsx
│   ├── CodeComposer.tsx
│   ├── CodeApprovalCard.tsx
│   ├── CodeInspector.tsx
│   ├── CodeChangesPanel.tsx
│   └── CodeTerminalDrawer.tsx
└── lib/
    ├── codexEventAdapter.ts
    └── codeThreadBinding.ts
```

공통 transcript 렌더러는 `features/agents`에서 중립적인 위치로 단계적으로
추출한다. 1차 구현에서 대규모 이동을 먼저 하지 않고 필요한 leaf component만
공유해 upstream 충돌을 줄인다.

### 11.3 Native API 초안

```text
code_runtime_probe
code_runtime_start
code_runtime_stop
code_runtime_status
code_runtime_events

code_thread_preparations_list
code_threads_list
code_thread_start
code_thread_binding_recover
code_thread_resume
code_turn_start
code_turn_steer
code_turn_interrupt
code_approval_respond

code_worktree_prepare
code_worktree_status
```

터미널과 worktree 제거 command는 Phase 2 범위이며 현재 구현하지 않는다.

Native가 frontend로 보내는 event는 raw app-server 메시지가 아니라 다음
normalized envelope다.

```text
CodeWorkspaceEvent {
  scope
  runtime_generation
  sequence
  thread_id?
  turn_id?
  item_id?
  kind
  payload
}
```

`sequence`는 UI reconnect/reducer dedup에 쓰며, 승인 응답 권한은 갖지 않는다.

## 12. 구현 단계

### Phase 0 — 계약 고정

- 이 문서 승인
- 현재 Codex generated schema snapshot과 fixture 작성
- 기능 flag `schoolxCodeWorkspace` 추가
- `CodeRuntime` lifecycle/JSONL 단위 테스트 작성

완료 기준: 실제 앱 UI 없이 fake app-server와 initialize/notification/request/
response/crash round-trip이 통과한다.

### Phase 1 — 첫 수직 슬라이스

- 프로젝트에서 `Open in SchoolX Code`
- app-server probe/start + initialize
- worktree 준비
- thread start/resume
- prompt 전송과 streamed timeline
- steer/interrupt
- command/file/permission 승인 카드
- 기존 Git diff를 사용하는 Changes inspector
- 종료 후 다시 열었을 때 thread 복구

완료 기준: 사용자가 작은 수정 요청을 보내고, 명령을 승인하고, 변경 diff를
확인하고, 앱 재시작 뒤 같은 thread를 이어갈 수 있다.

### Phase 2 — 터미널과 작업 관리

- PTY 기반 terminal drawer
- terminal resize/stdin/terminate
- thread 검색/이름 변경/archive/fork
- worktree 목록, 보존, 안전한 제거
- model/reasoning selector

### Phase 3 — Git handoff와 Talk 공유

- stage/unstage/commit UI
- branch/push/PR 기존 기능과 연결
- inline diff comment를 다음 turn context로 전달
- 선택적 Talk 공유
- review/start 기반 코드 검토

### Phase 4 — 편집기 평가

- 직접 편집 사용성 데이터 수집
- Monaco와 CodeMirror의 번들 크기, IME, 접근성, 대용량 파일 성능 비교
- 필요할 때만 편집기 도입

## 13. 테스트 전략

### 13.1 Rust

- JSONL partial/multiple/invalid/oversized line
- request ID correlation과 timeout
- out-of-order notification
- unknown server request fail closed
- approval generation binding
- process crash/restart/stop race
- canonical path, symlink escape, worktree root containment
- dirty worktree remove 거부
- stderr/auth/env redaction

### 13.2 TypeScript

- notification delta reducer
- duplicate/out-of-order sequence 처리
- active turn과 approval card 연결
- reconnect snapshot + live event merge
- community/project/thread storage key 격리
- keyboard/focus state

### 13.3 Desktop E2E

mock Tauri bridge에 app-server fixture event를 넣고 다음 화면을 검증한다.

- empty/new task
- running plan/command/file change
- approval pending/accepted/declined
- interrupted turn
- runtime missing/crashed/recovered
- light/dark, 800×500 minimum, wide split layout
- keyboard-only, VoiceOver label, reduced motion

E2E build는 저장소 규칙대로 반드시 `pnpm build:e2e` 경로를 사용한다.

## 14. 관측과 장애 UX

진단에는 다음만 남긴다.

- app-server version과 runtime generation
- method name, request ID, duration, result category
- thread/turn/item의 짧은 opaque ID
- process exit status
- redaction된 protocol error

파일 내용, prompt 전문, stdout 전문, auth/env 값은 기본 진단에 남기지 않는다.

장애 문구는 원인과 다음 action을 구분한다.

| 상태 | 사용자 안내 |
|---|---|
| Codex 없음 | 설치 위치를 찾지 못함 + 다시 확인/설치 안내 |
| 초기화 실패 | 버전과 호환성 확인 + 진단 복사 |
| thread resume 실패 | 원래 worktree 보존 + 새 thread로 복구 옵션 |
| worktree 충돌 | 충돌 경로/dirty 상태 + 수동 열기 |
| 승인 만료 | 실행이 이미 끝났음을 inline 표시 |
| 프로세스 crash | 1회 자동 복구 후 실패 시 수동 재시작 |

## 15. 첫 구현에서 수정할 파일 범위

예상 첫 PR은 아래로 제한한다.

- 새 `desktop/src-tauri/src/code_workspace/**`
- 새 `desktop/src-tauri/src/commands/code_workspace.rs`
- `AppState`에 runtime state 등록
- `commands/mod.rs`, `lib.rs`에 command/event wiring
- 새 `desktop/src/features/code/**`
- 프로젝트 상세에 Code workspace 진입점 한 곳
- mock bridge + unit/E2E fixture
- semantic brand token 보완

기존 `buzz-acp`, `buzz-dev-mcp`, relay protocol, managed agent runtime 동작은 첫
PR에서 바꾸지 않는다. 이렇게 해야 현재 Talk 봇 동작과 새 로컬 Code 실행을
독립적으로 검증할 수 있다.

## 16. 결정 기록

| 항목 | 결정 |
|---|---|
| 실행 인터페이스 | ACP adapter가 아니라 `codex app-server` 직접 통합 |
| transport | 첫 버전 stdio JSONL |
| process 수 | desktop process당 하나 |
| 기본 실행 위치 | thread별 Git worktree |
| 기본 권한 | `workspace-write` + `on-request` |
| transcript 저장 | Codex 원본 + SchoolX에는 binding index만 |
| relay 동기화 | 자동 없음, 사용자 선택 공유만 |
| editor | 1차 제외, viewer/diff/terminal 우선 |
| 브랜딩 | SchoolX 제품군 아래 SchoolX Talk + SchoolX Code |
| 내부 Buzz 이름 | protocol/namespace 호환을 위해 유지 |

## 17. 참고 자료

- OpenAI, [Codex app-server](https://learn.chatgpt.com/docs/app-server)
- OpenAI, [Local environments and built-in Git tools](https://learn.chatgpt.com/docs/environments/local-environment#use-built-in-git-tools)
- OpenAI, [Git worktrees](https://learn.chatgpt.com/docs/environments/git-worktrees)
- OpenAI, [Sandboxing and permissions](https://learn.chatgpt.com/docs/sandboxing#how-permissions-work)
- SchoolX, [`PRODUCT_IDENTITY.md`](PRODUCT_IDENTITY.md)
- SchoolX, [`SECURITY_CONTRACT.md`](SECURITY_CONTRACT.md)
