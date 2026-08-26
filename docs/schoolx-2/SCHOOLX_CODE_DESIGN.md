# SchoolX Code — Codex형 로컬 개발 환경 설계

상태: 구현 기준안 v0.2
작성일: 2026-08-13
최종 갱신일: 2026-08-25
대상: SchoolX 데스크톱(Tauri 2 + React 19)

구현 현황: Phase 0 runtime부터 Phase 1D typed adapter/pure reducer, Phase 1E UI
수직 슬라이스를 거쳐 Phase 1F normalized event/recovery E2E와 exact bound-thread
Changes inspector, Phase 1F.1 freshness closure, 역사적 Codex 0.145 permission 보완,
실제 managed-worktree Changes 회귀, UI lifecycle/exact E2E bridge와 최초 0.145
app-server manual boundary, 현재 exact 0.149.0 schema/wire snapshot과 recovery 호환성,
Git replacement-object immutable-base 차단과 native runtime
diagnostic egress redaction, cross-platform descendant cleanup, permission display/authority
분리, authoritative runtime checkpoint, generation Changes/prompt reconciliation과
Changes completeness/status closure, Phase 2 exact bound-thread PTY terminal과 exact-bound
검색/이름 변경, persisted archive/unarchive lifecycle authority, clean managed-source
`thread/fork`, read-only exact-scope managed-worktree inventory, public safe removal과
runtime-generation model/reasoning selector와 Phase 3 Git write public contract, strict durable transaction,
owned-lock/CAS/startup recovery, Native admission gate와 remount-safe frontend recovery UX까지 구현했다.
Crash/response-loss 행렬과 전체 Native/frontend/fresh-build E2E 회귀도 통과했다. 세 native Git helper의
process-launch authority도 선택 B(signed unprivileged macOS XPC + Linux pinned direct spawn)로 구현했다.
현재 Codex 계약, 제품 진입점, 운영 증거와 다음 Phase 3 decision gate는
[`SESSION_HANDOFF_20260825_CODEX_0_149_AND_NEXT_SLICE_DECISION.md`](SESSION_HANDOFF_20260825_CODEX_0_149_AND_NEXT_SLICE_DECISION.md)를
우선한다. Artifact별 최신 검증과 canonical release 잔여 gate는
[`SESSION_HANDOFF_20260825_CODE_RELEASE_READINESS.md`](SESSION_HANDOFF_20260825_CODE_RELEASE_READINESS.md)를
기준으로 한다. 내부 SchoolX 시각 체계의 적용/미적용 경계와 다음 구현 slice는
[`SESSION_HANDOFF_20260825_SCHOOLX_VISUAL_SYSTEM_V1.md`](SESSION_HANDOFF_20260825_SCHOOLX_VISUAL_SYSTEM_V1.md)를
우선한다. Launch authority의 원래 지원 범위, fault 결과와 residual은
[`SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY_DECISION.md`](SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY_DECISION.md)를
기준으로 하고, 원래 착수 조건은
[`SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY.md`](SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY.md),
직전 완료 경계는
[`SESSION_HANDOFF_20260821_CODE_PHASE3_GIT_WRITE_IMPLEMENTATION.md`](SESSION_HANDOFF_20260821_CODE_PHASE3_GIT_WRITE_IMPLEMENTATION.md)와
원본 착수 계약인
[`SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md`](SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md)를
대조한다. Phase 2 직전 완료 경계는
[`SESSION_HANDOFF_20260819_CODE_PHASE2_MODEL_SELECTOR.md`](SESSION_HANDOFF_20260819_CODE_PHASE2_MODEL_SELECTOR.md),
이전 구현 경계는
[`SESSION_HANDOFF_20260819_CODE_PHASE2_PUBLIC_REMOVAL.md`](SESSION_HANDOFF_20260819_CODE_PHASE2_PUBLIC_REMOVAL.md),
[`SESSION_HANDOFF_20260816_CODE_PHASE2_FORK.md`](SESSION_HANDOFF_20260816_CODE_PHASE2_FORK.md),
[`SESSION_HANDOFF_20260816_CODE_PHASE2_ARCHIVE.md`](SESSION_HANDOFF_20260816_CODE_PHASE2_ARCHIVE.md),
[`SESSION_HANDOFF_20260816_CODE_PHASE2.md`](SESSION_HANDOFF_20260816_CODE_PHASE2.md),
[`SESSION_HANDOFF_20260814_CODE_PHASE1F.md`](SESSION_HANDOFF_20260814_CODE_PHASE1F.md),
[`SESSION_HANDOFF_20260814_CODE_PHASE1E.md`](SESSION_HANDOFF_20260814_CODE_PHASE1E.md),
[`SESSION_HANDOFF_20260814_CODE_PHASE1D.md`](SESSION_HANDOFF_20260814_CODE_PHASE1D.md),
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
승인, 스트리밍 이벤트, 인증 계약을 사용한다. 현재 exact schema/wire와 설치
동작을 검증한 버전은 `codex-cli 0.149.0`이며 `codex app-server`, `generate-ts`,
`generate-json-schema`를 제공한다. `0.145.0`은 최초 역사적 baseline으로 보존되고
runtime gate도 두 audited minor의 numeric patch를 호환 범위로 유지하지만, 최신
exact 증명 snapshot은 `0.149.0`이다.

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
| 프로젝트 카드/목록과 상세 탭 | `features/projects/ui/ProjectCards.tsx`, `ProjectWorkspaceTabs.tsx` | 두 Code 진입점과 기존 Project 문맥 유지 |
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

SchoolX Code는 프로젝트 카드/목록의 직접 action이나 프로젝트 상세 action에서
열리며 다음 네 영역을 사용한다.

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
- 아래: `⌘J`로 여닫는 터미널. 기존 `Open in Terminal`도 고급 사용자용
  escape hatch로 함께 제공한다.
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

> **현재 구현 상태 (2026-08-26):** Dock/app bundle, tray, DMG에 이어 webview의
> first-party theme, Appearance preview/label, flat·opaque core shell, boot와
> community-switch mark, favicon, Nostr bind/Mobile QR app icon이 SchoolX V1을
> 사용한다. internal ID와 third-party theme 호환/cache/E2E 계약은
> [`SESSION_HANDOFF_20260825_SCHOOLX_VISUAL_SYSTEM_V1.md`](SESSION_HANDOFF_20260825_SCHOOLX_VISUAL_SYSTEM_V1.md)를
> 따른다. V1은 core shell까지이며 first-run onboarding과 일부 legacy branded
> status surface의 재설계는 후속이다.

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

컴포넌트에 hex를 직접 쓰지 않는다. 실제 전역 theme entry인
`desktop/src/shared/styles/globals/theme.css`와 theme resolver의 의미 토큰으로
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
- 앱 종료 시 pending request를 취소하고 child tree를 정리한다. Unix는 app-server 전용
  process group에 TERM/grace/KILL을 적용하고, Windows는 kill-on-close Job Object를 사용한다.
- leader가 먼저 종료해도 descendant cleanup 뒤 leader를 reap한다. Windows Job 배치 실패는
  runtime start 자체를 fail closed한다.
- 예기치 않은 종료는 1회 자동 재시작하고 기존 thread ID로 resume한다.
- 반복 실패는 재시작 loop 대신 명확한 복구 화면을 보여준다.

### 7.3 Codex 프로토콜 사용 범위

1차 vertical slice에서 필요한 메서드만 허용한다.

| 목적 | app-server 메서드 |
|---|---|
| handshake | `initialize`, `initialized` |
| 작업 목록/복구 | `thread/list`, `thread/read`, `thread/start`, `thread/resume` |
| 작업 분기 | `thread/fork` |
| 작업 메타데이터 | `thread/name/set` |
| 작업 lifecycle | `thread/archive`, `thread/unarchive` |
| 실행 | `turn/start`, `turn/steer`, `turn/interrupt` |
| 검토 | `review/start` |
| 모델 | `model/list` |
| app-server 1회성 command(통합 터미널과 별도) | `command/exec`, `/write`, `/resize`, `/terminate` |
| 승인 | `item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, `item/permissions/requestApproval` |

현재 audited 0.145/0.149 수직 슬라이스는 `thread/name/set`, `thread/archive`,
`thread/unarchive`, `thread/fork`, `model/list`를 활성화한다. `command/exec` 계열은 아직
활성화하지 않는다. 통합 터미널은 app-server RPC가 아니라 native가 소유하는 별도 OS shell
PTY다.

Model selector는 pinned catalog를 cursor 끝까지 bounded pagination하고
`includeHidden:false`인 visible row만 사용한다. Model id와 실제 wire `model` 값은 같다고
가정하지 않으며, reasoning effort도 닫힌 enum으로 복제하지 않고 선택한 model이 광고한
non-empty 문자열만 허용한다. Catalog는 runtime generation에 결박하고 duplicate id/model,
duplicate effort, 지원 목록에 없는 default effort, 복수 default, cursor cycle과 page/model cap
초과를 모두 publish 전에 거부한다.

`thread/start`의 non-null model과 `turn/start`의 non-null model/effort pair는 native가 같은
runtime mutex를 잡은 채 fresh catalog로 다시 검증한 뒤 JSONL byte write까지 이어 간다.
`turn/start`는 model과 effort가 모두 null이거나 모두 non-null이어야 하며 `turn/steer`에는
선택값을 싣지 않는다. Ordinary resume과 response-loss recovery는 model override를 보내지 않고,
start/resume/fork 응답의 top-level model과 nullable reasoning effort를 현재 thread authority로
사용한다. 응답이 현재 visible catalog에 없는 legacy/hidden model이어도 이미 성공한 mutation을
input failure로 다시 분류하지 않는다.

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

`0.145.0`은 최초 fixture이자 역사적 compatibility baseline으로 보존한다.
현재 최신 exact fixture는 `0.149.0`이다. Runtime은 `0.145.<numeric patch>`와
`0.149.<numeric patch>`를 admit하지만, 모든 patch의 schema가 동일하다는 뜻은
아니다. 두 exact snapshot 모두 영구 API로 가정하지 않는다.

## 8. 로컬 데이터와 relay 데이터

### 8.1 저장 책임

| 데이터 | 저장 위치 | 비고 |
|---|---|---|
| Codex thread/turn 원본 | Codex가 관리하는 `$CODEX_HOME` | SchoolX가 포맷을 복제하지 않음 |
| 프로젝트↔thread 연결 | SchoolX app data의 versioned index | thread ID, project ID, worktree ID만 |
| worktree | `~/.schoolx/WORKTREES/...` | 제품 경계 안에서 관리 |
| 사용자 설정 | SchoolX app data | panel 폭, 기본 권한, `code/model-selection.json`의 최근 model/effort |
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
  `sandboxPolicy.writableRoots`에 전달한다. Audited Codex app-server 0.145/0.149
  계약에 없는 top-level `runtimeWorkspaceRoots`는 보내지 않는다.
- symlink를 따라간 최종 경로가 허용 root 밖이면 거부한다.
- 같은 worktree를 두 active thread가 동시에 쓰지 못한다.
- 실제 삭제는 clean/merged 여부를 확인한 별도 사용자 action으로만 수행하며,
  [`SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md`](SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md)의
  merged authority, durable journal, transcript tombstone, pinned deletion boundary가 모두
  구현·검증되기 전에는 command나 button을 노출하지 않는다.
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

구현된 runtime은 canonical executable과 raw version을 native 내부 spawn/compatibility
authority로만 유지한다. Tauri로 반환하는 probe/status에서는 executable display path,
version, initialize metadata, RPC/stderr error와 `lastError`를 동일한 protocol redactor로
정리한다. SchoolX Changes Git command는 replacement objects도 강제로 비활성화해 persisted
base OID의 tree 의미를 local `refs/replace/*`가 바꾸지 못하게 한다.

Permission approval의 raw authority도 native pending store만 소유한다. frontend에는
deterministic typed `permissionDisplay`만 노출하고, frontend 응답은 grant/decline intent와
turn/session lifetime만 포함한다. native는 정확하고 non-empty인 display에 한해 pending
request의 전체 raw permissions를 복원하며, malformed/empty/redaction-loss display의 grant는
fail closed한다. decline은 canonical empty turn response다.

읽기 전용 Changes command는 persisted exact binding의 immutable base와 execution root만
사용한다. tracked/untracked 전체 manifest를 먼저 정렬한 뒤 250개 반환 한도를 적용하고,
`totalFiles`, `filesTruncated`, closed status
(`added`, `modified`, `deleted`, `typeChanged`, `unmerged`, `untracked`)와 binary 여부를
전달한다. additions/deletions는 반환된 파일 subset의 patch-truncation 전 count다. unmerged는
base-relative name-status 결과와 별도로 `diff-filter=U`를 읽어 실제 working-tree conflict를
보존한다.

Changes read는 strict UTF-8, literal pathspec, replacement-object/filter 차단, untracked
`openat`/`O_NOFOLLOW` descriptor, 파일별 2,000줄/256 KiB patch 상한을 지킨다. 초기/최종
tracked numstat/status/unmerged와 untracked path inventory 또는 per-patch count가 달라지면 전체
read를 한 번 재시도하고 반복 drift는 fail closed한다. 이는 atomic filesystem snapshot은
아니다. 동일 count의 tracked content 변경과 untracked content-only race까지 막으려면 helper
내부 streaming digest 또는 immutable filesystem snapshot이 필요하다. symlink를 따라가고
mode/type/gitlink를 놓치는 `git hash-object --no-filters -- <path>`는 이 hardening에 사용하지
않는다.

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
│   ├── discovery.rs        # executable/version discovery와 0.145.x/0.149.x gate
│   ├── model_catalog.rs    # generation-bound model catalog와 최근 선택
│   ├── worktrees.rs        # git identity와 descriptor-bound worktree 준비
│   └── terminal.rs         # exact bound-thread PTY session actor/ownership
└── commands/
    ├── code_workspace.rs          # core Code command facade
    ├── code_thread_management.rs  # exact-bound metadata mutation facade
    └── code_terminal.rs           # typed PTY command facade
```

`AppState`에는 app-server용 `Arc<CodeRuntime>`과 OS shell PTY용
`Arc<CodeTerminalManager>`를 별도로 둔다. command 함수가 child stdin/stdout lock을
직접 만지지 않으며 각 process/session authority의 actor로 요청한다.

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
code_models_list
code_model_selection_set

code_repository_inspect
code_thread_changes
code_thread_preparations_list
code_threads_list
code_thread_rename
code_thread_archive
code_thread_unarchive
code_thread_start
code_thread_fork
code_thread_binding_recover
code_thread_resume
code_turn_start
code_turn_steer
code_turn_interrupt
code_approval_respond

code_worktree_prepare
code_worktree_status
code_worktrees_list
code_worktree_remove

code_terminal_open
code_terminal_resize
code_terminal_stdin
code_terminal_terminate
```

터미널, worktree removal과 model selector command는 Phase 2 수직 슬라이스로 구현했다.
Frontend는 cwd authority를 갖지 않지만 native root revalidation과
`portable-pty` spawn 사이의 pathname replacement/disappearance TOCTOU 및 library의
missing-cwd fallback은 남은 platform/path residual이다.

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

Replay backlog는 필요할 때 다음 authoritative transient checkpoint를 함께 반환한다.

```text
CodeEventCheckpoint {
  runtime_generation
  sequence_watermark
  active_turns[]
  pending_approvals[] { event, respondable }
}
```

full replay, runtime generation 변경, ring truncation은 checkpoint를 만든다. command boundary는
현재 exact binding scope의 thread만 남긴다. frontend는 listener를 먼저 등록해 live event를
buffer하고 replay를 적용한 뒤 checkpoint로 active turn/approval state를 overwrite하며,
watermark 이후 buffered event만 적용한다. transcript가 잘렸다면 checkpoint만으로 완료하지
않고 selected thread를 exact `thread/read`로 한 번 다시 읽는다. generation/epoch/selection이
달라진 refresh 결과는 폐기하고, refresh 중 write interaction은 차단한다.

Changes cache key에는 runtime generation을 포함한다. 새 generation은 synchronized replay를
첫 baseline으로 삼아 native Changes를 한 번 읽고, 이후 같은 generation의
`turn/diff/updated`, `item/fileChange/patchUpdated`, `turn/completed`만 invalidate한다. Timeline의
optimistic user prompt는 persisted snapshot row와 exact `[turnId, text]`별 개수로 상쇄해
generation resume 뒤 중복 표시를 막는다.

Frontend Changes decode는 빈/중복 path, binary file의 non-zero count, 반환 subset 합계와
top-level 합계의 불일치, `totalFiles`/`filesTruncated` 모순을 거부한다. inspector는 partial
file list와 per-file patch truncation을 별도로 알리고, replay synchronization 중에는 최초
read와 Refresh를 막는다. 같은 generation의 authoritative refresh 완료는 exact query 하나만
invalidate한다.

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

현재 Phase 1 구현은 bounded replay ring 밖으로 밀려난 active turn/approval 복구,
cross-platform app-server child-tree ownership, permission display/authority 분리와 generation
전환의 Changes/prompt 정합성, Changes의 complete manifest/status/binary/bounded patch와
drift retry까지 포함한다. Phase 2의 exact bound-thread terminal과 로컬 bound-result 검색,
audited 0.145/0.149 `thread/name/set` 이름 변경, leaf-only archive/unarchive lifecycle authority도
완료했다. clean managed-source의 전체 persisted history를 fresh destination worktree로
분기하는 fork와 exact-scope managed-worktree inventory도 완료했다. Native proof-based eligibility,
strict public remove command/receipt, explicit confirmation과 response-loss-safe authoritative reconciliation을
포함한 safe worktree removal 수직 슬라이스도 완료했다. Runtime-generation-bound catalog,
model/reasoning selector, installation-global recent preference와 thread-open authority 복구까지
완료했고 Phase 3의 whole-file stage/unstage와 staged-only commit handoff도 완료했다.

### Phase 2 — 터미널과 작업 관리

- 첫 수직 슬라이스: exact bound-thread PTY session ownership과 terminal drawer
- typed terminal resize/stdin/terminate와 `⌘J` lifecycle
- exact-scope bound-result 로컬 검색과 audited 0.145/0.149 thread 이름 변경
- persisted lifecycle authority와 leaf-only thread archive/unarchive
- fresh destination worktree를 사용하는 thread fork
- worktree 목록, 보존, 안전한 제거
- model/reasoning selector

Exact-scope durable binding/preparation에서 파생한 managed-worktree inventory와 보존/제거불가 사유,
native positive proof에 결박된 eligibility까지 구현했다. Merged authority/proof, crash-safe durable journal,
제거 뒤 binding/transcript tombstone semantics, pinned deletion/recovery boundary와 strict public remove
command/confirmation UI도 하나의 닫힌 수직 slice로 구현했다. Model/reasoning selector도 별도의 닫힌
Phase 2 slice로 구현했으며 Git write는 Phase 3 전까지 열지 않는다.

Model/reasoning selector는 header에서 새 작업과 열린 thread 모두에 접근할 수 있고, catalog가
실패해도 composer와 task 생성은 Codex default(null override)로 계속 사용할 수 있다. Model 변경은
새 model이 현재 effort를 지원하면 보존하고, 아니면 advertised default로 전환한다. 열린 thread의
model/effort가 unavailable 또는 effort-unknown이면 현재 값을 그대로 표시하되 사용자가 visible pair를
명시적으로 선택하기 전에는 override를 보내지 않는다. Recent preference는 binding index v4와
safe-removal journal에 포함하지 않는 installation-global UX 데이터이며, 매 catalog read에서 현재
visible pair와 다시 대조한다.

Phase 2의 PTY, 검색/이름 변경, archive/unarchive, fork 수직 슬라이스는 완료했다. 검색은 native가
exact scope의 persisted binding부터 조회하고 `thread/read`로 검증한 결과만 frontend에서
필터링한다. 이름 변경은 frontend cwd나 argv를 받지 않고 exact binding/root를 다시 검증한
뒤 `thread/name/set`과 authoritative `thread/read`를 연속 수행한다.

Archive/unarchive는 binding persistence의
`active | archiving | archived | unarchiving | unknown` lifecycle gate와 delivery-aware
operation journal을 사용하며, resume/turn start/PTY open과 native serialization 경계를
공유한다. archive는 binding, managed worktree, dirty file을 삭제하지 않으며 stable
`active`에서만 실행과 새 PTY를 허용한다. `archiving`, `archived`, `unarchiving`, `unknown`은
crash/reload와 in-flight 동안 모두 fail closed하고, stable store state와 app-server
active/archived membership이 drift하면 먼저 `unknown`으로 durable 전환한다. App-wide
authority latch와 generation/exact/global-graph dirty revision이 reconciliation 및 notification
race 동안 모든 active-only native gate를 닫는다. `turn/start` 성공은 delayed notification을
기다리지 않고 native active-turn gate에 반영하며 archive는 frontend replay가 아니라 이
authority와 exact pending/reserved approval을 RPC 전에 검사한다.

Pinned archive가 spawned descendant까지 연쇄 archive할 수 있으므로 native는 authoritative
thread graph에서 target의 descendant 부재를 증명한 leaf thread만 허용한다. 증명은 cwd
filter 없이 active와 archived 양쪽을 audited 0.145/0.149의 모든
`ThreadSourceKind`(`cli`, `vscode`, `exec`, `appServer`, `subAgent`, `subAgentReview`,
`subAgentCompact`, `subAgentThreadSpawn`, `subAgentOther`, `unknown`)로 cursor 끝까지 조회하고
`parentThreadId`/`forkedFromId` ancestry를 검사해야 한다. default interactive-source 또는
`appServer`-only 조회를 재사용하지 않는다. bound/unbound 또는 scope가 다른 descendant,
unknown/new source, incomplete pagination, duplicate/conflicting ancestry가 있으면 RPC 전에
fail closed하며, exact single-binding command가 암묵적인 cascade 권한을 갖지 않는다.
Archive RPC 전에는 exact PTY owner를 terminate/drain/reap하고 SessionEnd hook 경계를 위해
persisted execution root를 다시 revalidate한다. Definitely-not-sent만 exact journal snapshot으로
rollback하며 response loss, conflicting notification, stable commit failure는 `unknown`으로 닫는다.

Fork도 부모의 managed root를 공유하지 않는다. lifecycle-clean stable `active` managed binding,
idle thread, clean source worktree만 허용하고 source의 current immutable HEAD에서 detached clean
destination을 먼저 만든다. Public input은 exact `{scope, threadId}`뿐이며 audited 0.145/0.149 request는
`threadId`, native destination `cwd`, `approvalPolicy`, `sandbox`, preparation-derived
`threadSource` 다섯 필드만 사용한다. 첫 수직 슬라이스는 `lastTurnId`를 노출하지 않고 전체
persisted history를 복사한다.

Preparation v4 journal은 `operation: start | fork`, fork의 exact `sourceThreadId`와 native-only optional
direct-local `mergeTargetRef`를 저장한다.
응답의 new ID, `sessionId`, `forkedFromId`, top-level/thread destination cwd, explicit
non-ephemeral flag, app-server source와 preparation marker를 모두 검증하고 source lifecycle/activity
proof를 다시 확인한 뒤에만 새 binding을 atomic commit한다. Definitely-not-sent만 같은 preparation과
destination을 `prepared`로 rollback하고, byte admission 이후 response loss나 4 MiB line cap 초과는
`starting`으로 sticky하게 남긴다. Recovery는 재-fork하지 않는다. Active/archived
`thread/list`와 `thread/loaded/list`를 bounded pagination한 뒤, list에 없고 loaded-only인
ID는 exact bound/deferred target 또는 pending fork expectation일 때만
`thread/read(includeTurns:false)`로 hydrate한다. 이 read의 ID, SchoolX marker, ancestry,
canonical root, quiescence와 empty-turn 조건을 검증해 한 child만 bind한다. 실제 0.149
recovery wire에서 관측한 `vscode`와 schema/0.145 spelling인 `appServer`는 이 recovery
경로에서만 동등하게 admit하며, unrelated source나 marker 없는 row는 fail closed한다.
이를 모든 app-server flow의 source가 동등하다는 주장으로 확장하지 않는다.
Source의 dirty patch 복사와 worktree 자동 삭제는 Git handoff 전에는 지원하지 않는다. Codex
app-server가 실행하는 command와 사용자 OS shell PTY는 계속 별도 process/session authority로
유지한다.

Read-only worktree inventory의 public input은 exact `{scope}`뿐이다. Native는 binding store v4를
한 번 읽어 그 scope의 managed binding+lifecycle과 unfinished managed start/fork preparation만
row authority로 채택한다. Local checkout, `WORKTREES` directory orphan, 임의 linked worktree,
frontend path/descriptor는 결과에 넣지 않는다. Read-only store open은 exact app-data
`code/thread-bindings.json` 경계만 읽고 directory 생성이나 permission repair를 하지 않는다. Unix에서는
`code` directory `0700`, index `0600`과 owner를 검사해 다르면 fail closed한다.

각 persisted root는 독립적으로 containment, repository identity, immutable base, HEAD, detached
branch, dirty 상태를 읽고 invalid root는 sibling을 숨기지 않는 tagged `unavailable` row가 된다. 전체
list의 Git 검사는 row별 timeout이 아니라 공유 30초 deadline 안에서 수행하며, 이후 budget exhaustion도
row-local unavailable 상태다. Native inspection error는 UTF-8 경계를 지키며 row당 512 bytes로 제한한다.

모든 row는 native-derived `preserved: true`다. Active binding, transition/Unknown lifecycle, unfinished
preparation, unavailable/dirty/branch-attached root와 merge proof 부재를 closed blocker로 반환한다. Stable
Archived binding만 persisted direct-local merge authority를 사용한 bounded native ancestry proof를 시도한다.
Positive proof는 committed `headDrift`를 제거 관점에서 해소하고 `mergeProofUnavailable`를 추가하지 않으며,
다른 blocker까지 모두 비어 있을 때만 `canRemove: true`다. Not-merged/unavailable proof, proof 뒤
binding/lifecycle/authority/removal join drift는 `mergeProofUnavailable`로 닫힌다.

List는 lifecycle reconciliation을 호출하지 않고 binding index, Git admin metadata, managed root 내용을
바꾸지 않는다. Binding snapshot 뒤 Git read 동안 app-wide binding lock을 유지하지 않는다. 따라서 외부
process가 pathname/Git state를 동시에 바꾸는 multi-process inspection은 정보성 residual이며
`canRemove: true` 자체도 deletion authority나 receipt가 아니다. Public command는 admission부터 proof와
pinned physical identity를 다시 검증한다. UI는 task sidebar의 `Managed worktrees` section에서 eligible
Archived row에만 explicit remove action을 표시한다.

#### Worktree removal decision gates — authority/proof+journal+physical removal implemented, public surface open

Public safe-remove가 지키는 네 gate의 normative 계약은
[`SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md`](SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md), machine-readable
mirror는 `desktop/src-tauri/src/code_workspace/fixtures/worktree-removal-gates-v1.json`에 고정했다.
Binding store v4는 public 8-field binding을 유지한 채 native-only `mergeTargets`, preparation
`mergeTargetRef`와 strict removal records를 가진다. `mergeTargets` record는 exact `{communityId,
projectDtag, repositoryIdentity,
codexThreadId, worktreeId, targetRef}`이고 binding과 0:1 join하며, preparation authority는 public
preparation/inventory projection 전에 scrub한다. Fixture는 version 1과 기존 `futureSurface`/
`futureReceipt` key 이름을 유지하되 status를
`authorityProofJournalPhysicalRemovalImplementedPublicSurfaceOpen`으로 고정한다.

첫 safe-remove의 public input은 exact `{scope, threadId}`다. Tauri는 top-level `{input}`만 받는다. Path,
descriptor, worktree ID,
base/HEAD/target OID, ref, lifecycle, blocker, force/request/removal claim은 caller가 보낼 수 없다.
Native가 preparation 때 선택된 base를 같은 common-dir의 direct local `refs/heads/*`로 유일하게
resolve한 경우에만 v4 binding의 optional merged-target authority로 저장한다. Fork는 source
authority만 복사하고 V1/V2/V3 row는 `baseRef`에서 target을 추론하지 않는다. Proof는 stable Archived
binding의 exact detached HEAD `H`가 authorized ref의 stable resolved commit `T`의 ancestor인지 hardened
`merge-base --is-ancestor H T` exit 0으로 확인한다. HEAD/ref/common-dir/root를 proof 전후 재검증하고
replacement/graft/lazy fetch/network, squash/cherry-pick equivalence와 다른 ref의 reachability는 인정하지
않는다. Capture와 bounded typed proof는 native-only로 구현했으며 public proof command는 없다.
Inventory positive proof는 committed `headDrift`를 해소하고 blocker가 모두 사라진 stable Archived row만
`canRemove: true`로 만든다. Public removal admission은 이 read-only projection을 authority로 재사용하지
않고 exact store/Git/physical state를 다시 증명한다.

V4의 required `removals` namespace는 archive lifecycle이나 start/fork preparation을 재사용하지 않는
strict `claimed -> removing -> removed` journal을 구현한다. Flat tagged record는 native-issued canonical
`removalId`, 원래 8-field binding, literal Archived claim, exact merge proof, SHA-256 manifest digest,
managed-root/quarantine/Git-admin coordinates와 literal transcript/execution disposition을 보존한다.
Raw v4 removal probe는 `Value` decode 전에 duplicate removal/tag/authority member를 거부하며 legacy
migration은 그대로 둔다. Merge proof의 head/target OID 길이는 original binding base OID와 일치해야 한다.
`claimed`는 deletion mutation 0회인 durable proof, `removing`은 최초 Git/filesystem mutation 전에 sync되는
sticky recovery state, `removed`는 verified absence 뒤 live binding+lifecycle+merge-target을 permanent
transcript tombstone으로 한 atomic store write에서 retire한 state다. Exact cancellation, pre/post-save
response loss와 stale CAS/ABA, final-save retry가 pure-store fault injection으로 고정됐다. `(scope,
threadId)` retry는 native-issued 동일 `removalId`로 수렴하며 새 proof/target을 받지 않는다. 모든 상태는
thread/worktree/root identity를 영구 예약하고 binding/preparation/fork/recovery admission에서 재사용을
막는다. Tombstone은 원래 binding과 `transcriptDisposition: preserved`, `executionDisposition: removed`
receipt coordinate를 보존하되 resume/turn/PTY/Changes/rename/unarchive/fork authority가 아니다. Codex
transcript/thread 자체는 변경하거나 삭제하지 않는다.

Linux/macOS pinned engine은 physical absence를 만들고 검증한다. Pinned inspector가 exact root와
reciprocal Git-admin identity를 두 번 확인하고, root/admin entry identity와 content digest, sibling snapshot을
담은 canonical strict v1 manifest를 digest-addressed sidecar로 atomic sync한다. Manifest-derived claim input은
removal module 밖에서 만들 수 없고 final store primitive는 inspector만 발급하는 opaque single-use
verified-absence capability를 소비한다. `code_worktree_remove`는 exact public coordinate와 app-owned
runtime/PTY/lifecycle/shutdown context만 이 sealed engine에 전달하며 public proof command는 계속 없다.

Sidecar directory/file은 code data directory에서 same-mount handle-relative로 접근하고 persist/load/remove의
named directory/file identity를 재검증한다. Replacement나 absent fast-path ambiguity를 cleanup success로
해석하지 않는다.

Startup은 binding mutex 아래 pending removal과 removed tombstone proof-ref cleanup을 emitter/runtime start 및
lifecycle/start/fork reconciliation보다 먼저 처리한다. Archived rename과 raw-binding turn interrupt도
`claimed/removing` 동안 같은 mutex 아래 RPC 전에 fail closed한다. 새 private claim은 exact thread idle과
PTY-owner absence를 증명하고 binding/runtime/activity/approval locks를 끝까지 보유한 sealed activity
clearance만 받는다. Linux/macOS 외 platform은 pending journal이 있으면 zero-mutation `unsupported`로 닫힌다.

Deletion은 inventory의 `dirty:false`나 현재 target-only pinned Git helper를 재사용하지 않는다.
Ignored file과 untracked empty directory까지 보는 no-follow physical manifest가 `.git`과 tracked
entry/ancestor 외 모든 ignored/untracked/special/cross-device/nested entry를 거부한다. Git-admin의
`locked` marker나 `*.lock`도 삭제 authority로 채택하지 않고 claim 전에 거부한다. Journal sync 뒤
parent-handle의 exact UUID name을 deterministic quarantine으로 atomic no-replace rename하고, frozen
manifest만 handle-relative no-follow 삭제한 다음 reciprocal proof를 가진 exact Git-admin entry를
제거한다. Manifest identity는 birth-time/generation을 포함하고, HEAD blob local existence와 no-follow
primary object storage를 증명하며 alternate/shared ODB와 non-files ref backend를 거부한다. `removing` 뒤 첫 Git mutation은 `refs/schoolx/removal-claims/<removalId>`가 exact
`targetCommit`을 가리키는 compare-create이며, tombstone finalization 뒤 같은 OID로 exact
compare-and-delete하고 ref directories를 sync한다. 그 absence가 durable해진 뒤 digest sidecar를 sync unlink한다.
Proof ref는 symbolic/ambiguous raw authority를 거부하고 `--no-deref`로 갱신하며, exact loose regular file을
no-follow로 확인한다. Git reference fsync와 ref file/parent directory fsync를 모두 완료해야 durable하다.
Cleanup crash는 tombstone coordinate로만 재시도하고 ref replacement는 보존한다.
Original common-dir가 offline/이동 상태면 sidecar cleanup marker를 보존하고 exact coordinate가 돌아올 때까지
cleanup을 defer하되 runtime startup은 계속한다.
Original/quarantine/admin replacement는 삭제하지 않고 sticky recovery로 닫는다. 동등한 platform
boundary가 없으면 zero-mutation unsupported다. `--force`, `git clean/reset`,
`git worktree remove/prune`, broad `remove_dir_all`, implicit orphan/archive/fork cleanup은 금지한다.

Public 성공값은 native-derived exact 9-field receipt
`{removalId, scope, threadId, worktreeId, headCommit, mergedIntoRef, mergedIntoCommit,
transcriptDisposition, executionDisposition}`다. Disposition literal은 각각 `preserved`, `removed`이고 같은
`(scope, threadId)` retry와 native commit 뒤 response loss는 동일 tombstone receipt로 수렴한다.

Frontend는 `canRemove: true`인 Archived row에서만 destructive confirmation을 연다. Cancel은 command 0회이며,
confirm 직전에 exact attempt coordinate를 scope별 cache에 보존하고 row를 optimistic하게 제거하지 않는다.
Receipt 없는 outcome-unknown 상태는 sidebar unmount/remount와 target row absence 뒤에도 남는다. 사용자가
destructive retry를 다시 확인하면 같은 public input으로 removal을 완료하거나 tombstone receipt를 회수한다.
Receipt 뒤에는 exact-scope inventory와 thread list를 QueryClient dedupe 밖에서 새로 읽고, 두 결과 모두 target
absence를 보일 때만 두 cache를 교체한다. Reconciliation이 실패하면 committed receipt를 scope별 cache에
보존하고 다른 removal을 막은 채 authoritative list read만 retry한다. UI와 live announcement는 transcript가
보존됐음을 명시한다.

### Phase 3 — Git handoff와 Talk 공유

- stage/unstage/commit UI
- branch/push/PR 기존 기능과 연결
- inline diff comment를 다음 turn context로 전달
- 선택적 Talk 공유
- review/start 기반 코드 검토

첫 독립 slice는 stable Active managed detached worktree의 whole-file stage/unstage와 staged-only
commit으로 제한한다. 별도 native status가 task/staged/unstaged를 한 snapshot에서 읽고 opaque
generation/snapshot/file coordinate를 발급한다. Caller path/ref/OID/argv/identity는 받지 않으며 candidate
index publish, 별도 Git-operation journal과 detached `HEAD` CAS로 response loss를 수렴시킨다. Local/Archived
write, hunk/stage-all, branch/push/PR과 Talk 공유는 후속 slice다. 상세 착수 계약은
[`SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md`](SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md)에 있고,
현재 완료 구현과 명시적 후속 hardening 경계는
[`SESSION_HANDOFF_20260821_CODE_PHASE3_GIT_WRITE_IMPLEMENTATION.md`](SESSION_HANDOFF_20260821_CODE_PHASE3_GIT_WRITE_IMPLEMENTATION.md)에 있다.
다음 독립 slice는 아직 승인되지 않았다. 2026-08-25 후보 비교와 권장
`inline diff note -> 다음 idle turn context`의 decision gate, public contract, fault matrix와
UI/E2E 완료 기준은
[`SESSION_HANDOFF_20260825_CODEX_0_149_AND_NEXT_SLICE_DECISION.md`](SESSION_HANDOFF_20260825_CODEX_0_149_AND_NEXT_SLICE_DECISION.md)에
있으며 사용자 승인 전에는 구현하지 않는다.

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
- worktree removal gate fixture와 exact `code_worktree_remove` input/9-field receipt registration sentinel
- direct-local-ref ancestry proof의 merge/unmerged/squash/drift/graft/timeout/zero-mutation
- `claimed/removing/removed` journal join, crash/retry/final-save와 permanent transcript tombstone
- ignored/untracked/empty/special entry 거부, quarantine/admin replacement와 sibling 보존
- exact-scope managed-worktree inventory authority와 invalid-row 국소화
- inventory 전후 binding index/Git admin/worktree recursive snapshot 불변
- read-only store exact-path/private-permission 거부와 shared deadline/error bound
- stderr/auth/env redaction
- actual Git의 added/modified/deleted/type-changed/unmerged/untracked와 binary/empty file
- Changes 전체 manifest cap, strict patch parser, transient/repeated inventory drift
- Git write strict durable phase/evidence, owned artifact/standard lock provenance와 startup cross-preflight
- stage/commit durable-boundary subprocess crash matrix, unstage parity crash smoke, response-loss, foreign
  replacement와 exact receipt 수렴
- runtime/approval/PTY/fork/archive/remove 대 Git write actual command admission XOR와 zero mutation

### 13.2 TypeScript

- notification delta reducer
- duplicate/out-of-order sequence 처리
- active turn과 approval card 연결
- reconnect snapshot + live event merge
- community/project/thread storage key 격리
- keyboard/focus state
- Changes status enum, unique path, binary count, subset totals와 truncation strict contract
- managed-only inventory descriptor, tagged inspection, proof-based eligible-only `canRemove`와 preservation literal
- removal decision fixture exactness, caller path/ref/OID/proof/removal ID 거부, strict remove adapter와 receipt 검증
- Git status/action strict schema, QueryClient attempt persistence, late revision/generation/receipt 거부
- pending/recovering bounded poll, mutation/ack response-loss reconcile와 composer shared blocker

### 13.3 Desktop E2E

mock Tauri bridge에 app-server fixture event를 넣고 다음 화면을 검증한다.

- empty/new task
- running plan/command/file change
- approval pending/accepted/declined
- interrupted turn
- runtime missing/crashed/recovered
- partial Changes list/status/binary/truncation
- replay synchronization 전 Changes read 차단
- authoritative truncated checkpoint 뒤 stale Changes exact one-shot refresh
- light/dark, 800×500 minimum, wide split layout
- keyboard-only, VoiceOver label, reduced motion
- active/archived/start/fork inventory row, unavailable sibling 보존, eligible Archived row에만 remove action
- explicit confirmation cancel, no optimistic row removal, concurrent same-input convergence와 response-loss retry
- receipt 없는 attempt와 committed receipt의 sidebar remount 보존, target row absence recovery, inventory+thread
  authoritative reconciliation과 peer row 보존
- force/clean/reset/worktree remove/prune/path/ref/OID/proof caller authority 부재
- whole-file stage receipt/ack/clear와 staged-only commit response-loss/reconcile/ack/clear, inspector remount와
  composer blocker

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
- 프로젝트 카드/목록과 상세에 Code workspace 진입점
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
