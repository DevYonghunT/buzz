# catalog 적용 권한 설계 (세션 E1)

세션 D가 만든 워크스페이스 catalog 적용에 남은 두 구멍을 닫는다. 새 기능은
없다. 배경과 공격 형태는
[`IMPLEMENTATION_HANDOFF.md`](IMPLEMENTATION_HANDOFF.md) 세션 D 「넘긴 것」
8·9번, 현재 강제되는 것은
[`WORKSPACE_CATALOG.md`](WORKSPACE_CATALOG.md) §5·§7·§8,
보안 전제는 [`SECURITY_CONTRACT.md`](SECURITY_CONTRACT.md)를 따른다.

## 1. 지금 열려 있는 것

**A. 아무나 적용할 수 있다.** 설정 섹션에 `featureGate`가 없고 두 Tauri
command 어디에도 역할 검사가 없다. 인증된 아무 커뮤니티 구성원이 적용을
돌려 기본 업무방들의 owner가 될 수 있고, 그러면 진짜 관리자는 `not_owned`를
받는다. Phase 3 완료 기준 #1의 주어가 "관리자가"인데 강제되지 않는다.

**B. 도출 채널 ID를 선점할 수 있다.** §5의 도출식 입력이 전부 공개값이다 —
네임스페이스는 오픈소스 리터럴, `item_key`는 `buzz catalog list`가 출력,
`generation`은 신규 항목이면 항상 `1`. 누구나 관리자보다 먼저 그 ID로 채널을
만들 수 있다.

세션 D가 provenance를 도출된 채널에 묶어(§5) 다른 채널에 실린 위조
증명서는 이미 막았다. 남은 것은 **선점한 채널 안에서 발행한 증명서**다 —
정말 그 채널에 있으므로 채널 결합 검사를 통과한다. 서명자를 보지 않기
때문이다.

강한 형태는 여기서 한 걸음 더 간다. 선점자가 피해 관리자에게 채널 `admin`
역할을 준다(relay는 수신자 동의를 요구하지 않는다). `is_owner`가 `admin`을
받으므로 owner 게이트가 **통과**하고, saga는 선점자의 채널을 채택해 시작
캔버스를 그 안에 쓴다. 게이트를 우회한 것이 아니라 **충족시킨** 것이다.

## 2. 관리자는 이미 정의돼 있다

새 개념을 만들지 않는다. `relay_members` 테이블이 커뮤니티 스코프로
`role IN ('owner','admin','member')`를 갖고, 데스크톱에는 이미
`get_my_relay_membership` command가 있다
(`desktop/src-tauri/src/commands/relay_members.rs`).

**그 값이 relay-signed kind 13534에서 온다는 점이 중요하다.** 클라이언트가
만드는 값이 아니라 서버가 서명한 목록이라 위조할 수 없다. 게이트 근거로
쓸 수 있는 이유다.

채널 레벨 `MemberRole`과 혼동하지 않는다. catalog 적용은 커뮤니티 전체에
기본 업무방을 만드는 일이므로 커뮤니티 역할로 판정한다. **`owner` 또는
`admin`을 요구한다.**

## 3. 게이트는 세 겹이고 하나만 진짜다

| 위치 | 역할 |
|---|---|
| 설정 화면 `featureGate` | 메뉴를 숨긴다. **보안이 아니다** — command를 직접 부를 수 있다 |
| Tauri command | 실제 게이트. `preflight`와 `apply` **양쪽** 진입에서 확인 |
| relay | 이번에는 건드리지 않는다 (§4) |

**preflight도 막는다.** 미리보기만 봐도 어떤 항목이 이미 적용됐는지가
드러나고, 그건 private 채널의 존재 정보다. 적용만 막고 미리보기를 열어두면
게이트가 아니라 지연일 뿐이다.

게이트에 걸리면 두 command 모두 아무것도 하지 않고 실패한다. 부분 결과를
돌려주지 않는다.

## 4. 왜 relay를 건드리지 않는가

에이전트에는 relay가 강제하는 membership 게이트가 있다(세션 A). 그건 relay가
principal을 분류할 수 있기 때문이다 — 검증되거나 영속 분류된 NIP-OA
에이전트라는 서버 측 사실이 있다.

catalog 적용에는 그런 사실이 없다. 사람이 자기 권한으로 채널을 만드는
일이고, relay 입장에서는 정상 동작이다. "이 사람이 catalog를 적용해도
되는가"를 relay가 답하려면 relay가 catalog를 알아야 하는데, 그건 upstream
병합 표면을 늘리면서 얻는 것이 적다.

**그래서 이 게이트는 클라이언트 측이고, 그 한계를 명시한다:** 직접 relay에
kind 9007을 쏘아 채널을 만드는 것은 이 게이트가 막지 못한다. 막는 것은
"SchoolX catalog 적용으로 기본 업무방 일습을 만드는 것"이지 "채널을 만드는
것"이 아니다. 후자는 애초에 모든 구성원의 정상 권한이다.

## 5. provenance 서명자 검증

§5의 채널 결합에 **누가 서명했는가**를 더한다.

**규칙: provenance는 그 채널을 만든 사람이 서명한 것만 인정한다.**

> **정정 이력.** 이 절은 처음에 「그 채널의 **owner**가 서명한 것만」이라고
> 적었다. 그건 안전하지 않다 — 근거는 §6의 정정을 보라. 규칙의 근거를
> 가변 역할에서 불변 생성자(`channels.created_by`)로 옮겼다.

- 정상 경로에서 채널을 만든 사람이 그대로 증명서를 발행하므로 조건이
  자동으로 성립한다.
- 선점 채널에서는 선점자가 생성자라 자기 증명서는 그 채널 안에서 유효하다.
  그러나 관리자 쪽 판정을 오염시키지 못한다 — 관리자는 §8의 게이트에서
  `not_owned`로 막히고, 그 경로는 캔버스도 증명서도 쓰지 않는다.
  (**주의**: 「관리자가 멤버가 아니면 애초에 읽지 못한다」는 **틀린 안심**이다.
  `open` 채널은 비멤버도 읽는다 — relay의 채널 스코프 ACL은 경계가 아니라
  1차 필터다. 그래서 읽힘 자체를 방어로 세지 않는다. 같은 오해가
  `crates/schoolx-catalog/src/preflight.rs`의 주석에도 있었고 함께 정정했다.)
- 생성자가 아닌 사람이 발행한 증명서는 버린다. 조용히 버리되 그 사실을
  코드에 남긴다 — 버려진 증명서가 있다는 것은 위조이거나 버그이지 정상이
  아니다.
- 생성자를 **특정할 수 없으면** 그 채널의 증명서는 전부 버린다. 「모른다」를
  통과로 읽지 않는다.

근거 값은 relay가 kind:39000에 싣는 `created_by` 태그다. 그 kind는
`is_relay_only_kind`가 클라이언트 저작을 ingest에서 거부하므로 위조할 수 없다
(회귀 테스트: `e2e_relay.rs`의
`test_client_submitted_nip29_group_metadata_and_admins_are_rejected`).
이 불변식이 없으면 아무 구성원이나 남의 채널 멤버 목록·메타데이터를 위조해
자기를 생성자로 적을 수 있고 §5가 통째로 무너진다.

### 생성자는 바뀌지 않는다 — 소유권 이전은 스스로 복구되지 않는다

`channels.created_by`는 생성 시 한 번 쓰이고 relay의 어떤 경로도 그 컬럼을
다시 쓰지 않는다. **그것이 이 규칙을 위조 불가로 만드는 성질이자, 동시에
소유권 이전이 통하지 않는 이유다.**

A가 만든 방을 B에게 「넘겨도」 catalog 판정에서 그 방은 영원히 A의 것이다.
B는 몇 번을 다시 돌려도 `not_owned`에 멈춘다. 그러므로
`UserAction::RequestOwnership`은 「소유권을 넘겨받으면 풀린다」가 아니라
**「이 방은 저 사람 것이니 저 사람에게 실행을 부탁하라」**로 읽어야 한다.
화면 문구도 그 뜻으로 되어 있다(`catalog.userAction.request_ownership`).

> 이전 판에는 「소유권 이전은 한 번의 재적용을 요구하고 스스로 복구된다」고
> 적혀 있었다. 그건 역할 기반 판정의 성질이었고, 그 판정은 §6대로 안전하지
> 않아 폐기했다. 열거형 이름 `RequestOwnership`은 그 시절의 잔재다 —
> 의미는 위와 같다.

### 증명서를 발행하는 사람은 언제나 생성자다

이 규칙은 스스로를 지탱한다. 생성자가 아닌 사람은 §8의 쓰기 전 게이트에 먼저
걸려 `not_owned`가 되고, 그 경로는 캔버스도 증명서도 쓰지 않는다. 따라서
생성자 아닌 서명자의 증명서는 정상 동작으로는 만들어질 수 없다 — 있다면
위조이거나 버그다.

## 6. 채택 판정을 `created_by`에 건다

> **이 절은 정정본이다.** 처음에는 「`is_owner`에서 `admin`을 뗀다」였고,
> 근거로 *"`owner`는 채널 생성자에게 고정되어 남이 줄 수 없다"*를 들었다.
> **그 근거는 사실이 아니다.**

relay가 실제로 하는 일은 이렇다. 상위 역할 부여의 유일한 조건은 「주는 쪽이
elevated인가」이고(`buzz-db/src/channel.rs`, `handlers/side_effects.rs`의
PUT_USER 검사) `is_elevated()`는 `Owner | Admin`이다. `MemberRole::Owner`는
평범한 부여 가능 값이다. owner 개수 가드는 전부 **하한**("마지막 owner를
못 뺀다")이라 상한이 없고 DB 유니크 제약도 없다 — 복수 owner가 허용된다.

따라서 `admin`만 떼는 것으로는 아무것도 닫히지 않는다. 선점자는 피해자에게
`admin` 대신 **`owner`**를 주면 되고, 비용은 조금도 늘지 않는다. (데스크톱의
`change_channel_member_role`이 `"owner"`를 거부하기는 하지만, §4가 이미
raw relay 이벤트를 위협 모델로 인정했으므로 클라이언트 측 거부는 방어가
아니다.)

**그래서 판정의 근거를 역할이 아니라 생성자로 옮긴다.**

- `channel_owner`는 kind:39000의 `created_by`를 돌려준다. 역할 목록
  (kind:39002)을 보지 않는다.
- `is_owner`는 그 값에서 **도출**한다 — `channel_owner(id) == Some(나)`.
  별도 근거로 답하면 두 값이 어긋나는 자리가 생기고, 그 틈이 정확히 막으려던
  것이다.
- `admin` 제거는 그대로 유지한다. 이제 그것은 이 규칙의 근거가 아니라
  같은 방향의 defense-in-depth다.

이렇게 하면 §1의 강한 형태가 실제로 닫힌다: 선점자가 무슨 역할을 뿌리든
피해자의 `is_owner`는 거짓이고, 선점 채널의 증명서는 선점자 것이라 피해자의
판정에 반영되지 않는다.

### 트레이드오프 — 공동 관리가 영구히 막힌다

관리자 A가 적용하고 관리자 B가 재시도하면, B가 채널 `owner`여도 채택하지
못하고 `not_owned`를 받는다. 역할을 아무리 조정해도 풀리지 않는다.

- B는 커뮤니티 관리자이므로 §3의 게이트는 통과한다. 적용을 **시작**할 수 있다.
- 그러나 A가 만든 방은 A의 방이고, B가 할 수 있는 일은 A에게 실행을
  부탁하는 것뿐이다(`request_ownership`).
- 안전한 실패다: B가 막히면 사람이 개입하지만, B가 통과하면 선점자도
  통과한다. 둘을 구별할 방법이 생성자 말고 없다.
- **남는 과제**: 「생성자에게 위임 실행을 요청」하는 흐름은 아직 없다.
  이전 판이 "future work"로 미뤄 둔 `request_ownership` UX와는 **다른**
  기능이다 — 소유권을 옮기는 것으로는 해결되지 않기 때문이다.

## 7. 검증 계획

| 검증 | 확인 대상 |
|---|---|
| 크레이트 단위 테스트 | owner 아닌 서명자의 증명서를 버린다 |
| | 선점 채널의 증명서가 관리자 판정을 오염시키지 않는다 |
| | `admin`은 채택을 통과시키지 못한다 (owner만) |
| 어댑터 단위 테스트 | `ev.pubkey`가 손실 없이 전달된다 |
| 데스크톱 테스트 | 역할이 없거나 `member`면 두 command가 실패한다 |
| | 게이트 실패 시 부분 결과를 돌려주지 않는다 |
| live relay E2E | 커뮤니티 `member`가 적용을 시도하면 막힌다 |
| 회귀 | `just test-e2e e2e_access_matrix` 17/17 유지 |

각 테스트는 목표한 버그를 재주입해 실패하는지 확인한 뒤 되돌린다. 세션 D의
`golden_value_matches_known_uuid`가 관계형 테스트만으로는 부족했던 것과 같은
이유다.

## 8. 범위 밖

- E2(에이전트 프로비저닝)와 E3(지식 승격)는 별도 설계다.
- `generation` 증가 경로(§6의 "다시 만들기")는 여전히 미구현이다. 이
  설계는 그것을 만들지 않는다.
- relay 측 catalog 인식은 §4의 이유로 넣지 않는다.

## 9. 근거가 된 코드 경로

| 사실 | 경로 |
|---|---|
| 커뮤니티 역할 모델 | `migrations/0001_initial_schema.sql` `relay_members` |
| 역할을 읽는 command | `desktop/src-tauri/src/commands/relay_members.rs` `get_my_relay_membership` |
| 역할의 권위가 relay-signed | 같은 파일, kind 13534 조회 |
| 채널 결합 검사 (세션 D) | `crates/schoolx-catalog/src/preflight.rs` |
| owner 게이트 | `crates/schoolx-catalog/src/saga.rs` |
| `is_owner`의 admin 허용 | `desktop/src-tauri/src/commands/workspace_catalog.rs` |
| 설정 섹션 등록 | `desktop/src/features/settings/ui/SettingsPanels.tsx` |
