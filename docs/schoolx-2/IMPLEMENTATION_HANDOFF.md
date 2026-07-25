# SchoolX 2.0 구현 세션 운영

이 문서는 SchoolX 2.0을 여러 세션과 추론 수준으로 개발할 때 현재 사실,
선행 의존성, 작업 경계를 유지하기 위한 handoff다. 제품 범위와 완료 기준은
[`DEVELOPMENT_PLAN.md`](DEVELOPMENT_PLAN.md), 재현 가능한 검증 상태는
[`BASELINE.md`](BASELINE.md)를 기준으로 한다.

## 기본 원칙

- 구조, 권한, 프로토콜, 배포 설정을 바꾸는 작업은 높은 추론으로 수행한다.
- 이미 확정된 패턴을 여러 파일에 반복 적용하는 작업만 낮은 추론으로 전환한다.
- 낮은 추론 작업은 파일 범위, 변경 금지 사항, 완료 조건, 실행할 검증 명령이 모두 정해진 경우에만 시작한다.
- 각 세션은 하나의 구조적 관심사만 다루고, 통과한 테스트와 남은 위험을 이 문서와 `BASELINE.md`에 남긴다.
- 낮은 추론 결과는 구조를 정한 고추론 세션에서 통합 검토한다.
- 문서의 단계 완료는 구현 파일 존재가 아니라 해당 단계의 acceptance criteria와 증거가 모두 있을 때만 표시한다.

## 현재 구현 snapshot

- 브랜치: `codex/schoolx-2-foundation`
- foundation commit: `a3e1ca4fb5f199b8ade62e14c120967ddab190d9`
- parent Buzz snapshot: `acfbb1bb6af54cb29cb152496ff43b8285dcb8cf`
- 현재 상태: foundation commit 위의 검토 작업트리이며 아직 commit하지 않음
- Phase 상태: Phase 0 미완료, Phase 1 서버 기반 일부 구현, Phase 2 i18n 구조 기반 구현

### 구현되어 있는 것

- `i18next`, `react-i18next` runtime과 React provider
- 한국어·영어 catalog와 영어 fallback
- 저장된 `ko`/`en` → 지원되는 OS 언어 → 한국어 순서의 초기 locale
- 실제 `app`, `settings`, `appearance` namespace와 typed translation key
- HTML `lang` 동기화
- 접근 가능한 설정 화면 언어 선택, 즉시 전환과 새로고침 후 저장값 유지
- 앱 로딩 상태와 설정 화면 일부 번역
- 한국어 시스템 글꼴 fallback
- locale 정규화, localStorage getter/write 실패와 한·영 catalog 구조 단위 테스트
- 공용 날짜·숫자·상대시간 formatter와 locale별 캐시
- 검색 결과 날짜와 `AnimatedCount` 숫자의 runtime locale 반영
- mock bridge 전에 init script를 등록하는 fresh-install·양방향 전환 E2E
- NIP-OA 에이전트의 private/open channel-scoped 접근에 active membership을
  강제하는 WS·HTTP·fan-out·Huddle 기반
- persistent `agent_owner_pubkey` 분류와 member-only channel cache
- derived-content source/target audience 부분집합을 검사하는
  `buzz-core::audience` 정책 primitive
- 템플릿 재적용 시 같은 persona 관리형 에이전트를 재사용하고 부분 실패를
  사용자에게 경고하는 기반

### 아직 구현 또는 검증되지 않은 것

- 변경 전 parent snapshot에서 같은 명령을 실행한 재현 가능한 기준선
- 전체 도구 경로를 실제 relay로 검증한 open/private 접근 E2E matrix
- 제품 설정 계층, SchoolX bundle ID, 데이터 디렉터리, 딥링크 호환성
- 앱 셸, 로그인, 온보딩, 채널 목록, 메시지, 검색 등 핵심 화면 전체 번역
- 아직 남은 화면의 하드코딩된 `en-US`와 영어 상대시간 제거
- 한글 IME 조합, 멘션, 자동완성, 검색 회귀 테스트
- 한국어 key 누락 시 영어 fallback E2E
- versioned workspace catalog, provenance, idempotent saga
- SchoolX persona, 관리형 에이전트, coordinator
- audience 정책 primitive를 실제 요약 생성·게시 seam에 연결하고 게시 직전
  membership을 다시 읽는 end-to-end enforcement
- AI draft→verified canvas 운영 흐름
- WF-08 approval과 자동 게시
- 패키징과 파일럿

현재 작업트리는 `a3e1ca4f` E2E의 localStorage seed 순서를 고쳐
`page.addInitScript`를 `installMockBridge(page)`보다 먼저 등록한다.

## 반드시 유지할 보안 사실

다음은 구현자가 임의로 단순화하면 안 되는 현재 SchoolX foundation
작업트리의 의미다.

1. `open` 채널은 멤버가 아닌 인증 사용자도 읽고 쓸 수 있다.
2. 검증되거나 영속 분류된 NIP-OA 에이전트는 `open`이어도 active
   membership이 있는 채널만 읽고 쓸 수 있다.
3. 일반 NIP-42 인증의 `channel_ids`는 제한되지 않으므로 서버의 principal
   분류와 membership 검사를 우회 경로마다 유지해야 한다.
4. ACP channel allowlist는 자동 구독 범위이며 동일 자격증명의 직접 Buzz
   CLI, HTTP, WebSocket 사용을 차단하지 않는다.
5. 제한이 필요한 SchoolX 채널은 1차 버전에서 `private`가 기본이다.
6. 출처 링크는 접근 제어가 아니며, 여러 source의 허용 audience는 멤버
   집합의 교집합이다. 현재 구현은 policy primitive까지이며 게시 seam 연결은
   아직 없다.
7. private source 내용을 다른 채널에 영구 게시하는 것은 공개 범위 변경이다.
8. WF-08은 현재 미구현이며 approval step에 도달한 workflow는 실패한다.
9. 현재 Huddle은 음성 기능이며 카메라 영상과 화면 공유는 없다.
10. Blossom media가 채널 ACL과 결합됐다고 가정하지 않는다.

현재 작업트리 기준 근거 경로:

- 사람/open read 범위와 agent member-only 범위:
  `crates/buzz-db/src/channel.rs`의 `get_accessible_channel_ids`,
  `get_member_channel_ids`
- channel write 강제: `crates/buzz-relay/src/handlers/ingest.rs`의
  `check_channel_membership`
- agent 분류: `crates/buzz-relay/src/api/mod.rs`의 `resolve_agent_owner`
- WS/HTTP read 및 live fan-out:
  `crates/buzz-relay/src/handlers/{req,count,event}.rs`,
  `crates/buzz-relay/src/api/bridge.rs`
- 일반 인증의 unrestricted channel IDs: `crates/buzz-auth/src/lib.rs`의 `AuthContext`
- source audience primitive: `crates/buzz-core/src/audience.rs`
- approval 미구현 실패 처리: `crates/buzz-workflow/src/lib.rs`와 `executor.rs`
- audio-only capture/relay: `desktop/src/features/huddle/HuddleContext.tsx`, `crates/buzz-relay/src/audio/mod.rs`

이 사실을 바꾸려면 일반 Buzz에도 유효한 별도 설계, 서버 강제, 보안 테스트가
필요하다. 현재 서버 기반도 실제 Postgres·Redis를 사용한 transport matrix가
통과하기 전에는 Phase 1 완료로 표시하지 않는다. 프롬프트나 ACP 구독
설정만으로 보안 완료를 주장하지 않는다.

## 다음 구현 순서

### 세션 0 — 기준선 보완

범위:

- `BASELINE.md` 절차대로 parent와 현재 snapshot에서 같은 명령 실행
- Hermit, Node, pnpm, Rust 버전과 실행 시각 기록
- 기존 실패와 SchoolX 변경 실패 구분
- upstream fetch·merge 절차 확인

완료 조건:

- `BASELINE.md`의 필수 표에서 `미검증` 항목이 없어야 한다.
- 같은 명령과 commit으로 다른 작업자가 결과를 재현할 수 있어야 한다.

### 세션 A — 보안과 호환성 계약

새 고추론 세션으로 시작하며 템플릿과 에이전트보다 먼저 끝낸다.

범위:

- 사람과 NIP-OA 에이전트별 open/private 채널 의미와 threat model
- ACP, Buzz CLI, HTTP bridge, WebSocket REQ/COUNT/EVENT 접근 matrix
- member add/remove와 cache 전후 테스트
- source audience 교집합과 target 부분집합 정책
- private source의 자동 교차 게시 금지
- WF-08 전 manual-review fallback
- 음성 Huddle 범위
- branding/deep-link/data-dir/CLI 경로 호환성 결정을 위한 기술 조사

완료 조건:

- private 채널 비멤버가 어느 도구 경로로도 존재, 제목, 메시지, 검색 결과를 얻거나 쓰지 못한다.
- 허용 audience보다 넓은 target에 private-source 요약 본문이 생성되지 않는다.
- 이후 세션이 사용할 machine-readable policy와 E2E matrix가 확정돼 있다.

### 세션 B — 제품 설정과 브랜딩

새 고추론 세션으로 시작한다.

범위:

- 표시 이름, 아이콘, bundle identifier
- SchoolX 전용 데이터 디렉터리
- 새 딥링크 생성과 기존 `buzz://` 수신 정책
- `message`, `join`, `connect`, `add-community`, `nostr-bind` 호환성
- OS scheme 등록과 Buzz 동시 설치
- CLI template 경로
- update channel과 서명 주체

완료 조건:

- Buzz와 SchoolX를 설치·업데이트·제거하는 순서별로 상대 제품 데이터를 쓰지 않는다.
- 과거 메시지 링크의 처리 결과와 실패 UX가 테스트로 고정돼 있다.
- 제품 문자열과 프로토콜 식별자를 구분한 설정 계층이 있다.

### 세션 C — i18n foundation 보강과 핵심 화면 번역

foundation 구조 보강은 현재 작업트리에 구현됐다.

- localStorage init script 순서와 저장소 실패 처리
- fresh-install `en-US`→영어, 미지원 locale→한국어, 한·영 양방향 전환 E2E
- typed resource와 실제 namespace
- 날짜·시간·숫자 formatter 계약과 대표 호출부

다음에는 missing-key 영어 fallback E2E와 아직 하드코딩된 formatter를
정리한 뒤 화면별 문자열을 추출한다.

위 패턴과 테스트가 확정된 뒤 화면별 문자열 추출만 낮은 추론으로 나눌 수
있다.

권장 작업 순서:

1. 앱 셸, 로그인, 온보딩
2. 채널 목록, 채널 생성, 멤버 관리
3. 메시지 작성기, 스레드, 검색, 파일
4. 에이전트, persona, team
5. canvas, forum, 알림
6. 음성 Huddle, workflow, 오류와 빈 상태

각 작업은 한 기능 디렉터리만 수정하며 비즈니스 로직, test ID,
Nostr/CLI/protocol 값은 바꾸지 않는다.

### 세션 D — versioned workspace catalog

새 고추론 세션으로 시작하고, 먼저 agent 없는 메인 회의방과 기획방만
구현한다.

범위:

- built-in `catalog_id`, `catalog_version`, stable `item_key`
- fresh-install seed와 catalog upgrade
- relay-visible channel provenance 또는 일반화된 manifest
- 공개 범위 preflight와 open 경고
- channel/canvas/membership 단계의 idempotent saga
- rename, delete, 동명 충돌, 부분 실패, retry, 보상 규칙
- machine-readable result ledger

완료 조건:

- 같은 선택의 두 번째 적용은 변경 없음이다.
- 단계별 fault injection 뒤 retry해도 중복 없이 desired state에 도달한다.
- 사용자 채널과 사용자 template copy를 자동 채택·덮어쓰기·삭제하지 않는다.

스키마와 적용 API가 확정된 뒤 10개 업무방의 이름, 설명, 시작 canvas,
운영 규칙, snapshot test만 낮은 추론으로 확장할 수 있다.

### 세션 E — 에이전트와 지식 승격 운영

새 고추론 세션으로 시작한다.

범위:

- persona와 managed agent의 stable provenance와 재사용
- private 채널 membership과 ACP 구독 설정
- 직접 CLI/HTTP/WebSocket 권한 우회 테스트
- coordinator source scope와 audience 교집합
- 같은 채널의 `AI 초안`과 출처 metadata
- 사용자가 검토·수정해 canvas에 수동 반영하는 흐름
- 비활성 agent 비용, 실패, timeout, 권한 상실 상태

완료 조건:

- agent가 다른 private 채널을 얻거나 쓰지 못한다.
- private-source 본문을 자동 교차 게시하지 않는다.
- agent가 공식 canvas를 자동으로 덮어쓰지 않는다.
- retry 후 managed agent가 중복 생성되지 않는다.

### 세션 F — 음성 Huddle, workflow, 패키징, 파일럿

새 고추론 세션으로 시작한다.

범위:

- 음성 Huddle 한국어 사용자 여정
- 같은 private 채널에서 coordinator를 멘션하는 정기 초안 workflow
- WF-08 전 수동 공유 UX
- WF-08을 구현할 경우 승인·거절·만료·재시작·중복 승인
- 접근, audience, idempotency, locale, 설치 격리 통합 E2E
- 파일럿 threshold와 개인정보 opt-in/보존/삭제

카메라·화면 공유와 자동 승인 게시를 이 세션의 기본 완료 조건으로 넣지
않는다.

## 낮은 추론으로 맡길 수 있는 작업

낮은 추론 작업은 상위 세션이 구조와 test fixture를 확정한 뒤에만 시작한다.

### 화면 문자열 추출

- 지정된 기능 디렉터리의 사용자 노출 영어 문구 추출
- 확정된 namespace와 typed key에 catalog 항목 추가
- `useTranslation()`과 `t()` 적용
- 번역 key parity와 해당 기능 test 갱신
- 버튼 폭과 줄바꿈 같은 단순 UI 보정

### catalog 콘텐츠

- 안정 키가 이미 배정된 업무방 10개의 이름과 설명
- 시작 canvas 콘텐츠
- persona 설명
- 운영 규칙과 workflow 예시
- catalog snapshot test

낮은 추론 작업에서는 schema, visibility 기본값, provenance, saga,
agent reuse, source audience, deep-link 정책을 변경하지 않는다.

## 높은 추론을 유지할 작업

- open/private 채널과 scoped credential 보안 모델
- source audience 교집합과 cross-channel publication
- 제품 설정, bundle ID, 데이터 디렉터리, update, signing
- 딥링크 생성·legacy 수신·OS 등록
- locale precedence, namespace, typed resources의 구조 변경
- workspace catalog versioning과 relay-visible provenance
- idempotent saga, 충돌, 부분 실패, recovery
- managed agent 생성·재사용·직접 CLI 권한
- AI 초안과 검증된 canvas의 운영·상태 경계
- WF-08 approval state machine
- Nostr event 또는 Rust command 변경
- 보안 리뷰, 통합 테스트, 개인정보와 최종 파일럿 gate

## 낮은 추론 모델에 전달할 작업 명세

```text
목표:
  지정된 화면의 사용자 노출 문자열을 확정된 i18n 패턴으로 전환한다.

허용 파일:
  <정확한 파일 또는 디렉터리>

선행조건:
  namespace와 typed key 계약이 통합돼 있고 관련 기반 테스트가 통과한다.

반드시 유지:
  비즈니스 로직, test ID, Nostr/CLI/protocol 값, 영어 fallback

변경 금지:
  공개 범위, 권한, provenance, saga, agent 설정, 딥링크, 제품 식별자

구현 규칙:
  확정된 namespace와 key naming을 따른다.
  번역되지 않은 동적 사용자 콘텐츠는 수정하지 않는다.
  한국어는 존댓말과 간결한 제품 문체를 사용한다.

완료 조건:
  타입 검사 통과
  정적 검사 통과
  해당 기능 테스트 통과
  실제 사용 key 검사 통과
  영어·한국어 key 구조 일치
```

이 명세보다 넓은 판단이 필요해지면 작업을 중단하고 고추론 세션으로
되돌린다.
