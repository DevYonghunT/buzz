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
- 마지막 upstream 동기화: `5e9e40f3` (2026-08-01, merge, upstream `b1b283cd`).
  그 동기화가 드러낸 것은 [`BASELINE.md`](BASELINE.md)의 「2026-08-01
  동기화에서 드러난 것」에 있다
- 현재 상태: 위 작업이 모두 commit·push됨
- Phase 상태: **Phase 0 완료**, Phase 1 계약 고정 완료(요약 audience 연결 제외),
  Phase 2 i18n 구조 기반 + **제품 설정·브랜딩 완료**(아이콘·업데이트·서명 제외),
  **Phase 3 완료** — 완료 기준 7개 전부에 증거가 있다. 넷은 세션 D가,
  나머지 셋은 세션 D2가 채웠다 (아래 세션 D의 판정표)

세션 0(기준선), 세션 A(보안 계약), 세션 B(제품 설정과 브랜딩)는 끝났다.
세션 C는 진행 중이고, 세션 D(워크스페이스 catalog)는 구현과 게이트를 마쳤으나
Phase 3을 완료로 표시하지 못했다. 세션 E1(catalog 적용 권한)은 세션 D가
남긴 보안 구멍 둘을 닫았고 **Phase 3 판정은 바꾸지 않았다** — 그 둘은
완료 기준 7개 중 어디에도 걸려 있지 않았다. 세션 D2가 세션 D의 「넘긴 것」
1–3을 닫아 **Phase 3을 완료로 올렸다.**

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
- 제품 문자열과 프로토콜 식별자를 구분하는 제품 설정 계층(Rust·데스크톱
  프론트엔드·웹 3벌)과 `tauri.conf.json` 일치 가드
- SchoolX 번들 식별자·표시명, `schoolx://` 딥링크, 전용 nest 디렉터리,
  전용 키체인 서비스
- Buzz 데이터를 읽지도 지우지도 않는 초기화·마이그레이션 경로
- SchoolX 전용 SQL 마이그레이션 예약 대역 `9001+`
- 공유 크레이트 `schoolx-catalog`에 컴파일 내장된 읽기 전용 업무방 catalog
  (`meeting`·`planning` 두 항목, 둘 다 `private`)와 `catalog_id`·
  `catalog_version`·안정 `item_key`
- `catalog_version`을 **제외한** 입력으로 도출하는 결정론적 채널 ID
  (catalog 버전이 올라가도 `meeting`은 같은 방이다)
- relay에 등록된 kind 39500 provenance — 예약 대역 `39500–39599`, 채널 스코프
  (`h` 태그) 저장, NIP-33 LWW
- preflight 판정 8종(`create_or_recreate`·`resume`·`no_change`·`deleted`·
  `adopted`·`not_owned`·`conflict`·`retired`)과 판정과 분리된 `renamed` 플래그
- 채널 생성 → owner 게이트 → 시작 캔버스 → owner 확인의 idempotent saga.
  실패해도 보상하지 않고 재시도가 이어서 한다
- 내용이 있는 캔버스를 덮어쓰지 않고 `skipped`로 기록하는 규칙, 그리고
  읽기 자체가 실패하면 쓰지 않는 규칙
- machine-readable result ledger(`outcome` 4종 · `user_action` 3종 ·
  `decision` 8종 · 단계 상태)와 그 wire format을 바이트 단위로 고정하는 golden
- 미리보기·적용·결과·재시도를 하는 설정 화면 카드와 Tauri command 2개
- 내장 catalog를 그대로 출력하는 읽기 전용 `buzz catalog list`
- catalog 적용의 커뮤니티 관리자 게이트 — `preflight`와 `apply` **양쪽**
  진입에서 relay-signed 커뮤니티 역할(kind 13534)을 확인하고, 걸리면 부분
  결과 없이 실패한다
- 채널 **생성자**(`channels.created_by`)에 건 채택 판정 — relay가 kind:39000에
  `created_by`를 싣고, `channel_owner`가 그것을 읽고, `is_owner`가 거기서
  도출된다. 역할 부여로는 판정이 움직이지 않는다
- 채널 결합에 더해 **그 채널의 생성자가 서명한 provenance만** 인정하는
  preflight. 생성자를 특정할 수 없으면 그 채널의 증명서는 전부 버린다
- 클라이언트가 저작한 kind 39000/39001/39002를 ingest에서 거부하는
  relay-only 강제 — 위 두 항목이 이 불변식 위에 서 있다
- 권한 없음을 이유와 함께 설명하는 설정 카드(오픈 릴레이의 명부 부재를
  권한 거부로 읽지 않는 구분 포함)
- ledger에도 실리는 `renamed` — `LedgerItem::name`은 언제나 catalog 표시
  이름이므로 ledger만 읽는 소비자에게는 이 플래그가 현재 이름과의 불일치를
  아는 유일한 단서다
- v2 catalog fixture로 태운 실제 upgrade 경로 — 같은 방을 이어 쓰고 팀이 쓴
  캔버스를 덮지 않는다
- 설정 카드의 Playwright 렌더 증거 5종(항목 목록·적용 결과·`deleted`
  재생성·`not_owned` 경고·게이트 거부)과 그것을 세우는 mock bridge command 2개
- 막힌 항목의 재생성 — 화면이 보여준 세대를 되돌려 보내고 saga가 일치할
  때만 한 칸 올린다. `deleted`는 기본 버튼, `not_owned`는 결과를 먼저 말한
  뒤의 부차 동작이다

### 아직 구현 또는 검증되지 않은 것

- 변경 전 parent snapshot에서 같은 명령을 실행한 재현 가능한 기준선
- 전체 도구 경로를 실제 relay로 검증한 open/private 접근 E2E matrix
- SchoolX 아이콘 자산, 업데이트 서버 endpoint, 배포 서명 주체
- 앱 셸, 로그인, 온보딩, 채널 목록, 메시지, 검색 등 핵심 화면 전체 번역
- 남은 24개 호출부의 하드코딩된 `en-US`와 OS locale 추종 제거
  (`desktop/scripts/check-i18n-formatters.mjs`의 `PENDING_CONVERSION`이 목록)
- 한글 IME 조합, 멘션, 자동완성, 검색 회귀 테스트
- 나머지 8개 업무방의 이름·설명·시작 canvas·운영 규칙 (낮은 추론 가능)
- **온보딩에 자체 호스팅 릴레이로 붙는 경로** — 「Join or create a community」의
  세 갈래 중 둘이 Builderlab 호스팅 로그인으로 가고 나머지는 초대 코드를
  요구하는데, 초대는 owner/admin만 발행할 수 있어 처음 들어가려면 이미 안에
  있어야 한다. 초대 입력창이 릴레이 URL도 받는 우회로가 있으나 어디에도
  안내가 없다. 학교가 자기 릴레이를 돌린다는 전제에서 **파일럿 차단 요인**이다.
  근거와 재현은 [`BASELINE.md`](BASELINE.md)의 「실제 앱에서의 catalog 적용
  확인」
- 앱 전반의 사용자 노출 문구가 아직 "Buzz"다 — `desktop/src` 기준 421줄 /
  120파일. `just schoolx-upstream-check`의 검사 3은 마지막 동기화 이후 **변경된**
  파일만 훑어 이것을 보지 못한다. 한국어 번역과 같은 문자열을 건드리므로 함께
  하는 것이 맞다
- catalog 적용의 CLI 경로 (`buzz catalog list`는 내장 정의만 출력한다)
- 도출 채널 ID 선점 **자체** — 세션 D3이 복구 경로를 열었으나 공격자가 다음
  세대를 미리 차지하는 것은 막지 못한다. 닫으려면 도출식에 비공개 salt가
  필요하고 기존 방 ID가 전부 바뀐다
- `open` 공개 범위 경고의 도달 경로 — 문구와 자리는 있으나 게이트가 상수
  `false`다
- 도출 채널 ID 선점의 **약한 형태** — 강한 형태(선점 채널을 피해자가 자기
  것으로 채택)는 세션 E1에서 닫혔으나, 먼저 차지해 그 항목을 모든 관리자에게
  영구히 막는 것은 그대로 가능하다. `generation`을 올리는 코드 경로가 크레이트
  전체에 없어 복구되지 않는다
- 생성자에게 **위임 실행을 요청**하는 흐름 — `channels.created_by`가 갱신되지
  않으므로 다른 관리자는 재적용해도 `not_owned`에 멈춘다. 소유권 이전으로는
  풀리지 않는 문제이고 UX가 아직 없다
- SchoolX persona, 관리형 에이전트, coordinator, agent provisioning의
  saga·ledger 편입
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
11. workspace provenance(kind 39500)는 **채널 스코프**(`h` 태그)로 저장되어
    private 채널 ACL을 그대로 받는다. 전역 스코프로 바꾸면 비멤버가 private
    채널의 존재를 알게 되므로 1·2와 함께 깨진다. 자동 채택 대신 사용자
    해결을 요구하는 쪽이 안전한 실패다.
12. **채널이 「우리 것인가」는 역할이 아니라 생성자로 답한다.**
    `MemberRole::Owner`는 상위 등급이 남에게 줄 수 있는 값이고 개수 상한도
    없다 — 부여 검사는 「주는 쪽이 elevated인가」만 보고, owner 가드는 전부
    하한("마지막 owner를 못 뺀다")이다. 따라서 역할로 판정하면 도출 ID를
    선점한 사람이 피해자에게 `owner`를 주는 것만으로 판정을 뒤집는다.
    `channels.created_by`는 생성 시 한 번 쓰이고 relay의 어떤 경로도 다시
    쓰지 않아서 그럴 수 없다. `is_owner`를 `channel_owner`와 별도 근거로
    답하게 만들면 두 값이 어긋나는 자리가 생기고, 그 틈이 정확히 막으려던
    것이다.
13. **kind 39000/39001/39002는 relay만 저작한다.** 12번과 provenance 서명자
    검증이 모두 이 불변식 위에 서 있다. 이것이 없으면 아무 구성원이나 남의
    채널 메타데이터·명부를 위조해 자기를 생성자로 적을 수 있다. `open`
    채널은 비멤버도 읽으므로 **읽힘 자체를 방어선으로 세지 않는다** —
    relay의 채널 스코프 ACL은 경계가 아니라 1차 필터다.

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
- kind 39500 채널 스코프 강제: `crates/buzz-relay/src/handlers/ingest.rs`의
  `requires_h_channel_scope`, 비멤버 차단 증거는
  `crates/buzz-test-client/tests/e2e_workspace_catalog.rs`의
  `non_member_cannot_read_provenance`
- 생성자 태그 발행: `crates/buzz-relay/src/handlers/side_effects.rs`의
  `emit_group_discovery_events`와 buzz-admin reconcile (백필 경로를 빠뜨리면
  백필된 채널이 생성자 불명이 된다)
- 생성자 기반 채택 판정: `desktop/src-tauri/src/commands/workspace_catalog.rs`의
  `channel_owner`·`is_owner`, 서명자 검증은
  `crates/schoolx-catalog/src/preflight.rs`
- 관리자 게이트: 같은 파일의 `require_community_admin`·`role_may_apply`,
  역할 출처는 `desktop/src-tauri/src/commands/relay_members.rs`의
  `get_my_relay_membership`
- relay-only 강제: `crates/buzz-core/src/kind.rs`의 `is_relay_only_kind`,
  회귀는 `crates/buzz-test-client/tests/e2e_relay.rs`의
  `test_client_submitted_nip29_member_lists_are_rejected`와
  `test_client_submitted_nip29_group_metadata_and_admins_are_rejected`
- 선점 시나리오 relay 수준 고정:
  `crates/buzz-test-client/tests/e2e_workspace_catalog.rs`의
  `squatted_channel_provenance_is_signed_by_the_squatter`

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

### 세션 A — 보안과 호환성 계약 · **완료 (2026-07-28)**

산출물은 [`SECURITY_CONTRACT.md`](SECURITY_CONTRACT.md)와
`crates/buzz-test-client/tests/e2e_access_matrix.rs`(17개 테스트, 살아있는
relay에서 전부 통과). 실행은 `just test-e2e e2e_access_matrix`.

세션 A에서 확인된 사실 중 다음 세 가지는 이후 세션이 반드시 전제해야 한다.

1. 판정은 `PrincipalClass::requires_explicit_channel_membership()` 한 곳뿐이며
   읽기·쓰기 전 경로가 이것을 참조한다. 새 경로가 이 함수를 거치지 않으면
   계약이 깨진다.
2. 분류는 태그를 빼도 유지된다(영속 `users.agent_owner_pubkey`). 제한 대상이
   스스로 제한을 벗을 수 없다.
3. 분류된 에이전트는 소유자만 채널에 추가할 수 있다(마이그레이션 0025). 단
   이 정책은 **분류 시점에 바인딩**되므로, 에이전트로 쓸 키는 채널에 넣기
   전에 소유자 attestation으로 한 번 접속시킨다.

**세션 A에서 넘긴 것:** source audience 교집합의 실제 게시 경로 연결.
`buzz-core::audience`는 primitive와 단위 테스트까지만 있고 어떤 생성·게시
경로에도 연결돼 있지 않다. 연결 대상인 요약 기능 자체가 아직 없어 검증할
표면이 없으므로 **세션 E3**(지식 승격)에서 기능과 함께 구현·검증한다. 그전까지 "출처 기반
자동 교차 게시"를 제공한다고 표현하지 않는다.

<details>
<summary>원래 계획 (참고)</summary>

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

</details>

### 세션 B — 제품 설정과 브랜딩 · **완료 (2026-07-28)**

산출물은 [`PRODUCT_IDENTITY.md`](PRODUCT_IDENTITY.md)와 세 벌의 제품 설정
계층(`desktop/src-tauri/src/product.rs`,
`desktop/src/shared/product/index.ts`, `web/src/shared/lib/product.ts`).

확정된 값: 번들명 `SchoolX`(UI는 `스쿨엑스`), 식별자
`io.github.schoolx520.app`, 딥링크 `schoolx://`, nest `~/.schoolx`,
키체인 `schoolx-desktop`.

세션 B에서 확인된 사실 중 다음 네 가지는 이후 세션이 전제해야 한다.

1. **번들 식별자를 바꿔도 격리되지 않는 자원이 있다.** nest 디렉터리, Huddle
   모델 캐시, OS 키체인 서비스명은 `$HOME` 경로이거나 상수라 식별자를 따르지
   않는다. 특히 키체인은 Nostr 신원과 모든 관리형 에이전트 키를 담는다. 새로
   추가하는 `$HOME` 경로나 서비스명 상수는 반드시 `product` 계층을 거친다.
2. **제품 설정 계층은 세 벌이고 서로 강제되지 않는다.** 빌드가 모듈 그래프를
   공유하지 않아서다. 웹 클라이언트는 데스크톱이 등록하는 스킴의 링크를
   생성하므로, 웹만 안 바꾸면 초대·연결 버튼이 아무것도 열지 않는다.
3. **`buzz://`는 OS 경계에서만 거부하고 앱 내부 링크 텍스트로는 읽는다.**
   과거 메시지가 담고 있는 링크는 SchoolX 자신의 기록이며, 텍스트 파싱은 OS
   라우팅이 아니다. 생성은 절대 하지 않는다.
4. **SchoolX 전용 SQL 마이그레이션은 `9001+` 대역을 쓴다.** sqlx는 중복 버전을
   컴파일 타임에 거부하지 않아 충돌이 조용히 마이그레이션 하나를 영구
   무효화한다.

**각 세션은 시작 전에 `just schoolx-upstream-preflight` → `just
schoolx-upstream-merge`로 upstream을 받는다.** 절차와 근거는
`.claude/skills/schoolx-upstream-sync/SKILL.md`에 있고, 이 스킬은 모든 세션의
스킬 목록에 자동으로 뜬다.

`schoolx-upstream-check`의 제품 식별자 검사는 사람이 훑어서 놓친 5건을
잡았다 — 전부 컴파일되고 테스트를 통과하던 코드다. 목록은
[`PRODUCT_IDENTITY.md`](PRODUCT_IDENTITY.md) §3.

**세션 B에서 넘긴 것:** 아이콘 자산, 업데이트 서버 endpoint, 배포 서명 주체.
셋 다 코드가 아니라 디자인·계정 결정이라 세션 F(패키징)에서 처리한다.
현재 배포본은 여전히 Buzz 아이콘을 달고 나온다.

<details>
<summary>원래 계획 (참고)</summary>

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

</details>

### 세션 C — i18n foundation 보강과 핵심 화면 번역 · **진행 중**

foundation 구조 보강은 현재 작업트리에 구현됐다.

- localStorage init script 순서와 저장소 실패 처리
- fresh-install `en-US`→영어, 미지원 locale→한국어, 한·영 양방향 전환 E2E
- typed resource와 실제 namespace
- 날짜·시간·숫자 formatter 계약과 대표 호출부

**세션 C에서 끝난 것 (2026-07-28):**

- i18next init 옵션을 `shared/i18n/config.ts`로 분리 — 테스트가 배포되는
  설정을 검사한다. 이전에는 테스트가 자체 옵션을 만들어 써서 `fallbackLng`를
  읽는 테스트가 하나도 없었다.
- 한국어 키 누락 시 영어 fallback: 단위 테스트와 E2E(`__BUZZ_E2E_I18N__`로
  런타임에 catalog에 구멍을 뚫는다)
- `check-i18n-formatters.mjs`를 `pnpm check`에 연결 — locale이 고정되거나
  OS를 따라가는 호출부를 막는다. 남은 24개는 `PENDING_CONVERSION`에 기록
- 메시지 타임라인 날짜·시간 로컬라이즈(`messages/lib/dateFormatters.ts`와
  소비자 7곳), `time` 네임스페이스 신설

**세션 C에서 확인된 사실 중 다음 세 가지는 이후 세션이 전제해야 한다.**

1. **네임스페이스 누락은 fallback으로 구제되지 않는다.** 키 하나가 없으면
   영어로 대체되지만, `ko`에 네임스페이스가 통째로 없으면 원시 키
   `appearance.title`이 화면에 나온다. `{ lng: "en" }`을 명시해도 그렇다.
   `nsSeparator`가 "."이라 i18next가 접두사의 네임스페이스 여부를 *현재
   언어에 로드된 목록*으로 판단하기 때문이다. 막아주는 것은 런타임이 아니라
   `ko satisfies TranslationShape<typeof en>`와 키 parity 테스트뿐이다.
   **네임스페이스를 추가할 때는 `en`, `ko`, `APP_I18N_NAMESPACES`를 한 번에
   바꾼다.**
2. **한국어 day period를 "오전/오후"로 가정하지 않는다.** 현재 CLDR(ICU 78)은
   `ko-KR`에 "AM"/"PM"을 준다. 마커는 ICU 릴리스마다 바뀌는 데이터이고
   webview와 Node가 다를 수 있으므로, 마커를 다루는 코드는 하드코딩하지 말고
   `formatToParts`로 되물어야 한다. 한국어 정확 문자열을 단위 테스트에
   박으면 ICU 버전에 묶인다.
3. **영어 제품 문구는 한국어 추가의 부수효과로 바뀌지 않는다.** 서수
   ("May 19th")와 상대시간 문구는 글자 그대로 보존했고 테스트가 고정한다.
   한국어만 `Intl` 표준형을 쓴다.

**남은 것:** `PENDING_CONVERSION`의 24개 호출부 전환, 한글 IME·멘션·자동완성·
검색 회귀 테스트, 그리고 아래 권장 순서의 화면별 문자열 추출. 타임라인 주변
문구("last reply", "reply/replies" 같은 영어 복수형)는 아직 하드코딩이며
화면 추출 단계에서 처리한다.

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

### 세션 D — versioned workspace catalog · **구현 완료, Phase 3은 미완료 (2026-07-30)**

설계는 [`WORKSPACE_CATALOG.md`](WORKSPACE_CATALOG.md), 구현은 새 공유 크레이트
`crates/schoolx-catalog`와 relay kind 39500 등록 3곳, Tauri command 2개,
설정 카드, 읽기 전용 CLI다. 코드 경로 전체는 `WORKSPACE_CATALOG.md` §12.

게이트는 모두 이 트리에서 직접 돌렸다. `just ci` 구성 레시피 8개 전부 통과,
`just test-e2e e2e_access_matrix` 17 passed, `just test-e2e
e2e_workspace_catalog` 4 passed, `just schoolx-upstream-check` 3/3 통과.
실행 기록은 [`BASELINE.md`](BASELINE.md).

**Phase 3은 세션 D 시점에 완료로 표시하지 않았다.** 완료 기준 7개 중 4개만
증거가 있었다. 나머지 셋은 **세션 D2(2026-08-04)에서 닫혔다** — 아래 표의
「증거」 칸에 그때 더한 것을 함께 적었다. 세션 D 시점의 판정을 보려면 각
행의 「세션 D 시점」 문장을 읽는다.

| # | 완료 기준 | 판정 | 증거 / 부족한 것 |
|---:|---|---|---|
| 1 | 선택한 private 업무방만 생성 | **충족** | `only_selected_items_are_applied` + `builtin_rooms_are_created_private`(fake의 생성 요청 로그를 읽어 항목별 `visibility`·`description`·`channel_type`을 고정) |
| 2 | 두 번째 적용은 변경 없음 | **충족** | `second_apply_changes_nothing` — 전 항목 `unchanged` + 채널 수 동일 + **발행 횟수 동일** |
| 3 | 이름을 바꿔도 추적 | **충족** (D2) | 추적은 `rename_is_a_flag_not_a_decision`·`renamed_complete_item_is_no_change`. §7이 요구한 "ledger에도 표시"는 `LedgerItem::renamed`와 `renamed_survives_into_the_ledger`, golden의 `ops`(`adopted`) 항목이 `true`로 직렬화되는 것까지. **세션 D 시점**: `LedgerItem`에 필드가 없어 부분이었다 |
| 4 | provenance 없는 동명 채널 자동 채택 금지 | **충족** | `name_conflict_blocks_without_touching_anything` — 채널 수뿐 아니라 `created`·`canvases`·`published` 세 로그가 모두 비었음을 단언한다 |
| 5 | 단계 실패 후 재시도 | **충족** | 세 단계 모두 fault injection. 채널 커밋 **전**·**후** 실패, 캔버스 쓰기·읽기 실패, owner 확인 실패, 증명서 발행 실패까지 6종 |
| 6 | upgrade가 사용자 수정본을 덮어쓰지 않음 | **충족** (D2) | 캔버스 보호는 재개·채택 양쪽에 테스트가 있고, 실제 upgrade는 `catalog_v2_over_applied_v1_does_not_touch_the_canvas`가 v2 fixture로 태운다 — 같은 방을 이어 쓰고 팀이 쓴 캔버스가 그대로다. **세션 D 시점**: upgrade를 실제로 돌리는 테스트가 없었다 |
| 7 | 상태가 UI와 machine-readable 결과에 표시 | **충족** (D2) | machine-readable 절은 `ledger_serializes_for_ui_and_cli`가 어휘 전체를 바이트 단위로 고정. UI 절은 `desktop/tests/e2e/workspace-catalog.spec.ts` 3개 — 항목 목록과 이름 변경 배지, 적용 후 `outcome`·`user_action`·캔버스 note·`error`, 게이트 거부와 적용 버튼 숨김. **세션 D 시점**: 카드를 렌더하는 테스트가 하나도 없었다 |

각 부족분을 닫는 방법은 아래 「세션 D에서 넘긴 것」에 적었다.

세션 D에서 확인된 사실 중 다음 여섯 가지는 이후 세션이 반드시 전제해야 한다.

1. **relay는 kind 39000을 DB 컬럼에서만 재구성한다.** 채널 생성
   이벤트(kind 9007)에 실은 임의 태그는 **어디에도 보존되지 않는다**
   (`crates/buzz-relay/src/handlers/side_effects.rs`의
   `emit_group_discovery_events`). 나가는 태그는 `name`, `about`,
   `visibility`, `t`, `topic`, `purpose`, `archived`, `ttl`뿐이다. 채널에
   메타데이터를 붙이려면 **별도 이벤트여야 한다.** provenance가 kind 39500인
   이유가 이것이고, 채널 생성 이벤트에 태그를 실어 해결하려는 시도는 조용히
   아무것도 하지 않는다.
2. **채널 삭제는 soft delete이고 삭제된 채널의 provenance는 읽을 수 없다.**
   `soft_delete_discovery_events`는 kind 39000/39001/39002만 지우므로 39500
   행은 살아남지만, 채널 조회가 전부 `deleted_at IS NULL`로 걸러지기 때문에
   (`crates/buzz-db/src/channel.rs`) 살아남은 증명서에 닿을 수 없다. 따라서
   "증명서는 완료인데 방이 없다"는 대조가 **불가능하고**, 앱 눈에는 "적용한
   적 없음"과 구별되지 않는다. 대신 쓰는 사실은 채널 생성이
   `INSERT ... ON CONFLICT (community_id, id) DO NOTHING`이라 **한 번 쓴 채널
   번호가 영구히 탄다**는 것이다. 삭제 감지는 `duplicate` 거부 + 접근 가능
   목록에 없음의 조합이며, live relay E2E `deleted_channel_id_is_burned`가
   거부 메시지 문자열까지 고정한다.
3. **SchoolX 전용 Nostr kind는 예약 대역 `39500–39599`를 쓴다.** 이유는 SQL
   마이그레이션 `9001+`와 같다 — upstream이 같은 번호를 쓰면 조용히 충돌하고
   충돌은 컴파일 타임에 잡히지 않는다. 39500은 addressable 대역(30000–39999)
   안이라 NIP-33 LWW가 적용되고, 그래서 재시도가 이벤트를 쌓지 않는다.
4. **이번 실행이 직접 만들지 않은 방은 첫 쓰기 전에 소유권을 확인한다.**
   만든 사람이 곧 owner이므로 **직접 만든** 방만 이 게이트를 건너뛴다.
   증명서는 채널 스코프라 owner가 아니라 **멤버**면 읽히므로, 관리자 A가 만든
   방의 미완료 증명서를 보고 멤버일 뿐인 B가 재개를 돌리는 것이 정상 경로다.
   확인이 `false`면 아무것도 쓰지 않고 `not_owned`로 막고, 확인 **자체**가
   실패하면 채택도 차단도 확정하지 않고 채널 단계를 `failed`로 적고 멈춘다.
   순서가 핵심이다 — 캔버스를 먼저 쓰고 나중에 확인하면 확인이 실패한 시점에
   팀의 내용은 이미 사라진 뒤다.
5. **provenance는 채널 스코프라 private 채널의 증명서는 그 채널 멤버만
   읽는다.** 의도한 트레이드오프다. 다른 관리자가 preflight를 돌리면 자기가
   멤버가 아닌 항목은 "이미 적용됨"을 볼 수 없고 `conflict`로 떨어진다.
   전역 스코프로 바꾸면 비멤버가 private 채널의 존재를 알게 되어
   [`SECURITY_CONTRACT.md`](SECURITY_CONTRACT.md)가 깨진다. E2E
   `non_member_cannot_read_provenance`가 이것을 고정한다.
6. **`steps`에 값을 더하는 것은 읽기 쪽 breaking change이고, 실패가
   조용하다.** 데스크톱 어댑터가 파싱 실패한 증명서를 버리므로 그 항목은
   "적용한 적 없음"으로 보이고 saga가 캔버스 단계까지 내려가 팀이 써 둔
   내용을 덮어쓴다. 그래서 **모르는 값을 관용하는 리더를 먼저 릴리스하고**
   (`StepStatus::Unrecognized`), 그 리더가 퍼진 **뒤에** 새 값을 쓰는 라이터를
   낸다. 모르는 값은 **끝난 것**으로 센다 — 미완료로 세면 그 단계를 다시
   실행한다는 뜻이고 캔버스 단계의 재실행이 바로 그 덮어쓰기다.

**세션 D에서 넘긴 것.** 1–3은 Phase 3을 닫기 위해 필요하고, 4–7은 다음
세션의 범위다. 8·9는 남은 구현이 아니라 브랜치 전체 리뷰(2026-07-31)에서
나온 보안 결정이다 — 개별 작업 리뷰로는 보이지 않던 문제였고, 세션
소유자가 세션 D 범위에서 고치는 대신 세션 E로 넘기기로 정했다. 서로 얽혀
있어 한 쌍으로 적는다. **둘 다 세션 E1에서 처리됐다 (2026-08-04)** — 아래
각 항목 끝에 무엇이 닫혔고 무엇이 남았는지 적었다. 8번은 완전히 닫혔고,
9번은 강한 형태만 닫혔다.

1. **설정 카드의 실행 증거** (완료 기준 #7). 카드는 `outcome` 4종,
   `user_action` 3종, `decision` 8종, 캔버스 `skipped`/`unrecognized`를 모두
   구별해 그리지만 이를 렌더하는 테스트가 **하나도 없다**. 데스크톱 단위
   테스트 3,756개 중에도, Playwright 스펙에도 없다. 카드의
   `data-testid`(`settings-workspace-catalog`, `catalog-apply`,
   `catalog-canvas-note-*`, `catalog-user-action-*`, `catalog-error-*`)는
   이미 붙어 있으므로 스펙 하나로 닫힌다.

   **세션 D2에서 닫혔다.** 다만 "스펙 하나"라는 견적은 틀렸다 — mock
   bridge에 `preflight_workspace_catalog`·`apply_workspace_catalog` 핸들러가
   없어서 스펙을 세울 수 있는 상태가 아니었다. 노브 셋(항목·ledger·거부
   문자열)을 더한 뒤 `desktop/tests/e2e/workspace-catalog.spec.ts` 3개로
   닫았다. 거부는 문자열로 reject해야 한다 — 카드가 에러 문자열로 두 거부를
   구별한다.
2. **`catalog_version` upgrade 경로** (완료 기준 #6). 버전은
   `Provenance`와 `Ledger`에 **기록만 되고 어디서도 읽히지 않는다.**
   preflight는 `item_key` 존재와 단계 완료도로만 판정하므로 v2를 v1 위에
   돌려도 v1을 다시 돌리는 것과 동작이 같다. 그 동작이 옳다는 판단은 서 있지만
   **그것을 확인하는 테스트가 없다.** 사용자 template copy
   (`desktop/src-tauri/src/templates/storage.rs`)를 건드리지 않는다는 것도
   구조로만 보장된다 — `CatalogEffects` trait에 파일시스템 능력이 없어서다.
   닫는 방법: catalog v2 fixture로 "이미 적용된 v1 방에 v2를 돌리면 캔버스가
   그대로다"를 고정한다.

   **세션 D2에서 닫혔다** (`catalog_v2_over_applied_v1_does_not_touch_the_canvas`).
   그 테스트가 단독으로 지키는 것은 **채널 동일성**이다 — 도출식 입력에
   버전이 섞이면 upgrade가 "같은 방을 이어 쓴다"에서 "버전마다 새 방"으로
   바뀌는데, 버전이 다를 때만 무는 형태로 주입하면 크레이트 전체에서 이것
   하나만 실패한다. 캔버스 단언은 그렇지 않다: 판정이 `no_change`라 saga가
   캔버스 단계 앞에서 반환하므로, 캔버스 가드를 둘 다 열어도 이 테스트는
   초록으로 남고 대신 기존 여섯 개가 실패한다.
3. **`renamed`의 ledger 노출** (완료 기준 #3). `PreflightItem`에만 있고
   `LedgerItem`에는 없다. ledger만 읽는 소비자는 `LedgerItem::name`이 그 방의
   현재 이름이 아닐 수 있다는 사실을 알 방법이 없다. 이름이 바뀐 방은
   `adopted`로 끝나므로 실제로 도달하는 상태다.

   **세션 D2에서 닫혔다.** `LedgerItem::renamed`와
   `renamed_survives_into_the_ledger`, 그리고 golden의 `ops`(`adopted`)
   항목이 `true`로 직렬화되는 것까지. 카드는 바꾸지 않았다 — 각 행이 이미
   preflight 항목의 `renamed`로 배지를 그리므로 같은 사실을 두 번 그리지
   않는다. 이 필드가 필요한 소비자는 ledger만 읽는 쪽, 즉 아직 없는 CLI
   적용 경로다.
4. **나머지 8개 업무방 콘텐츠** — 이름, 설명, 시작 canvas, 운영 규칙,
   snapshot test. 스키마와 적용 API가 고정됐으므로 **낮은 추론**으로 나눌 수
   있다. `open` 항목을 넣으려면 `visibility`를 `PreflightItem` →
   `CatalogPreflightItem`까지 실어 보내는 작업이 먼저다 (§9 참조).
5. **에이전트 provisioning** — 세션 E2. ledger 스키마에 자리만 두고
   `not_implemented`로 표시하기로 한 부분이다.
6. **CLI 적용 경로.** 이번 CLI는 `buzz catalog list`뿐이고 preflight도 ledger도
   읽지 않는다. `ledger_serializes_for_ui_and_cli`가 고정한 wire format은 그
   경로가 붙을 때를 위한 것이다.
7. **재생성(recreate) 프롬프트가 답할 수 없다.** `deleted` 판정은
   `user_action: confirm_recreate`를 실제로 보고하고 설정 카드가 "이전에
   삭제한 방입니다. 다시 만들까요?"를 문구로 띄우지만, 그 물음에 답할
   컨트롤이 없다 — `generation`을 늘리는 코드 경로가 크레이트 전체에 없어서다
   (preflight·saga·ledger·provenance·channel_id 다섯 곳 모두 이미 정해진
   값을 그대로 나르기만 한다). `deleted`로 떨어진 항목은 이 컨트롤이 생기기
   전까지 재시도해도 같은 판정을 반복할 뿐이다. 자세한 내용과 닫는 방법은
   [`WORKSPACE_CATALOG.md`](WORKSPACE_CATALOG.md) §6의 「구현 상태」 문단을 본다.

   **세션 D3에서 닫혔다 (2026-08-04).** 카드에 버튼이 붙고 적용 인자가
   `Vec<Selection>`이 되어 항목마다 `recreate_from`을 나른다. 세대는
   preflight가 알아내지 않고 **사용자 확인이 나른다** — 탄 세대를 조회할
   방법이 없기 때문이다(§6의 1–3번). 설계는
   [`CATALOG_RECREATE.md`](CATALOG_RECREATE.md).
8. **관리자 게이트 부재.** catalog 적용은 계획서 전반에서 관리자의 동작으로
   서술된다 — [`DEVELOPMENT_PLAN.md`](DEVELOPMENT_PLAN.md) Phase 3의 첫
   완료 기준이 "관리자가 선택한 private 업무방만 생성되고"로 시작한다.
   그러나 이를 강제하는 코드가 없다. 설정 화면의 `workspace-catalog`
   섹션에는 `featureGate`가 없다 — 형제 nav 섹션인 `channel-templates`는
   `featureGate: "channel-templates"`를 갖는다
   (`desktop/src/features/settings/ui/SettingsPanels.tsx`). featureGate
   자체는 권한 검사가 아니라 preview-feature 토글이라 이 부재가 곧 침해는
   아니지만, catalog 카드가 형제 섹션이 따르는 관례에서조차 벗어나 있다는
   신호다. 실질적인 게이트는 Tauri 쪽에도 없다 —
   `preflight_workspace_catalog`·`apply_workspace_catalog`
   (`desktop/src-tauri/src/commands/workspace_catalog.rs`) 모두 서명 키가
   유효한 워크스페이스 멤버라는 것 말고는 호출자에게 아무것도 요구하지
   않는다. 그 결과 인증된 워크스페이스 멤버는 누구나 적용을 돌려 표준
   업무방들의 owner가 될 수 있고, 정작 관리자는 그 방에서 `not_owned`를
   받는다. [`WORKSPACE_CATALOG.md`](WORKSPACE_CATALOG.md) §8의 owner
   게이트는 *이미 있는 방을 함부로 채택하지 못하게*는 막지만, 새로 만드는
   첫 실행이 누구인지는 막지 않는다 — "생성이 곧 owner 권한"이기 때문이다.

   Phase 3 완료 기준 7개 중 이 게이트를 요구하는 항목이 없어 위 판정에는
   영향이 없었다. 세션 소유자가 세션 D 범위에서 고치는 대신 세션 E로
   넘기기로 정했다 — 아래 9번과 서로 얽혀 있어서다. 세션 E가 알아야 할
   것: 이 코드베이스에는 channel-local owner/admin과 구분되는 커뮤니티
   단위 owner/admin이 이미 있다(`relay_members` 테이블,
   `crates/buzz-relay/src/handlers/moderation_authz.rs`의
   `authorize_moderation_action`). "관리자"가 이 커뮤니티 역할을 뜻하는지
   부터 정하는 것이 설계 결정이고, 그 결정 전까지 확정된 사실은 지금 이
   흐름 어디서도 role이 확인되지 않는다는 것뿐이다.

   **세션 E1에서 닫혔다.** 위 설계 결정은 「그렇다」로 정해졌다 — 커뮤니티
   역할이다. `preflight`와 `apply` 양쪽 진입에서 `require_community_admin`이
   relay-signed kind 13534의 역할을 확인하고 `owner`/`admin`만 통과시킨다.
   모르는 역할은 거부한다. `featureGate`도 함께 달았으나 그건 메뉴를 숨길
   뿐 보안이 아니다.
9. **도출된 채널 ID 선점.** §5의 채널 ID를 만드는 네 입력(relay scope,
   catalog id, item key, generation)은 모두 공개다 — 네임스페이스는
   오픈소스 코드의 리터럴이고 `item_key`는 `buzz catalog list`가 그대로
   출력한다. 인증된 사용자라면 누구나 그 UUID를 계산해 관리자가 적용하기
   전에 그 ID로 채널을 먼저 만들어 둘 수 있다.

   브랜치 전체 리뷰(2026-07-31)에서 나온 관련 결함 — 도출된 채널이 아닌
   곳에 실린 증명서를 버리는 결합 검사
   (`record_sits_in_its_derived_channel`,
   [`WORKSPACE_CATALOG.md`](WORKSPACE_CATALOG.md) §7) — 은 이미 고쳐
   커밋됐다. 이건 "아무 채널에나 위조 증명서를 발행해 다른 관리자의
   preflight를 영구히 잠그는" 경로를 막는다. **도출된 ID 자체를 선점하는
   것은 막지 않는다:**

   - **약한 형태.** catalog 이름이 아닌 다른 이름으로 도출된 ID에 채널을
     먼저 만들어 둔다 — 이름이 같으면 `conflict`로 걸린다. 그러면 모든
     관리자의 적용이 `deleted` 또는 `not_owned`로 막히고, `generation`을
     올리는 코드 경로가 크레이트 전체에 없어 **영구히** 복구되지 않는다.
     공격자는 provenance를 발행할 필요도 관리자일 필요도 없다 — 채널
     하나만 먼저 만들면 그 catalog 항목은 그 학교에서 영원히 막힌다.
   - **강한 형태.** 선점한 채널에서 공격자가 피해 관리자에게 `admin`
     role을 준다 — kind 9000 PUT_USER의 제3자 role 부여는 대상의 동의를
     요구하지 않는다(`crates/buzz-relay/src/handlers/side_effects.rs`).
     `is_owner`(`desktop/src-tauri/src/commands/workspace_catalog.rs`)는
     `owner`와 `admin`을 동일하게 취급하므로 판정이 `adopted`로 떨어지고,
     saga가 공격자의 채널을 그대로 채택해 시작 캔버스를 쓰고 성공을
     보고한다. 화면에는 치환의 흔적이 없다 — ledger의 `name`은 항상
     catalog 표시 이름이지 그 채널의 실제 이름이 아니다(§10). 관리자는
     표준 업무방을 성공적으로 만들었다고 믿지만 실제로는 공격자가 만든
     채널이고, 공격자는 그 채널의 최초 owner라 이후에도 접근권을
     유지한다.

   닫으려면 채널 결합만으로는 부족하다 — **provenance의 서명자를 확인해야
   한다.** 지금 결합 검사는 "이 레코드가 도출된 채널에 있는가"만 보고
   "누가 서명했는가"는 보지 않으므로, 선점한 자기 채널 안에서는 공격자도
   결합 검사를 통과하는 레코드를 만들 수 있다
   ([`WORKSPACE_CATALOG.md`](WORKSPACE_CATALOG.md) §7 「남는 구멍」).
   서명자를 무엇과 비교할지는 위 8번의 "관리자가 누구인가" 결정에 달려
   있다. `is_owner`가 `owner`뿐 아니라 `admin`도 통과시키는 것이 강한
   형태를 가능하게 하는 지점이므로, 서명자 확인과 함께 재검토한다.

   **세션 E1에서 강한 형태가 닫혔다. 약한 형태는 남는다.** 재검토 결과
   `admin`만 떼는 것으로는 아무것도 닫히지 않는다는 것이 드러났다 —
   `MemberRole::Owner` 자체가 부여 가능한 값이고 개수 상한도 없어서,
   선점자는 `admin` 대신 `owner`를 주면 그만이었다. 그래서 판정 근거를
   역할이 아니라 불변 생성자(`channels.created_by`)로 옮겼고, provenance는
   그 채널의 생성자가 서명한 것만 인정한다. 선점자가 무슨 역할을 뿌리든
   피해자의 `is_owner`는 거짓이다. 근거와 정정 이력은
   [`CATALOG_SECURITY.md`](CATALOG_SECURITY.md) §5·§6.

   **약한 형태는 설계상 남긴 것이다.** 도출 ID를 먼저 차지해 그 항목을
   모든 관리자에게 영구히 막는 것은 그대로 가능하다 — 안전한 실패(아무것도
   쓰지 않고 멈춘다)이지만 복구 경로가 없다. 닫으려면 `generation` 증가
   경로가 필요하고, 그것은 여전히 미구현이다(위 7번). 같은 성질에서
   나오는 부작용으로 **공동 관리도 막힌다** — A가 만든 방은 B가 재적용해도
   `not_owned`이고, 생성자에게 위임 실행을 요청하는 흐름은 아직 없다.

<details>
<summary>원래 계획 (참고)</summary>

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

</details>

### 세션 D2 — Phase 3 닫기 · **완료 (2026-08-04)**

세션 D 「넘긴 것」 1–3을 닫는 것만이 범위였다. 새 동작은 만들지 않았다 —
셋 다 **이미 옳게 돌던 것을 관측 가능하게** 만드는 작업이었다. 계획은
[`plans/2026-08-04-phase3-closure.md`](plans/2026-08-04-phase3-closure.md),
게이트 실행 기록은 [`BASELINE.md`](BASELINE.md)의 세션 D2 절.

세션 D2에서 확인된 사실 중 다음 셋은 이후 세션이 전제해야 한다.

1. **`Ledger`에 필드를 더하는 것은 `steps`에 값을 더하는 것과 다르다.**
   세션 D 사실 6번의 "조용한 읽기 쪽 breaking change"는 relay에 저장된
   provenance의 `steps` 어휘를 **구버전이 읽다** 파싱에 실패하는 경로였다.
   `Ledger`는 relay에 저장되지 않고 `apply_workspace_catalog`의 반환값으로만
   살아 생산자와 소비자가 같은 빌드 안에 있다. 버전 skew가 없으므로 필드
   추가가 조용한 실패를 만들지 않는다. **`steps` 쪽 규칙은 그대로다.**
2. **`no_change` 판정은 캔버스 단계에 도달하지 않는다.** upgrade 테스트의
   캔버스 단언이 단독 방어선이 아닌 이유이고, 반대로 캔버스 보호를 바꾸는
   작업이 upgrade 경로 테스트로는 검증되지 않는다는 뜻이기도 하다. 캔버스
   가드를 건드리면 `adoption_does_not_overwrite_a_canvas_that_has_content`
   계열 여섯 개를 봐야 한다.
3. **mock bridge에 없는 command는 조용히 실패하지 않고 스펙을 세울 수 없게
   만든다.** 카드가 `data-testid`를 다 갖고 있어도 그것을 그리는 데이터가
   오지 않으면 스펙을 쓸 수 없다. 새 Tauri command를 더하는 세션은 그
   command의 mock 핸들러도 함께 더한다 — 그러지 않으면 그 화면은 Playwright
   범위 밖에 남고, 그 사실은 스펙을 쓰려고 할 때까지 드러나지 않는다.

**세션 D2에서 넘긴 것.** Phase 3은 닫혔지만 catalog 표면에 남은 것이 있다.
전부 완료 기준 7개 밖이다.

1. `generation` 증가 경로 부재 — `deleted` 판정이 재생성을 묻지만 답할
   컨트롤이 없다 (세션 D 「넘긴 것」 7번).
2. 선점의 약한 형태와 위임 실행 요청 흐름 (세션 E1 「넘긴 것」 1·2번).
3. catalog 적용의 CLI 경로 (세션 D 「넘긴 것」 6번).
4. 나머지 8개 업무방 콘텐츠 (세션 D 「넘긴 것」 4번) — 낮은 추론 가능.

### 세션 D3 — catalog 재생성 · **완료 (2026-08-04)**

세션 D 「넘긴 것」 7번과 세션 E1 「넘긴 것」 1번이 같은 메커니즘 하나에
걸려 있어 한 세션으로 처리했다. 새 판정도 새 단계도 만들지 않았다 — 이미
있는 `deleted`·`not_owned`에 **답할 수단**을 준 것이다. 설계는
[`CATALOG_RECREATE.md`](CATALOG_RECREATE.md), 계획은
[`plans/2026-08-04-catalog-recreate.md`](plans/2026-08-04-catalog-recreate.md),
게이트 기록은 [`BASELINE.md`](BASELINE.md)의 세션 D3 절.

세션 D3에서 확인된 사실 중 다음 넷은 이후 세션이 전제해야 한다.

1. **세대를 조회할 방법은 없다.** 삭제된 채널의 증명서는 `deleted_at IS NULL`
   필터에 걸려 읽히지 않고, 선점 채널의 것은 서명자 검증에서 버려진다. 그래서
   「최신 세대를 읽어 +1」은 성립하지 않는다 — 세대를 아는 주체는 **방금 그
   판정을 받은 실행**뿐이고, 그 값이 화면을 거쳐 되돌아온다.
2. **클라이언트가 세대를 올리지 않는다.** 화면은 **본 값 그대로** 보내고
   saga가 일치할 때만 올린다. 그 일치 검사가 낡은 화면·두 번째 클릭·이미
   처리한 다른 관리자를 전부 no-op으로 만드는 유일한 장치다 — 클라이언트가
   미리 +1 하면 그 검사가 무력해지고 누를 때마다 방이 하나씩 생긴다.
3. **`not_owned`는 선점과 정상적인 공동 관리를 구별하지 못한다.** 둘 다
   「생성자가 내가 아니다」뿐이고 이름으로 추측할 수도 없다(선점자가 catalog
   이름을 쓸 수 있고 정상 방의 이름은 팀이 바꿨을 수 있다). 그래서 판정은
   사람이 하도록 두고 — 1차 안내는 `request_ownership`, 재생성은 결과를
   먼저 말한 뒤의 부차 동작 — 코드가 자동으로 고르지 않는다.
4. **재생성은 이전 세대의 단계 상태를 버려야 한다.** `deleted`로 온 항목은
   증명서가 읽히지 않아 어차피 비어 있어서 이 규칙이 눈에 띄지 않지만,
   `resume`이 `not_owned`로 끝난 항목은 **다른 사람의 진행**이 적힌 채로
   온다. 버리지 않으면 saga가 채널 생성을 건너뛰고 있지도 않은 새 세대 방에
   그다음 단계를 건다.

**세션 D3에서 넘긴 것.**

1. **선점 자체는 여전히 열려 있다.** 위 「넘긴 것」 1번 참조 — 영구 차단이
   유한한 경합으로 바뀐 것이다.
2. **잘못 만든 방의 정리.** `not_owned`에서 재생성을 눌러 생긴 여분의 방은
   수동으로 지운다. 자동 정리는 catalog가 사용자 채널을 지우는 경로를 만드는
   일이라 §8의 「자동 채택·덮어쓰기·삭제하지 않는다」와 충돌한다.
3. **위임 실행 요청 흐름.** 「A에게 실행을 부탁한다」를 앱 안에서 보내는 것은
   여전히 별개 기능이고 만들지 않았다.

### 세션 E1 — catalog 적용 권한 · **완료 (2026-08-04)**

세션 D 「넘긴 것」 8·9를 닫는 것만이 범위였다. 새 기능은 없다. 설계는
[`CATALOG_SECURITY.md`](CATALOG_SECURITY.md), 계획은
[`plans/2026-08-01-catalog-security.md`](plans/2026-08-01-catalog-security.md),
구현은 커밋 `0a2ccada`–`14925137`이다.

게이트 실행 기록은 [`BASELINE.md`](BASELINE.md)의 세션 E1 절에 있다.

세션 E1에서 확인된 사실 중 다음 다섯 가지는 이후 세션이 반드시 전제해야
한다.

1. **「이 방이 우리 것인가」를 역할로 묻지 않는다.** 처음 설계는
   `is_owner`에서 `admin`만 떼는 것이었고 근거는 "`owner`는 생성자에게
   고정되어 남이 줄 수 없다"였다. **그 근거가 사실이 아니었다** —
   `MemberRole::Owner`는 평범한 부여 가능 값이고 개수 상한도 없다. 선점자가
   `admin` 대신 `owner`를 주면 비용 없이 같은 공격이 성립한다. 그래서 판정을
   불변 생성자로 옮겼다. **역할로 소유를 답하는 코드를 새로 만들면 이 구멍이
   다시 열린다.**
2. **`is_owner`는 독립 근거를 갖지 않는다.** `channel_owner(id) == Some(나)`로
   도출된다. 둘을 따로 답하게 하면 두 값이 어긋나는 자리가 생기고, 그 틈이
   정확히 막으려던 것이다.
3. **「관리자」는 커뮤니티 역할이다.** `relay_members`의
   `owner`/`admin`이며 근거는 relay-signed kind 13534다. 채널 레벨
   `MemberRole`과 혼동하지 않는다. **preflight도 같은 게이트 뒤에 있다** —
   미리보기만으로도 어떤 항목이 이미 적용됐는지가 드러나고 그것은 private
   채널의 존재 정보다.
4. **오픈 릴레이에서는 적용이 아예 막힌다.** `require_relay_membership`는
   기본값이 `false`이고 그때 relay는 NIP-43을 광고하지도 발행하지도 않는다.
   `.env.example`도 `just test-e2e`도 그 값을 켜지 않으므로 **기본 개발
   릴레이가 그 상태다.** 게이트는 「명부 없음」을 「제한 없음」으로 읽지
   않는다 — 그렇게 읽으면 게이트가 그 릴레이에서 no-op이 된다. 대신
   `catalog-membership-unavailable`로 거부하고, 이것은
   `catalog-admin-required`와 **구별되는** 식별자다. 사용자가 할 일이 다르기
   때문이다(전자는 릴레이 설정 문제, 후자는 관리자에게 요청). catalog 적용을
   실제로 태우는 세션은 이 설정부터 켜야 한다.
5. **이 게이트는 클라이언트 측이다.** relay는 catalog를 모르고, 그렇게 두기로
   했다(§4). 직접 relay에 kind 9007을 쏘아 채널을 만드는 것은 막지 못하며
   막을 대상도 아니다 — 막는 것은 「catalog 적용으로 기본 업무방 일습을
   만드는 것」이다.

**세션 E1에서 넘긴 것.**

1. **선점의 약한 형태.** 도출 ID를 먼저 차지해 그 항목을 막는 것은 그대로
   가능하다. 안전한 실패이지 가용성 보장이 아니다.

   **세션 D3이 복구 경로만 열었다 (2026-08-04).** `generation` 증가 경로가
   생겨 관리자가 다음 세대에 만들 수 있다. **선점 자체는 여전히 막지
   못한다** — 도출식 입력이 전부 공개이므로 공격자가 `g+1`, `g+2`를 미리
   차지할 수 있고, 그러면 누를 때마다 한 칸씩 밀린다. 정확히 말하면
   **영구 차단이 유한한 경합으로 바뀐 것**이고 그 이상으로 표현하지 않는다.
   진짜로 닫으려면 도출식에 비공개 salt가 필요한데 기존 방 ID가 전부 바뀌어
   마이그레이션이 따로 필요하다([`CATALOG_RECREATE.md`](CATALOG_RECREATE.md) §5·§8).
2. **공동 관리와 위임 실행.** `channels.created_by`가 갱신되지 않으므로 A가
   만든 방은 B가 재적용해도 `not_owned`다. `UserAction::RequestOwnership`은
   「소유권을 넘겨받으면 풀린다」가 아니라 「저 사람에게 실행을 부탁하라」로
   읽어야 하고, 그 요청 흐름은 아직 없다. 열거형 이름은 역할 기반 판정
   시절의 잔재다.
3. **Phase 3의 남은 세 항목은 그대로다.** 세션 D 「넘긴 것」 1–3(설정 카드
   실행 증거, `catalog_version` upgrade 경로, `renamed`의 ledger 노출)은
   E1 범위가 아니었고 여전히 열려 있다.

### 세션 E2·E3 — 에이전트와 지식 승격 운영

새 고추론 세션으로 시작한다. E2는 에이전트 프로비저닝, E3는 지식 승격이며
서로 별도 설계다.

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
