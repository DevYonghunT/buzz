# catalog 재생성 설계 (세션 D3)

`generation`을 올리는 코드 경로가 크레이트 전체에 없어서 생긴 두 막힘을
닫는다. 새 판정도 새 단계도 만들지 않는다 — 이미 있는 `deleted`·`not_owned`
판정에 **답할 수단**을 준다.

배경은 [`IMPLEMENTATION_HANDOFF.md`](IMPLEMENTATION_HANDOFF.md) 세션 D
「넘긴 것」 7번과 세션 E1 「넘긴 것」 1·2번, 현재 강제되는 것은
[`WORKSPACE_CATALOG.md`](WORKSPACE_CATALOG.md) §5·§6·§7·§8, 보안 전제는
[`CATALOG_SECURITY.md`](CATALOG_SECURITY.md)를 따른다.

## 1. 지금 막혀 있는 것

두 세션이 각각 독립적으로 같은 결론에 도달했다. 원인이 하나다.

**A. `deleted` 판정이 묻는 말에 답할 수 없다.** saga는 `duplicate` + 접근
불가를 「예전에 만들었다가 삭제됨」으로 읽고 `user_action: confirm_recreate`를
보고한다. 카드는 그것을 "이전에 삭제한 방입니다. 다시 만들까요?"로 띄운다.
**"예"를 누를 컨트롤이 없다.** `generation`을 늘리는 코드 경로가
preflight·saga·ledger·provenance·channel_id 다섯 곳 어디에도 없고, 신규
항목의 값은 항상 리터럴 `1`이다.

**B. 선점의 약한 형태가 영구히 복구되지 않는다.** 도출 ID의 입력이 전부
공개이므로 누구나 관리자보다 먼저 그 ID로 채널을 만들 수 있다. 그러면 모든
관리자의 적용이 `not_owned`에 멈추고, 세대를 올릴 수 없으니 그 catalog 항목은
그 학교에서 영원히 막힌다. 세션 E1은 **강한 형태**(선점 채널을 피해자가 자기
것으로 채택)만 닫았다 — 안전한 실패이지 가용성 보장이 아니라고 적은 그대로다.

두 막힘의 해소는 같은 메커니즘 하나다.

## 2. 세대를 preflight가 알아낼 수는 없다

먼저 되지 않는 방법을 지운다. **탄 세대를 provenance에서 읽어 올 수 없다.**

§6이 확인한 사실이다. 채널 삭제는 soft delete이고 `soft_delete_discovery_events`는
kind 39000/39001/39002만 지우므로 39500 행은 살아남지만, 채널 조회가 전부
`deleted_at IS NULL`로 걸러지기 때문에 살아남은 증명서에 닿을 수 없다. 선점
채널의 증명서는 살아 있어도 **서명자가 선점자**라 §5의 검증에서 버려진다.

그래서 「preflight가 최신 세대를 조회해 +1 한다」는 성립하지 않는다. 세대를
아는 유일한 주체는 **방금 그 판정을 받은 실행**이다.

## 3. 세대는 사용자 확인이 나른다

**규칙: 재생성은 「어느 세대를 보고 내린 결정인가」를 함께 제출한다.**

ledger가 이미 항목마다 `generation`을 싣는다(막힌 세대다). 사용자가 그 화면을
보고 "다시 만들기"를 누르면, 다음 적용은 그 값을 들고 온다.

```
Selection { item_key, recreate_from: Option<u32> }
```

- `recreate_from: None` — 평소 적용. 지금과 같다.
- `recreate_from: Some(g)` — 「세대 `g`가 막힌 것을 보았다. 다음 것을
  만들어라」. saga는 `g + 1`로 도출해 생성한다.

**왜 `g + 1`이지 「다음 빈 세대 탐색」이 아닌가.** 탐색은 relay 왕복을 세대마다
한 번씩 쓰면서도 경계가 없고, 무엇보다 **사용자가 승인한 것보다 더 멀리 간다.**
`g + 1`도 막혀 있으면 그 실행은 다시 `deleted`(또는 `not_owned`)를 세대 `g + 1`로
보고하고, 사용자가 다시 누른다. 한 번 누를 때 한 칸 — 화면에 보이는 것과
일어나는 일이 같다.

**멱등성은 유지된다.** 같은 `recreate_from: Some(g)`를 두 번 제출하면 두 번째는
`g + 1` 채널이 이미 있고 그 증명서도 읽히므로 평소 경로로 `no_change`가 된다.
세대가 또 올라가지 않는다 — 재생성은 `Some(g)`가 가리키는 세대가 **여전히
막혀 있을 때만** 한 칸 움직인다.

### 성공 이후는 스스로 일관된다

`g + 1` 채널에 증명서가 `generation: g + 1`로 쓰이고, 그 방의 멤버인 관리자는
그것을 읽는다. 다음 preflight부터는 `p.generation`이 그 값이므로 특별 취급이
사라진다. 재생성은 **한 번의 전이**이고 영구 상태가 아니다.

## 4. `not_owned`에서의 재생성 — 허용하되 기본이 아니다

여기가 이 설계의 유일한 위험 지점이다.

`not_owned`는 두 상황을 **구별하지 못한다.**

| 상황 | 옳은 조치 |
|---|---|
| 선점자가 도출 ID를 차지했다 | 세대를 올려 우리 방을 만든다 |
| 관리자 A가 만든 정상적인 방을 관리자 B가 재적용했다 | A에게 실행을 부탁한다. **세대를 올리면 표준 업무방이 둘이 된다** |

구별할 근거가 없다 — 둘 다 「이 방의 생성자가 내가 아니다」이고, 그것이
`created_by`가 말해 주는 전부다. 이름으로 추측하지 않는다(선점자가 catalog
이름을 그대로 쓸 수 있고, 정상 방의 이름은 팀이 바꿨을 수 있다).

**그래서 판정은 사람이 한다.** 규칙 셋:

1. `not_owned`의 기본 `user_action`은 지금처럼 `request_ownership`이다.
   화면의 1차 안내는 「저 사람에게 실행을 부탁하라」로 남는다.
2. 재생성은 **부차 동작**으로만 제공한다 — 기본 버튼이 아니고, 누르기 전에
   그 방을 만든 사람이 누구인지(`created_by` 축약형)와 "새 방이 하나 더
   생긴다"는 결과를 보여준다.
3. `deleted`와 달리 **되돌릴 수 없다는 점을 명시한다.** 잘못 눌러 만든 방은
   자동으로 정리되지 않는다.

`deleted`에는 이 위험이 없다. 그 판정은 「ID가 탔고 그 방에 접근할 수 없다」
이므로 세대를 올리는 것 말고 도달할 수 있는 상태가 없다.

## 5. 선점에 대해 이것이 하는 일과 하지 못하는 일

**하는 일:** 영구 차단을 푼다. 선점당한 항목을 관리자가 한 번의 확인으로
다음 세대에 만들 수 있다.

**하지 못하는 일:** 선점 자체를 막지 못한다. 도출식 입력이 전부 공개인 것은
그대로이므로, 공격자는 `g + 1`, `g + 2`를 미리 차지할 수 있다. 그러면 관리자는
누를 때마다 한 칸씩 밀리고 공격자는 그때마다 채널 하나를 만든다.

**그래서 이것은 "선점을 닫는다"가 아니라 "영구 차단을 유한한 경합으로
바꾼다"다.** 문서에 그렇게 적고, 그 이상으로 표현하지 않는다. 진짜로 닫으려면
도출식에 공격자가 예측할 수 없는 입력(예: 커뮤니티 생성 시 정해지는 비공개
salt)이 있어야 하고, 그건 이 설계의 범위가 아니다 — 기존에 적용된 모든 방의
ID가 바뀌므로 마이그레이션이 따로 필요하다. §8에 남긴다.

## 6. 와이어 변경

`selected: Vec<String>`이 `Vec<Selection>`이 된다. 경로는 다섯 곳뿐이다.

| 계층 | 파일 |
|---|---|
| saga | `crates/schoolx-catalog/src/saga.rs`의 `apply` |
| Tauri command | `desktop/src-tauri/src/commands/workspace_catalog.rs` |
| TS api | `desktop/src/shared/api/tauriWorkspaceCatalog.ts` |
| hook | `desktop/src/features/workspace-catalog/hooks.ts` |
| 카드 | `desktop/src/features/settings/ui/WorkspaceCatalogSettingsCard.tsx` |

**`Ledger`나 `Provenance`의 와이어 포맷은 바꾸지 않는다.** `generation`은 이미
둘 다에 있다. `steps`도 건드리지 않으므로 §4의 리더-우선 순서는 이 변경과
무관하다 — 세션 D2가 확인한 대로 `apply` 인자와 반환은 생산자와 소비자가 같은
빌드 안에 있다.

## 7. 검증 계획

| 검증 | 확인 대상 |
|---|---|
| 크레이트 단위 | `recreate_from: Some(1)`이 세대 2 채널을 만든다 |
| | 같은 재생성을 두 번 제출해도 세대가 한 번만 오른다 (`no_change`) |
| | 세대 2도 막혀 있으면 `deleted`를 **세대 2로** 보고한다 |
| | `recreate_from: None`은 지금 동작 그대로다 (기존 테스트 전부) |
| | 재생성이 이전 세대의 채널·캔버스·증명서를 건드리지 않는다 |
| 어댑터 단위 | `Selection`이 손실 없이 전달된다 |
| Playwright | `deleted` 항목에 재생성 컨트롤이 뜨고, 누르면 그 세대가 제출된다 |
| | `not_owned`의 기본은 `request_ownership`이고 재생성은 부차 동작이다 |
| 회귀 | `cargo test -p schoolx-catalog` 80개 유지, `e2e_access_matrix` 17/17 |

각 테스트는 목표한 버그를 재주입해 실패하는지 확인한 뒤 되돌린다. 세션 D2가
확인한 대로 **판정이 `no_change`로 끝나는 경로는 캔버스 단계에 도달하지
않으므로**, 캔버스 관련 단언을 이 세션의 방어선으로 세지 않는다.

## 8. 범위 밖

- **도출식에 비공개 salt를 넣어 선점 자체를 막는 것.** 기존 방의 ID가 전부
  바뀌므로 마이그레이션이 따로 필요하다. §5에 적은 대로 이 설계는 영구 차단만
  푼다.
- **잘못 만든 방의 정리.** `not_owned`에서 재생성을 눌러 생긴 여분의 방은
  수동으로 지운다. 자동 정리는 catalog가 사용자 채널을 지우는 경로를 만드는
  일이라 §8(WORKSPACE_CATALOG)의 「자동 채택·덮어쓰기·삭제하지 않는다」와
  충돌한다.
- **CLI 적용 경로.** 여전히 `buzz catalog list`뿐이다.
- **위임 실행 요청 흐름.** 「A에게 실행을 부탁한다」를 앱 안에서 보내는 것은
  별개 기능이다. 이 설계는 그 문구를 바꾸지 않는다.

## 9. 근거가 된 코드 경로

| 사실 | 경로 |
|---|---|
| 세대가 도출식 입력이다 | `crates/schoolx-catalog/src/channel_id.rs` |
| 신규 항목은 항상 세대 1 | `crates/schoolx-catalog/src/preflight.rs`의 `None` 분기 |
| `deleted` 판정과 `confirm_recreate` | `crates/schoolx-catalog/src/saga.rs` |
| `not_owned` 판정과 `request_ownership` | 같은 파일 |
| 삭제된 채널의 증명서를 읽을 수 없다 | `crates/buzz-db/src/channel.rs`의 `deleted_at IS NULL` |
| 선점 채널 증명서는 서명자 검증에서 버려진다 | `crates/schoolx-catalog/src/preflight.rs` |
| 적용 진입점 | `desktop/src-tauri/src/commands/workspace_catalog.rs`의 `apply_workspace_catalog` |
