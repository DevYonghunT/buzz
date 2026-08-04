# SchoolX 2.0 보안 계약 (세션 A)

이 문서는 SchoolX가 Buzz 위에 얹은 접근 제어 계약을 고정한다. 이후 세션의
템플릿·에이전트 작업은 여기에 적힌 의미를 전제로 하며, 이 의미를 바꾸려면
일반 Buzz 배포에도 유효한 별도 설계와 서버 강제, 보안 테스트가 필요하다.

근거는 실행이다. 모든 항목은
[`crates/buzz-test-client/tests/e2e_access_matrix.rs`](../../crates/buzz-test-client/tests/e2e_access_matrix.rs)의
17개 테스트로 살아있는 relay에서 확인했다. 실행 방법은
`just test-e2e e2e_access_matrix`.

## 1. principal 분류

서버가 판정하며 클라이언트가 주장할 수 없다.

| 분류 | 조건 | 정의 위치 |
|---|---|---|
| `Human` | agent owner가 없음 | `crates/buzz-relay/src/handlers/ingest.rs` `PrincipalClass::Human` |
| `ManagedAgent` | NIP-OA로 소유자가 증명되었거나 영속 분류됨 | 〃 `PrincipalClass::ManagedAgent { owner_pubkey }` |

판정 함수는 하나뿐이다.

```rust
pub const fn requires_explicit_channel_membership(&self) -> bool {
    self.is_managed_agent()
}
```

읽기 경로(`req.rs`, `count.rs`, `api/bridge.rs`)와 쓰기 경로(`ingest.rs`)가
모두 이 하나를 참조한다. 새 경로를 추가할 때 이 함수를 거치지 않으면 계약이
깨진다.

### 분류의 두 층위를 혼동하지 않는다

- **연결 단위** `AuthContext.agent_owner_pubkey` — 그 연결에 제시된 태그로만 설정.
- **영속** `users.agent_owner_pubkey` — community 범위, first-write-wins, 영구.

태그 없이 재접속해도 DB 기반 검사는 여전히 에이전트로 취급한다. 제한 대상이
스스로 제한을 벗을 수 없어야 하므로 이 비대칭은 의도된 것이다.
(`agent_cannot_shed_its_class_by_dropping_the_tag`)

## 2. 접근 매트릭스

`R` = 읽기 허용, `W` = 쓰기 허용, `—` = 거부.

| principal | open 채널 | private 채널 |
|---|---|---|
| 사람 · 멤버 | R W | R W |
| 사람 · 비멤버 | **R W** | — |
| 에이전트 · 멤버 | R W | R W |
| 에이전트 · 비멤버 | **—** | — |

굵은 두 칸이 SchoolX와 원본 Buzz가 갈리는 지점이다. 사람의 open 의미는
그대로 두고, 에이전트만 좁혔다.

### 적용 경로

계약은 아래 전 경로에서 동일하게 성립해야 한다. ACP 구독 allowlist는 자동
구독 범위일 뿐 보안 경계가 아니다 — 같은 자격증명으로 언제든 raw WebSocket을
열거나 HTTP bridge를 직접 호출할 수 있다.

| 경로 | 검증 테스트 |
|---|---|
| WS `REQ` | `agent_nonmember_cannot_read_open_channel_over_ws` |
| WS `COUNT` | (HTTP `/count`과 동일 게이트) |
| WS `EVENT` | `agent_nonmember_cannot_write_open_channel_over_ws` |
| HTTP `POST /query` | `agent_nonmember_cannot_read_open_channel_over_http_query` |
| HTTP `POST /count` | `agent_nonmember_cannot_count_open_channel_over_http` |
| HTTP `POST /events` | `agent_nonmember_cannot_write_open_channel_over_http` |
| 라이브 fan-out | `agent_nonmember_receives_no_live_fanout` |
| NIP-50 검색 | `search_does_not_leak_open_channel_to_nonmember_agent` |
| kind:9021 self-join | `agent_cannot_self_join_open_channel` |

`COUNT`를 별도로 세는 이유는 **개수도 정보**이기 때문이다. 본문을 못 봐도
메시지가 몇 건인지 알면 이미 누출이다.

`9021`을 세는 이유는 그것이 원래 멤버십 검사를 건너뛰는 kind이기 때문이다.
제한된 에이전트가 스스로 들어갈 수 있으면 계약 전체가 무의미해진다.
`skips_generic_membership`이 같은 플래그를 보고 이 우회를 막는다.

## 3. 권한 부여와 취소

접근 조회 캐시는 TTL 10초다(`AppState`, `state.rs`). 멤버십 변경은 해당
pubkey의 `member_channels_cache` 항목을 명시적으로 무효화한다.

| 동작 | 반영 시점 | 테스트 |
|---|---|---|
| 멤버 제거 | 즉시 (TTL 미만) | `agent_loses_access_immediately_on_removal` |
| 멤버 추가 | 즉시 (TTL 미만) | `agent_gains_access_immediately_on_add` |

두 테스트는 경과 시간을 단언한다. 시간 단언이 없으면 무효화 코드가 죽어 있어도
TTL 만료로 통과하고, 실제로는 모든 권한 취소에 10초의 잔존 접근 창이 생긴다.

### 에이전트 징발 금지

한 번 관리형으로 분류된 키는 **소유자만** 채널에 추가할 수 있다
(`channel_add_policy = 'owner_only'`, 마이그레이션 0025). 이것이 없으면
멤버십 규칙이 "에이전트가 닿는 범위"는 막으면서 "그 범위를 부여하는 행위"는
방치하게 된다. (`stranger_cannot_conscript_a_classified_agent`)

**한계 — 정책은 분류 시점에 바인딩된다.** NIP-OA 태그로 인증하기 *전에* 채널에
추가된 키는 일반 사용자로 들어가고, 이후 에이전트로 분류돼도 그 멤버십을
유지한다. 운영상 의미는 "에이전트로 쓸 키는 채널에 넣기 전에 먼저 소유자
attestation으로 한 번 접속시킨다"이다.

## 4. 기계가 읽는 형태

이후 세션이 코드에서 소비할 수 있도록 같은 내용을 구조화해 둔다.

```yaml
principals: [human, managed_agent]
visibilities: [open, private]
rules:
  - principal: human
    visibility: open
    member: false
    read: allow
    write: allow
  - principal: human
    visibility: private
    member: false
    read: deny
    write: deny
  - principal: managed_agent
    visibility: open
    member: false
    read: deny          # SchoolX가 좁힌 지점
    write: deny         # SchoolX가 좁힌 지점
  - principal: managed_agent
    visibility: [open, private]
    member: true
    read: allow
    write: allow
paths_that_must_agree:
  - ws_req
  - ws_count
  - ws_event
  - http_query
  - http_count
  - http_events
  - live_fanout
  - nip50_search
  - self_join_9021
grant_policy:
  classified_agent_added_by: owner_only   # migration 0025
  binds_at: classification_time
cache:
  ttl_seconds: 10
  revocation: immediate_via_explicit_invalidation
```

## 5. 이 계약이 아직 덮지 않는 것

정직하게 남긴다. 바로 아래 넷은 세션 A 범위 밖이며 **여전히** 완료로
표시하지 않는다. 세션 D가 이 계약 위에 얹은 workspace catalog gap 둘은
세션 E1에서 닫혔고, 무엇이 닫혔고 무엇이 남았는지는 이 절 끝에 따로 적었다.

**derived content의 audience 교집합.** `crates/buzz-core/src/audience.rs`는
I/O 없는 정책 primitive로 존재하고 단위 테스트도 있으나, **어떤 생성·게시
경로에도 연결돼 있지 않다.** 연결 대상인 요약 기능 자체가 아직 없으므로
검증할 표면이 없다. 세션 E3(지식 승격)에서 기능과 함께 연결하며, 그때 게시 직전
membership 재확인과 생성-게시 사이 race까지 함께 테스트한다. 그전까지
"출처 기반 자동 교차 게시"를 제공한다고 표현하지 않는다.

**WF-08.** 승인 게이트는 여전히 미구현이다. 승인 단계에 도달한 workflow는
실패한다. 1차 안전 모드는 에이전트가 허용된 private 채널에 초안만 만들고
사람이 일반 UI에서 수동으로 공유하는 방식이다.

**Blossom 미디어 ACL.** 미디어 경로가 메시지의 `h` 태그와 같은 channel ACL을
제공한다고 가정하지 않는다. 이 매트릭스는 미디어를 다루지 않는다.

**E2E가 상시 게이트에 없다.** `just test-e2e`로 실행할 수는 있으나 CI 잡이
없다. 이 매트릭스는 회귀를 자동으로 잡아주지 못하며, 각 세션 시작 전에 수동
실행이 필요하다. CI 연결은 세션 F(패키징) 범위로 둔다.

### 세션 D가 남긴 workspace catalog gap 둘 — 세션 E1에서 닫힘 (2026-08-04)

둘 다 위 접근 매트릭스가 다루는 채널 read/write 축이 아니라 *누가 적용을
실행할 수 있는가*, *도출된 채널 ID를 누가 선점할 수 있는가*라는 별도
축이었다. 설계는 [`CATALOG_SECURITY.md`](CATALOG_SECURITY.md),
구현은 커밋 `0a2ccada`–`14925137`이다.

**적용은 커뮤니티 관리자만 할 수 있다.** `preflight_workspace_catalog`와
`apply_workspace_catalog`(`desktop/src-tauri/src/commands/workspace_catalog.rs`)
가 진입에서 `require_community_admin`을 통과해야 한다. 근거가 되는 역할은
`relay_members`의 커뮤니티 스코프 역할이고, 그 값은 relay가 서명한
kind 13534에서 온다 — 클라이언트가 만드는 값이 아니라 위조할 수 없다.
`owner`와 `admin`만 통과하고 모르는 역할은 거부한다
(`role_may_apply`). **preflight도 막는다** — 미리보기만으로도 어떤 항목이
이미 적용됐는지가 드러나고 그것은 private 채널의 존재 정보다. 게이트에
걸리면 두 command 모두 부분 결과 없이 실패한다.

**선점한 채널을 남의 것으로 채택하지 않는다.** 도출식의 네 입력은 여전히
공개이므로 선점 자체는 막지 못한다. 막는 것은 그 채널이 *우리 것으로
읽히는* 경로다. 판정 근거를 가변 역할에서 불변 생성자로 옮겼다 —
`channel_owner`는 relay가 kind:39000에 싣는 `created_by`를 읽고
(`side_effects.rs::emit_group_discovery_events`, backfill 경로인 buzz-admin
reconcile도 같은 태그를 낸다), `is_owner`는 `channel_owner(id) == Some(나)`
로 **도출**된다. provenance는 채널 결합(세션 D)에 더해 **그 채널의 생성자가
서명한 것만** 인정하고, 생성자를 특정할 수 없으면 그 채널의 증명서는 전부
버린다. 그래서 선점자가 피해자에게 `admin`이든 `owner`든 무엇을 뿌려도
판정이 움직이지 않는다. 이 불변식은 kind:39000/39001/39002가 relay-only라는
사실 위에 서 있다(`is_relay_only_kind`, 회귀 테스트는 `e2e_relay.rs`의
`test_client_submitted_nip29_member_lists_are_rejected`와
`test_client_submitted_nip29_group_metadata_and_admins_are_rejected`).

증거: `just test-e2e e2e_workspace_catalog` 5/5 — 신규
`squatted_channel_provenance_is_signed_by_the_squatter`가 선점자가 피해
관리자에게 `owner`를 준 **뒤에도** provenance 서명자와 `created_by`가 둘 다
선점자를 가리킨다는 것을 살아있는 relay에서 고정한다. `just test-e2e
e2e_access_matrix` 17/17 유지.

**남는 조건 셋.** 닫힌 것을 과장하지 않기 위해 함께 적는다.

1. **이 게이트는 클라이언트 측이다.** 직접 relay에 kind 9007을 쏘아 채널을
   만드는 것은 막지 못하며, 막으려는 대상도 아니다 — 채널 생성은 모든
   구성원의 정상 권한이다. 막는 것은 「catalog 적용으로 기본 업무방 일습을
   만드는 것」이다. relay에 catalog 인식을 넣지 않은 이유는
   [`CATALOG_SECURITY.md`](CATALOG_SECURITY.md) §4에 있다.
2. **선점의 약한 형태는 여전히 열려 있다 — 복구 경로만 생겼다.** 닫힌 것은
   강한 형태(선점 채널을 피해자가 자기 것으로 채택하는 것)뿐이다. 도출 ID를
   먼저 차지해 그 catalog 항목을 `not_owned`·`deleted`로 막는 것은 그대로
   가능하다. 세션 D3(2026-08-04)이 `generation` 증가 경로를 만들어 관리자가
   다음 세대에 만들 수는 있게 됐지만, 도출식 입력이 전부 공개인 것은 그대로라
   공격자가 `g+1`, `g+2`를 미리 차지할 수 있다 — 누를 때마다 한 칸씩 밀린다.
   **영구 차단이 유한한 경합으로 바뀐 것**이지 선점이 닫힌 것이 아니다.
   진짜로 닫으려면 도출식에 공격자가 예측할 수 없는 입력이 필요하고, 그러면
   기존에 적용된 모든 방의 ID가 바뀌어 마이그레이션이 따로 필요하다
   ([`CATALOG_RECREATE.md`](CATALOG_RECREATE.md) §5·§8).
3. **공동 관리가 막힌다.** `channels.created_by`는 갱신되지 않으므로 관리자
   A가 만든 방은 관리자 B가 몇 번을 재적용해도 `not_owned`다. 역할을 넘겨받아도
   풀리지 않는다. `UserAction::RequestOwnership`은 「소유권을 넘겨받으면
   풀린다」가 아니라 「저 사람에게 실행을 부탁하라」로 읽어야 하고, 그 위임
   실행을 요청하는 흐름은 아직 없다.

## 6. 관련 문서

- [`DEVELOPMENT_PLAN.md`](DEVELOPMENT_PLAN.md) — §2.6 최소 권한, §7 Phase 1
- [`IMPLEMENTATION_HANDOFF.md`](IMPLEMENTATION_HANDOFF.md) — 반드시 유지할 보안 사실 10개
- [`BASELINE.md`](BASELINE.md) — 실행 환경과 재현 절차
