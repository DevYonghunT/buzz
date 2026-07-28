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

정직하게 남긴다. 아래는 세션 A 범위 밖이며, 완료로 표시하지 않는다.

**derived content의 audience 교집합.** `crates/buzz-core/src/audience.rs`는
I/O 없는 정책 primitive로 존재하고 단위 테스트도 있으나, **어떤 생성·게시
경로에도 연결돼 있지 않다.** 연결 대상인 요약 기능 자체가 아직 없으므로
검증할 표면이 없다. 세션 E에서 기능과 함께 연결하며, 그때 게시 직전
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

## 6. 관련 문서

- [`DEVELOPMENT_PLAN.md`](DEVELOPMENT_PLAN.md) — §2.6 최소 권한, §7 Phase 1
- [`IMPLEMENTATION_HANDOFF.md`](IMPLEMENTATION_HANDOFF.md) — 반드시 유지할 보안 사실 10개
- [`BASELINE.md`](BASELINE.md) — 실행 환경과 재현 절차
