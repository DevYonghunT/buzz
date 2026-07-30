# SchoolX 워크스페이스 catalog 설계 (세션 D)

이 문서는 세션 D의 설계 산출물이다. 버전이 있는 내장 catalog, relay에서 다시
확인 가능한 provenance, idempotent saga, machine-readable result ledger의
구조를 고정한다. 제품 범위는
[`DEVELOPMENT_PLAN.md`](DEVELOPMENT_PLAN.md) §6·Phase 3, 보안 전제는
[`SECURITY_CONTRACT.md`](SECURITY_CONTRACT.md), 세션 운영은
[`IMPLEMENTATION_HANDOFF.md`](IMPLEMENTATION_HANDOFF.md)를 따른다.

## 1. 이번 세션 범위

에이전트 없이, private 업무방 두 개만 재현 가능하게 적용한다.

| 항목 | 범위 |
|---|---|
| catalog 항목 | `meeting`(메인 회의방), `planning`(기획방) — 둘 다 `private` |
| saga 단계 | 채널 생성 → 시작 캔버스 → owner 확인 |
| 적용 화면 | 설정 화면의 새 카드 (미리보기 · 적용 · 결과 · 재시도) |
| CLI | 읽기 전용 — **구현된 것은 catalog 목록(`buzz catalog list`)까지다.** preflight 판정은 데스크톱에만 있다 |
| relay | kind 39500 등록 3곳 |

**범위 밖 (Phase 4 이후):** 에이전트·persona provisioning, workflow, 나머지
8개 업무방, 멤버 초대. ledger 스키마에는 자리만 두고 `not_implemented`로
표시한다.

## 2. 확정된 구조 결정

| 결정 | 선택 | 이유 |
|---|---|---|
| provenance 표현 | 새 Nostr kind 39500 (채널 스코프 addressable) | 로컬 파일이 아닌 relay에 남고, private 채널 ACL이 그대로 적용된다 |
| catalog 위치 | 새 공유 크레이트 `schoolx-catalog`에 컴파일 내장 | 디스크로 드리프트하지 않고, desktop과 CLI가 같은 정의를 읽는다 |
| saga 실행기 | 같은 크레이트 + Tauri command | fault injection을 브라우저 없이 `cargo test`로 검증한다 |

## 3. 왜 provenance를 채널 메타데이터에 넣지 않는가

relay는 kind 39000을 **DB 컬럼에서만 재구성한다**
(`crates/buzz-relay/src/handlers/side_effects.rs`의
`emit_group_discovery_events`). `name`, `about`, `visibility`, `t`, `topic`,
`purpose`, `archived`, `ttl`만 태그로 나가고, 채널 생성 이벤트(kind 9007)에
실은 임의 태그는 어디에도 보존되지 않는다.

따라서 provenance는 **별도 이벤트**여야 한다. relay는 미등록 kind를 거부하므로
(`handlers/ingest.rs`의 `required_scope_for_kind`) 새 kind 등록이 필요하다.

## 4. provenance 이벤트 — kind 39500

**프로토콜 리뷰 대상.** 계획서 §6이 요구한 "다른 Buzz 배포에서도 재사용
가능한 workspace-template manifest"다. SchoolX 전용 필드를 넣지 않는다.

```
kind    39500
d       <catalog_id>:<item_key>          항목당 정확히 하나
h       <channel_id>                     채널 스코프 저장 → private ACL 적용
content JSON (아래)
```

```json
{
  "catalog_id": "schoolx.default",
  "catalog_version": 1,
  "item_key": "meeting",
  "generation": 1,
  "steps": { "channel": "done", "canvas": "done", "membership": "done" },
  "applied_at": "2026-07-28T09:00:00Z"
}
```

- `steps`의 값은 `pending`·`done`·`failed`·`skipped` 넷이다. `skipped`는
  "그 자리에 지켜야 할 사용자 내용이 있어 쓰지 않았다"이고 `done`과 뜻이
  다르다 (§8 「내용이 있는 캔버스는 덮어쓰지 않는다」).
- **`steps`에 값을 더하는 것은 읽기 쪽 breaking change다. 라이터보다 리더가
  먼저 나가야 한다.** 학교 한 곳에 관리자가 여럿이고 앱 버전이 서로 다른 것이
  정상이므로, 새 값을 쓰기 시작하면 그 값을 모르는 빌드가 그 레코드를 만난다.
  그 빌드가 파싱에 실패하면 실패는 **조용하다** — 데스크톱 어댑터가 파싱
  실패한 이벤트를 버리므로(`workspace_catalog.rs`의
  `provenance_records_from_events`, `.filter_map(...ok())`) 그 항목은
  "적용한 적 없음"으로 보이고 saga가
  `create_or_recreate` → `duplicate` → `adopted` → 캔버스 단계로 곧장 내려가
  팀이 써 둔 내용을 덮어쓴다. 순서는 이렇다.
  1. 모르는 값을 관용하는 **리더**를 먼저 릴리스한다 — 모르는 값은
     `StepStatus::Unrecognized`로 읽힌다
     (`crates/schoolx-catalog/src/provenance.rs`).
  2. 그 리더가 퍼진 **뒤에** 새 값을 쓰는 라이터를 릴리스한다.

  모르는 값은 **끝난 것**(`is_settled`)으로 센다. 미완료로 세는 쪽이 안전해
  보이지만 정반대다 — 미완료는 곧 그 단계를 다시 실행한다는 뜻이고, 캔버스
  단계의 재실행이 바로 위의 덮어쓰기다. 끝난 것으로 세면 최악이 "구버전이 이
  항목에 아무것도 하지 않는다"이고 그건 되돌릴 수 있다.
- **addressable 대역(30000–39999)** 이라 NIP-33 LWW가 적용된다. 재시도해도
  이벤트가 쌓이지 않고 항상 최신 하나가 권위다.
- client-signed, `Scope::ChannelsWrite` — `KIND_CANVAS`(40100)와 같은 취급.
- **예약 대역 `39500–39599`.** SQL 마이그레이션 `9001+`와 같은 이유다. upstream이
  같은 번호를 쓰면 조용히 충돌하고, 충돌은 컴파일 타임에 잡히지 않는다.

relay 변경은 세 곳뿐이다.

| 파일 | 변경 |
|---|---|
| `crates/buzz-core/src/kind.rs` | 상수와 예약 대역 주석 |
| `crates/buzz-relay/src/handlers/ingest.rs` | `required_scope_for_kind` |
| `crates/buzz-relay/src/handlers/ingest.rs` | `requires_h_channel_scope` |

### 의도적 트레이드오프

채널 스코프 저장이라 **private 채널의 provenance는 그 채널 멤버만 읽는다.**
비멤버 관리자가 preflight를 돌리면 그 항목의 provenance도 동명 채널도 보이지
않는다 — `conflict`는 비멤버 자신의 접근 가능 채널 목록에 동명 채널이 **보일**
때만 성립하는데, private 채널은 애초에 그 목록에 없다. 그래서 판정은
`conflict`가 아니라 `create_or_recreate`로 나온다.

`create_or_recreate`를 따라 생성을 시도하면 결정론적 ID(§5)가 이미 점유돼
있어 거부되고(`duplicate`), 그 채널은 여전히 비멤버의 접근 가능 목록에 없다.
saga는 이 조합 — §7 `deleted`의 조건인 `duplicate` + 접근 불가 — 을 "예전에
만들었다가 삭제됨"으로 읽어 `user_action: confirm_recreate`를 보고한다.
실제로는 지운 적 없고 지금도 존재하는 방을 두고서다.

이것도 오판이지 오작동은 아니다 — 같은 트레이드오프가 낳은 결과다. 비멤버가
정확한 판정인 `conflict`를 보려면 그 자신의 접근 가능 목록에 동명 채널이
있어야 하는데, 그건 비멤버에게 private 채널의 존재를 알리는 것과 같은 일이다
— provenance를 전역 스코프로 바꿔 누구나 읽게 하는 것과 다르지 않다. 정확히
이 누출을 [`SECURITY_CONTRACT.md`](SECURITY_CONTRACT.md)가 막는다. 그러므로
비멤버에게는 존재를 드러내지 않는 오판(`deleted`)이 정확하지만 존재를
드러내는 판정(`conflict`)보다 안전한 실패다 — 자동 채택 대신 사용자 해결을
요구하는 쪽을 택한 것과 같은 원칙이다.

## 5. 채널 ID 도출

```
UUIDv5(SCHOOLX_CATALOG_NAMESPACE,
       "schoolx-catalog:v1:{relay_scope}:{catalog_id}:{item_key}:{generation}")
```

`desktop/src-tauri/src/commands/channels.rs`의 `starter_channel_uuid()`와 같은
패턴이다. **`catalog_version`은 넣지 않는다** — catalog 버전이 올라가도
`meeting`은 같은 방이어야 한다.

판정 권위는 provenance 이벤트이고, 결정론적 ID는 중복 방지 보조 장치다.
증명서를 남기기 직전에 앱이 죽어도 같은 ID의 두 번째 생성이 relay에서
거부되므로 방이 두 개 생기지 않는다.

**그 증명서의 권위 자체가 이 도출식에 묶여 있다.** 증명서는 도출식이 예측하는
채널에 실려 있을 때만 판정 근거가 된다 — 다른 채널에 실린 것은 버린다. 이유와
버리는 규칙은 §7 「증명서 있음」의 정의에 있다.

## 6. 삭제된 항목: 왜 증명서로 감지할 수 없는가

**확인된 사실 — 구현자가 되돌리면 안 된다.**

1. 채널 삭제는 soft delete다 (`handle_delete_group` → `soft_delete_channel`).
2. `soft_delete_discovery_events`는 **kind 39000/39001/39002만** 지운다.
   39500 행은 살아남는다.
3. 그러나 채널 조회가 전부 `deleted_at IS NULL`로 걸러지므로
   (`crates/buzz-db/src/channel.rs`), **살아남은 증명서를 읽을 수 없다.**

따라서 "증명서는 완료인데 방이 없다"는 대조는 불가능하고, 앱 눈에는
"적용한 적 없음"과 구별되지 않는다.

**대신 쓰는 사실:** 채널 생성은
`INSERT ... ON CONFLICT (community_id, id) DO NOTHING`이고
(`crates/buzz-db/src/channel.rs`), soft-delete된 행이 그 ID를 계속 점유한다.
**한 번 쓴 채널 번호는 영구히 탄다.**

| 같은 ID로 생성 시도 | 의미 |
|---|---|
| 성공 | 정말 처음이다 |
| `duplicate: channel already exists` + 접근 가능 목록에 없음 | 예전에 만들었고 지금은 삭제됐다 |
| `duplicate: channel already exists` + 접근 가능 목록에 있음 | 삭제된 게 아니다. relay는 생성을 커밋했는데 증명서를 쓰기 전에 클라이언트가 죽었다 |

세 번째 줄이 §5가 결정론적 ID를 두는 이유 그 자체다. 증명서가 없다고 방이
없는 것은 아니므로, `duplicate` 한 절만으로 "삭제됨"을 확정하면 살아 있는
방을 두고 세대를 올려 방을 하나 더 만든다.

미리보기 단계에서는 이 셋을 구분할 수 없다. 미리보기는 `create_or_recreate`로
표시하고, 적용 시 거부가 나오면 접근 가능 여부와 owner 여부로 §7의
`deleted`·`not_owned`·`adopted`를 가른다. 앞의 둘이면 **그 항목만 멈추고**
나머지 항목은 계속 진행한다. 사용자가 "다시 만들기"를 선택하면 `generation`을
올려 새 ID로 만들고 증명서에 세대를 기록한다.

> **구현 상태 (세션 D 종료 시점) — 프롬프트는 뜨지만 답할 방법이 없다.** 바로
> 위 문단의 마지막 문장 — "다시 만들기"를 선택하면 `generation`을 올려 새
> ID로 만든다 — 은 아직 아무 코드도 하지 않는다. saga는 §7의 `deleted`
> 판정에서 `user_action: confirm_recreate`를 실제로 보고하고, 설정 카드는 그
> 값을 `catalog.userAction.confirm_recreate`로 번역해 "이전에 삭제한
> 방입니다. 다시 만들까요?"를 **문구로만** 띄운다
> (`WorkspaceCatalogSettingsCard.tsx`) — "예"를 누를 버튼도 체크박스도 없다.
> `generation`을 늘리는 코드 경로는 크레이트 어디에도 없다: preflight·saga·
> ledger·provenance·channel_id 다섯 곳 모두 이미 정해진 값을 그대로 나르기만
> 하고, 신규 항목의 값은 항상 리터럴 `1`이다(`preflight.rs`의
> `None => { ... generation: 1, ... }`).
>
> 사용자 데이터를 잘못 건드리는 결함은 아니다 — `deleted`로 떨어진 항목은
> 아무것도 쓰지 않고 그 자리에 멈춘다. 그러나 이 컨트롤이 생기기 전까지는
> 재시도해도 같은 판정을 반복할 뿐 앞으로 나아갈 방법이 없다. 닫는 방법:
> 설정 카드에 재생성 확인 컨트롤을 추가하고, 확인되면 `generation + 1`로
> saga를 다시 부르는 경로를 Tauri command에 더한다. `derive_channel_id`는
> 이미 `generation`을 인자로 받으므로 saga·Tauri command 쪽만 있으면 된다.

## 7. preflight 판정표

| 판정 | 조건 | 동작 |
|---|---|---|
| `create_or_recreate` | 증명서 없음 + 동명 채널 없음 | 생성 시도. 거부되면 아래 세 줄(`deleted`·`adopted`·`not_owned`) 중 하나로 확정 |
| `resume` | 증명서 있음 + 일부 단계 미완료 | 미완료 단계만 실행 |
| `no_change` | 증명서 있음 + 전 단계 완료 | 아무것도 하지 않음 |
| `deleted` | 생성이 `duplicate`로 거부됨 + 접근 불가 | 자동 재생성 없음. 명시적 선택만 |
| `adopted` | 생성이 `duplicate`로 거부됨 + 접근 가능 + 적용자가 owner | 이미 만들어진 방으로 보고 채널 단계를 완료 처리. 캔버스부터 이어서 진행 |
| `not_owned` | 생성이 `duplicate`로 거부됨 + 접근 가능 + 적용자가 owner 아님 | 채택 없음. **아무것도 쓰지 않고** 사용자 해결 요청 |
| `conflict` | 증명서 없음 + 동명 채널 있음 | 자동 채택 없음. 사용자 해결 요청 |
| `retired` | 증명서 있음 + catalog에 항목 없음 | 목록에서 숨김. **기존 채널은 유지** |

판정 근거는 이름이 아니라 증명서다. 이름은 `conflict` 감지에만 쓴다.

### 「증명서 있음」은 「**도출된 채널에 실린** 증명서가 있음」이다

위 표의 「증명서 있음/없음」은 relay가 읽어 준 kind 39500 이벤트의 유무가
아니다. relay의 읽기 ACL은 "이 사용자가 접근할 수 있는 채널의 이벤트"까지만
좁히는데, 그 집합에는 커뮤니티의 **모든 `open` 채널**이 들어간다. 인증된
사용자라면 누구나 open 채널을 만들어 자기 채널에 kind 39500을 발행할 수 있고
— 자기 채널이므로 그 쓰기는 relay가 정당하게 승인한다 — 학교에서는 학생
아무나가 그 사용자다. 레코드의 `d` 태그도 `item_key`도 `generation`도 `steps`도
전부 발행자가 정하는 값이라, 어느 것도 그 레코드의 출처를 증명하지 않는다.

위조할 수 없는 결합은 하나뿐이다: 그 레코드가 실려 있는 채널(`h` 태그)이
§5의 `derive_channel_id(relay_scope, catalog_id, item_key, generation)`이
예측하는 채널과 같은가. preflight는 이 검사를 통과하지 못한 레코드를 **없었던
것처럼 버린다** (`crates/schoolx-catalog/src/preflight.rs`의
`record_sits_in_its_derived_channel`). 오류로 올리지 않는 이유는, 그러면 아무나
발행할 수 있는 이벤트 하나로 관리자의 preflight 전체를 영구히 막을 수 있기
때문이다. 버리면 그 항목은 `create_or_recreate`로 떨어져 정상 경로를 그대로
탄다.

검사가 없으면 학생 하나가 `d = "schoolx.default:meeting"`에 `steps` 전부
완료인 레코드를 자기 채널에 발행하는 것만으로 그 항목이 영원히 `no_change`가
되고 체크박스가 잠긴다. 관리자에게 우회로가 없다 — 그 이벤트는 남의 채널에
있어 지울 수 없고, NIP-33 LWW는 `(kind, pubkey, d)`별이라 덮어쓸 수도 없다.
`generation`까지 위조하면 채널 ID 도출 입력이 통째로 공격자 손에 들어간다
(다른 무엇도 그 필드를 흔들지 않는다). 그래서 이 검사는 어댑터가 relay 응답
에서 `h` 태그를 **잃지 않고** 넘기는 데까지 함께 걸린다
(`CatalogEffects::fetch_provenance`가 `(채널, 레코드)` 쌍을 돌려주는 이유).

**한 `item_key`는 한 줄이다.** kind 39500은 `(kind, pubkey, d)`별로
addressable이라 NIP-33 LWW가 신원 안에서만 적용된다 — 한 항목을 적용한 신원
수만큼 레코드가 남는다(§4). `retired` 줄은 레코드가 아니라 `item_key` 기준으로
중복을 제거한다. 관리자 둘이 적용했던 항목이 catalog에서 빠졌다고 화면에 같은
방이 두 번 나오면 안 된다.

`renamed`는 판정이 아니라 **별도 플래그**다. 사용자가 이름을 바꾼 항목도
단계가 미완료면 `resume`, 완료면 `no_change`로 판정된다. 플래그는 표시만 하고
**이름을 catalog 값으로 되돌리지 않는다.** 판정과 분리해야 "이름을 바꿨고
캔버스도 실패한" 항목이 재시도에서 누락되지 않는다.

> **구현 상태 (세션 D 종료 시점) — 미리보기까지만이다.** `renamed`는
> `PreflightItem`에만 있고(`crates/schoolx-catalog/src/preflight.rs`),
> `LedgerItem`에는 **없다**(`crates/schoolx-catalog/src/ledger.rs`). 위 문장이
> 원래 요구한 "미리보기와 ledger 둘 다"는 아직 성립하지 않는다.
>
> 결과가 있는 defect다. `LedgerItem::name`은 catalog가 정한 이름이지 그 방의
> **현재** 이름이 아닌데, ledger만 읽는 소비자에게는 그 사실을 알릴 플래그가
> 없다. 이름이 바뀐 방은 §7 「도달 조건」에 따라 `adopted`로 끝나므로 실제로
> 도달하는 상태다. 닫는 방법: `LedgerItem`에 `renamed: bool`을 더하고
> `PreflightItem`에서 그대로 나른 뒤 `ledger_serializes_for_ui_and_cli`의
> golden에 넣는다. 이것은 wire format 변경이므로 §4의 리더-우선 순서를 따른다.

### `adopted`의 owner 게이트

`adopted`는 §6 표의 세 번째 줄 — relay가 생성을 커밋한 뒤 증명서를 쓰기 전에
클라이언트가 죽은 상태 — 를 흡수한다.

**도출된 ID의 방이라는 사실은 그 방이 이 catalog의 방이라는 증거가 아니다.**
이 문서의 예전 판은 "ID가 도출값이라 그 방이 이 catalog의 방이라는 것은
확실하다"라고 적었다 — 틀렸다. 도출식의 입력은 전부 공개다: 네임스페이스는
오픈소스 코드의 리터럴이고(`crates/schoolx-catalog/src/channel_id.rs`),
`item_key`는 CLI가 그대로 출력하며(`buzz catalog list`), relay 범위는 그 학교의
주소다. 즉 아무나 그 UUID를 계산해 **미리 그 ID로 채널을 만들어 둘 수 있다.**
`duplicate` + 접근 가능이 말해 주는 것은 "이 ID는 이미 점유돼 있고 나는 그
방을 볼 수 있다"까지다.

**도출된 ID가 실제로 보장하는 것은 방의 출처가 아니라 증명서와 채널의
결합이다.** provenance 레코드는 도출식이 예측하는 채널에 실려 있을 때만 판정
근거가 된다(위 「증명서 있음」의 정의). 그 채널에 발행하려면 그 채널의 쓰기
권한이 필요하므로, 남의 채널에 레코드를 아무리 쌓아도 판정은 움직이지 않는다.

방 자체를 지키는 것은 그 다음 장치인 owner 게이트다. 공격자가 도출된 ID로
채널을 선점해 두었다면 적용은 `duplicate` + 접근 가능 + owner 아님으로
떨어져 `not_owned`가 되고, **아무것도 쓰지 않은 채** 사용자 해결로 넘어간다.

**남는 구멍 — 결합 검사가 막지 못하는 것.** 채널 ID는 클라이언트가 정하는
값이므로, 도출된 ID로 채널을 선점한 공격자는 **자기 채널 안에서** 결합 검사를
통과하는 레코드를 만들 수 있다. 그러면 그 항목은 다시 `no_change`로 잠긴다.
이벤트 하나를 발행하면 되는 일과는 값이 다르다 — 항목마다 그 ID를 영구히
태워야 하고(§6), 잠긴 항목은 그 ID 하나에 한정된다. 다만 설계상의 복구
경로인 §6의 「세대를 올려 새 ID로 만든다」는 **아직 구현돼 있지 않다**
(`generation`을 올리는 코드 경로가 없다). 이 구멍을 완전히 닫으려면 레코드의
**서명자**가 그 채널의 owner/admin인지까지 확인해야 하는데, kind 39500
content에는 서명자가 없고 `CatalogEffects`는 이벤트가 아니라 레코드를 본다 —
§4의 wire format 변경이나 새 effect가 필요한 별도 작업이다.

**그렇다고 쓸 권한이 생기지는 않는다.** 증명서를 남기지 못한 것은 관리자 A인데
그 방의 멤버일 뿐인 관리자 B가 적용을 돌릴 수 있다. B에게는 방이 보이고
증명서는 안 보이므로 판정이 그대로 여기까지 온다. 그래서 **owner 확인을
캔버스를 쓰기 전에** 한다. 캔버스를 먼저 쓰고 나중에 확인하면, 확인이 실패한
시점에는 팀이 그 방에 써 둔 내용이 이미 사라진 뒤다 — 되돌릴 수 없다.

- owner 확인이 `false`면 `not_owned`다. **캔버스도 증명서도 쓰지 않는다.**
- owner 확인 자체가 실패하면(relay 오류) 채택도 차단도 확정하지 않는다.
  채널 단계를 `failed`로 적고 멈춘다 — 모르는 채로 쓰지 않는 쪽이 안전한
  실패다. 재시도가 다시 묻는다.

ledger가 보고하는 값은 이렇다.

| 판정 | `outcome` | `user_action` |
|---|---|---|
| `adopted` | `applied` (남은 단계가 다 끝나면) | `null` |
| `not_owned` | `blocked` | `request_ownership` |

`adopted`를 `create_or_recreate`로 보고하지 않는다. 둘 다 `applied`로 끝나므로
같은 값으로 적으면 사용자가 "새로 만든 방"과 "이미 있던 방을 넘겨받았다"를
구별할 방법이 없다.

**도달 조건.** 방 이름이 catalog 값 그대로면 preflight가 먼저 `conflict`로
막으므로 생성 시도 자체를 하지 않는다. 즉 `adopted`·`not_owned`는 그 방의
이름이 바뀐 경우에만 나온다 — 위 `renamed` 플래그가 판정에 끼어들지 않기
때문에 그 항목이 계속 적용 대상으로 남는다.

## 8. saga 단계와 보상 규칙

단계: **채널 생성 → (이번 실행이 만들지 않은 방이면) owner 게이트 → 시작
캔버스(그 방에 내용이 있으면 쓰지 않는다) → owner 확인**

각 단계는 (1) 증명서를 보고 완료면 건너뛰고, (2) 실행하고, (3) 증명서를
갱신한다.

- **실패해도 되돌리지 않는다.** 채널 생성 후 캔버스에서 실패하면 채널을
  지우지 않고 `canvas: failed`로 기록한다. 재시도는 캔버스부터 시작한다.
- 보상은 **이번 실행에서 새로 만든 리소스**만 대상이다. 기존 사용자 데이터는
  어떤 경우에도 자동 삭제하지 않는다.
- 한 항목의 실패가 다른 항목을 막지 않는다.

### 쓰기 전 owner 게이트

**이번 실행이 만들지 않은 방은 첫 쓰기 전에 owner를 확인한다.** 만든 사람이
곧 owner이므로 이번 실행이 **직접 만든** 방만 이 게이트를 건너뛴다. §7의
`adopted`와 `resume`이 규칙 하나로 덮인다 — 특례 두 개가 아니다.

| 단계 1의 결과 | 게이트 | 통과 시 판정 |
|---|---|---|
| 새로 만들었다 | 건너뛴다 — 생성이 곧 owner 권한이다 | `create_or_recreate` |
| 건너뛰었다 (증명서가 `channel: done`) | **확인한다** | `resume` |
| `duplicate` + 접근 가능 | **확인한다** | `adopted` |
| `duplicate` + 접근 불가 | 확인하지 않는다 — 그 자리에서 멈춘다 | `deleted` |

- 확인이 `false`면 `not_owned`다. **캔버스도 증명서도 쓰지 않는다.**
  §7 표의 `not_owned` 행은 채택 경로를 적은 것이고, 같은 판정이 재개
  경로에서도 나온다.
- 확인 자체가 실패하면(relay 오류) 채택도 차단도 확정하지 않는다. 채널
  단계를 `failed`로 적고 멈춘다 — 별도 판정을 만들지 않는다. 증명서를
  발행하지 않으므로 relay에 남아 있는 증명서는 그대로고, 재시도가 다시 묻는다.

**재개 경로가 채택 경로보다 더 흔하다.** 증명서는 채널 스코프라 owner가
아니라 **멤버**면 읽힌다(§4의 트레이드오프). 관리자 A가 만든 방의 캔버스
단계가 일시적으로 실패해 증명서가 `channel: done, canvas: failed`로 남고 팀이
그 방을 쓰기 시작하면, 멤버일 뿐인 B의 적용이 `resume`으로 들어와 단계 1을
통째로 건너뛴다. 채택은 증명서가 **아예 없고** 방 이름까지 바뀌어야 도달하지만
(§7 「도달 조건」), 미완료 증명서는 부분 실패의 정상적인 결과다 — 위
「실패해도 되돌리지 않는다」가 설계상 만들어 내는 상태다.

### 내용이 있는 캔버스는 덮어쓰지 않는다

**캔버스 단계는 쓰기 전에 그 방의 현재 캔버스를 읽는다.** 위 owner 게이트는
*누가* 써도 되는가만 가른다 — 쓸 권한이 있어도 그 방에 지켜야 할 내용이 있을
수 있다.

도달 조건은 예외가 아니라 위 「실패해도 되돌리지 않는다」가 설계상 만들어 내는
상태다. 관리자가 적용을 돌렸는데 캔버스 단계가 일시적으로 실패해 증명서가
`channel: done, canvas: failed`로 남는다. 팀이 그 방을 쓰기 시작해 자기 내용을
채운다. 관리자가 재시도를 돌린다 — 조건 없이 쓰면 이 지점에서 팀의 내용이
catalog 기본값으로 사라진다. 되돌릴 수 없다.

| 읽기 결과 | 동작 | 단계 상태 |
|---|---|---|
| 내용이 있다 | **쓰지 않는다** | `skipped` |
| 캔버스가 없다 · 공백뿐이다 | 시작 캔버스를 쓴다 | `done` (실패하면 `failed`) |
| 읽기 자체가 실패했다 | **쓰지 않는다.** 그 자리에서 멈춘다 | `failed` |

- `skipped`는 `done`과 **구별해서** 적는다. 둘 다 "이 단계는 끝났다"이지만
  `done`은 catalog 값이 그 방에 들어가 있다는 뜻이고 `skipped`는 들어가 있지
  않다는 뜻이다. 같은 값으로 적으면 ledger가 하지 않은 쓰기를 보고하고,
  사용자는 자기 내용이 남았다는 사실을 읽을 방법이 없다.
- `skipped`는 **완료로 센다.** 재시도도 다시 읽지 않고 다시 쓰지 않는다.
  미완료로 세면 그 항목은 영원히 `partial`로 보고되는데, 재시도가 도달할 수
  있는 결론은 "쓰지 않는다" 하나뿐이다 — 사용자에게는 끝나지 않는 실패로
  보인다.
- 공백뿐인 캔버스는 지켜야 할 내용이 아니다. 그걸 내용으로 세면 사람 눈에
  비어 있는 방이 시작 캔버스를 영영 받지 못한다.
- **읽기 자체가 실패하면 쓰지 않는다.** 캔버스 단계를 `failed`로 적고 사유를
  싣는다 — 위 owner 확인 실패와 같은 규칙이다. 모르는 채로 쓰면 잃는 것이
  팀의 내용이고 되돌릴 수 없지만, 쓰지 않으면 잃는 것은 이번 실행의 진행뿐
  이다. 증명서는 미완료로 남으므로 재시도가 `resume`으로 들어와 이 단계부터
  이어서 하고 그때 다시 묻는다.

읽기와 쓰기 사이는 원자적이지 않다. 그 틈에 들어온 쓰기까지 막지는 못한다 —
이 규칙이 막는 것은 **이미 있던** 내용을 catalog가 지우는 것이다.

`membership` 단계는 이번 세션에서 **적용자가 실제로 owner로 들어갔는지
확인하고 증명서에 기록하는 데까지**만 한다. 초대는 범위 밖이다. 게이트를
지난 항목도 이 단계를 건너뛰지 않는다 — 게이트는 "써도 되는가"를 가르고, 이
단계는 그 사실을 증명서에 남긴다. 게이트를 지나지 않는 생성 경로에서는 이
단계가 **유일한** owner 검증이다.

## 9. 공개 범위 preflight

catalog의 두 항목은 모두 `private`이 기본이다. 관리자가 `open`으로 바꾸면
미리보기와 확인 화면에 두 문장을 함께 띄운다.

- 모든 로그인 사용자가 멤버가 아니어도 읽고 쓸 수 있습니다.
- 관리형 에이전트는 명시적으로 추가된 경우에만 접근합니다.

두 번째 문장은 세션 A에서 서버로 강제한 사실이고, 사람이 반대로 오해하기
쉬운 지점이다.

> **구현 상태 (세션 D 종료 시점) — 문구는 있고 경로는 없다.** 두 문장은
> `catalog.openWarningScope`·`catalog.openWarningAgents`로 번역까지 들어가
> 있고 카드가 렌더할 자리도 있지만, 게이트인
> `isOpenVisibility()`(`WorkspaceCatalogSettingsCard.tsx`)가 **무조건
> `false`를 돌려준다.** `visibility`가 Rust `CatalogItem`에만 있고
> `PreflightItem` → Tauri command → `CatalogPreflightItem`으로 실려 나오지
> 않기 때문이다. 지금은 내장 항목 둘 다 `private`이라 도달할 수 없는 상태가
> 맞지만, **`open` 항목을 추가하는 작업은 이 필드를 먼저 실어 보내야 한다.**
> 그러지 않으면 open 방이 화면에서 private처럼 보인다. 코드에도 같은 경고가
> 함수 doc comment로 붙어 있다.

## 10. result ledger

적용 실행이 반환하는 machine-readable 결과다.

```json
{
  "catalog_id": "schoolx.default",
  "catalog_version": 1,
  "items": [
    {
      "item_key": "meeting",
      "name": "메인 회의방",
      "decision": "create_or_recreate",
      "channel_id": "…",
      "generation": 1,
      "steps": { "channel": "done", "canvas": "failed", "membership": "pending" },
      "outcome": "partial",
      "user_action": null,
      "error": "캔버스 적용 실패"
    }
  ]
}
```

**`steps`는 배열이 아니라 객체다.** 이 문서의 초안은
`[{ "step": "channel", "status": "done" }, …]` 형태를 적었지만 구현은 §4의
provenance와 **같은 타입**(`StepStates`)을 그대로 싣는다 — 단계 이름이
컴파일 타임에 고정된 3필드 레코드다(`provenance.rs`). 두 곳이 같은 타입을
쓰기 때문에 증명서와 ledger의 단계 어휘가 어긋날 수 없고, 단계를 추가하는
것은 타입 변경이라 조용히 지나갈 수 없다. 배열 형태였다면 둘이 따로 놀았을
것이다. 단계별 `error`는 단계마다가 아니라 항목 하나에 **`error` 필드 하나**로
싣는다.

`outcome` ∈ `applied` · `unchanged` · `partial` · `blocked`.
`user_action` ∈ `null` · `confirm_recreate` · `resolve_conflict` ·
`request_ownership`.
`decision`은 §7 판정표의 여덟 값이다.

`name`은 catalog가 정한 **표시 이름**이고, preflight 결과에도 같은 필드가
실린다. 사람이 보는 표면(설정 카드·CLI)은 이 값을 쓰고 `item_key`를 쓰지
않는다 — 키는 내부 식별자라 그대로 나가면 사용자가 `메인 회의방` 자리에서
`meeting`을 본다. **`retired` 항목은 `null`이다**: catalog에서 빠진 항목이라
이름이 남아 있는 곳이 없다(이름은 catalog에만 있고 증명서에는 실리지 않는다).
그 자리를 `item_key`로 메우지 않는 이유는, 그러면 "이게 이름이다"와 "이름을
모른다"가 같은 값이 되어 소비자가 둘을 구별할 수 없기 때문이다. `null`을
어떻게 보여줄지는 소비자가 정한다.

이 값은 그 방의 **현재** 이름이 아니다. 사용자가 방 이름을 바꿨으면 다르고,
그 사실은 `renamed` 플래그가 말한다 (§7).

`request_ownership`은 `resolve_conflict`와 조치가 다르다. 이름 충돌은 사용자가
자기 채널 이름을 바꾸거나 항목을 건너뛰면 풀리지만, `not_owned`는 owner에게
적용을 맡기거나 owner 권한을 받아야만 풀린다.

계획서 Phase 3의 "성공, 실패, 건너뜀, 사용자 조치 필요"가 각각
`outcome`과 `user_action`에 대응한다. "건너뜀"은 `outcome`이 아니라
`steps.canvas == "skipped"`가 말한다 — §8의 「내용이 있는 캔버스는 덮어쓰지
않는다」가 만들어 내는 상태이고, 그 항목의 `outcome`은 `applied`다.

> **소비자 현황 (세션 D 종료 시점).** 이 ledger를 읽는 것은 데스크톱 설정
> 카드뿐이다. 이번 세션의 CLI(`buzz catalog list`)는 **내장 catalog 항목만**
> 출력하는 읽기 전용 명령이라 preflight도 ledger도 읽지 않는다
> (`crates/buzz-cli/src/commands/workspace_catalog.rs`). 단위 테스트
> `ledger_serializes_for_ui_and_cli`가 고정하는 것은 "지금 두 소비자가 읽고
> 있다"가 아니라 **wire format 그 자체**이며, CLI 적용 경로가 붙을 때 그
> 형식이 이미 고정돼 있게 하는 것이 목적이다.

## 11. 검증 계획

| 검증 | 명령 | 확인 대상 |
|---|---|---|
| 판정표 단위 테스트 | `cargo test -p schoolx-catalog` | §7의 여덟 판정과 `renamed` 플래그 |
| 증명서–채널 결합 | `cargo test -p schoolx-catalog` | 도출된 채널이 아닌 곳에 실린 증명서는 버려진다(§7 「증명서 있음」). `retired`는 `item_key`당 한 줄 |
| `h` 태그 보존 | `cargo test --manifest-path desktop/src-tauri/Cargo.toml` | 어댑터가 relay 응답의 `h` 태그를 잃지 않고 넘기고, 없거나 파싱되지 않으면 그 이벤트를 버린다 |
| fault injection | `cargo test -p schoolx-catalog` | 세 단계 각각의 실패(커밋 전·커밋 후 포함) 후 재시도가 desired state 도달 |
| 권한 게이트 | `cargo test -p schoolx-catalog` | `not_owned`에서 캔버스·증명서 둘 다 쓰지 않음 |
| 덮어쓰기 금지 | `cargo test -p schoolx-catalog` | 내용이 있는 캔버스는 그대로 두고 `skipped`로 보고, 빈 방에는 시작 캔버스를 씀, 읽기 실패는 쓰지 않고 `failed` |
| 재적용 | `cargo test -p schoolx-catalog` | 두 번째 적용이 `no_change`이고 채널 수 동일 |
| catalog snapshot | `cargo test -p schoolx-catalog` | 항목 키·이름·공개 범위 고정 |
| relay kind 수용 | `just test-e2e` | 39500 발행·조회, 비멤버 차단 |
| 데스크톱 | `pnpm --dir desktop test` | 설정 카드 렌더와 결과 표시 |

effect trait로 relay I/O를 주입하므로 fault injection은 live relay 없이
돈다. relay 왕복이 필요한 것만 E2E로 간다.

**완료 표시 조건:** Phase 3 완료 기준 7개 중 하나라도 증거가 없으면 이 단계를
완료로 적지 않는다 ([`IMPLEMENTATION_HANDOFF.md`](IMPLEMENTATION_HANDOFF.md)
기본 원칙).

### 실제로 돈 결과 (2026-07-30)

| 계획한 검증 | 결과 |
|---|---|
| `cargo test -p schoolx-catalog` (`just test-unit` 경유) | 66 passed / 0 failed |
| `just test-e2e e2e_workspace_catalog` | 4 passed / 0 failed |
| `just test-e2e e2e_access_matrix` (세션 A 계약) | 17 passed / 0 failed |
| `pnpm --dir desktop test` (`just desktop-test`) | 3,756 passed / 0 failed |

**계획한 검증 중 하나가 실제로는 서지 않았다.** 위 표의 "데스크톱 —
`pnpm --dir desktop test` — 설정 카드 렌더와 결과 표시" 행은 통과했지만, 그
3,756개 중 `WorkspaceCatalogSettingsCard.tsx`를 렌더하는 테스트는 **하나도
없다.** Playwright 스펙도 이 카드의 `data-testid`를 참조하지 않는다. 카드가
`outcome`·`user_action`·`skipped`로 분기한다는 것은 소스를 읽어서만 확인했고,
`tsc`가 `catalog.*` 번역 키의 존재를 보장하는 데까지다. 실행 증거는 없다.
Phase 3 완료 기준 #7의 UI 절이 여기서 막힌다 — 자세한 것은
[`IMPLEMENTATION_HANDOFF.md`](IMPLEMENTATION_HANDOFF.md) 세션 D를 본다.

`just ci`는 이 저장소 환경에서 명령 하나당 10분 제한을 넘긴다(cold clippy
빌드만 9분 44초). 구성 레시피 8개를 순서대로 따로 돌려 전부 통과를 확인했다.
기록은 [`BASELINE.md`](BASELINE.md).

### 브랜치 리뷰 수정 (2026-07-31)

위 표는 그날 돈 그대로의 기록이다. 이후 브랜치 전체 리뷰에서 증명서–채널
결합이 강제되지 않는 결함이 나와 §5·§7을 고치고 위 검증 계획에 두 행을
더했다. 그 수정 뒤 `cargo test -p schoolx-catalog`는 71 passed / 0 failed,
`cargo test --manifest-path desktop/src-tauri/Cargo.toml`은 1,837 passed /
0 failed / 14 ignored, `pnpm --dir desktop typecheck`는 통과다. 무엇이
왜 바뀌었는지는 §5·§7과 그 코드 주석에 그대로 있다.

## 12. 코드 경로

### 이번 세션이 만든 것

| 역할 | 경로 |
|---|---|
| 내장 catalog 데이터 (`include_str!`) | `crates/schoolx-catalog/catalog.json` |
| catalog 타입과 `builtin()` | `crates/schoolx-catalog/src/{lib,catalog}.rs` |
| 결정론적 채널 ID (§5) | `crates/schoolx-catalog/src/channel_id.rs` |
| kind 39500 wire 타입과 `StepStatus` (§4) | `crates/schoolx-catalog/src/provenance.rs` |
| relay I/O 주입 trait과 fault-injection fake | `crates/schoolx-catalog/src/effects.rs` |
| preflight 판정표 (§7) | `crates/schoolx-catalog/src/preflight.rs` |
| idempotent saga, owner 게이트, 캔버스 보호 (§8) | `crates/schoolx-catalog/src/saga.rs` |
| result ledger (§10) | `crates/schoolx-catalog/src/ledger.rs` |
| kind 39500 상수와 예약 대역 | `crates/buzz-core/src/kind.rs` |
| kind 39500 수용과 `h` 스코프 강제 | `crates/buzz-relay/src/handlers/ingest.rs` |
| relay 어댑터와 Tauri command | `desktop/src-tauri/src/commands/workspace_catalog.rs` |
| 설정 카드 (§9·§10 표시) | `desktop/src/features/settings/ui/WorkspaceCatalogSettingsCard.tsx` |
| react-query 훅 | `desktop/src/features/workspace-catalog/hooks.ts` |
| TS wire 타입과 invoke 래퍼 | `desktop/src/shared/api/tauriWorkspaceCatalog.ts` |
| 읽기 전용 CLI (`buzz catalog list`) | `crates/buzz-cli/src/commands/workspace_catalog.rs` |
| live relay E2E | `crates/buzz-test-client/tests/e2e_workspace_catalog.rs` |

### 근거가 된 기존 코드 경로

| 사실 | 경로 |
|---|---|
| 39000은 DB 컬럼에서만 재구성 | `crates/buzz-relay/src/handlers/side_effects.rs` `emit_group_discovery_events` |
| 미등록 kind 거부 | `crates/buzz-relay/src/handlers/ingest.rs` `required_scope_for_kind` |
| 채널 스코프 게이트 | `crates/buzz-relay/src/handlers/ingest.rs` `requires_h_channel_scope` |
| 결정론적 채널 ID 선례 | `desktop/src-tauri/src/commands/channels.rs` `starter_channel_uuid` |
| 중복 채널 거부 | `crates/buzz-relay/src/handlers/ingest.rs` / `crates/buzz-db/src/channel.rs` `create_channel_with_id` |
| soft delete | `crates/buzz-relay/src/handlers/side_effects.rs` `handle_delete_group` |
| 삭제 채널 조회 차단 | `crates/buzz-db/src/channel.rs` (`deleted_at IS NULL`) |
| 기존 template 저장소 | `desktop/src-tauri/src/templates/storage.rs` |
| 기존 적용 경로 (교체 대상) | `desktop/src/features/channel-templates/useApplyTemplate.ts` |
| CLI 템플릿 읽기 | `crates/buzz-cli/src/commands/channel_templates.rs` |
