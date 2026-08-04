# catalog 적용 권한 구현 계획 (세션 E1)

> **실행 완료 (2026-08-04).** Task 1–6 전부 실행했다. 게이트 결과는
> [`BASELINE.md`](../BASELINE.md) 세션 E1 절, 결과 서술은
> [`IMPLEMENTATION_HANDOFF.md`](../IMPLEMENTATION_HANDOFF.md) 세션 E1 절에
> 있다. **아래 두 곳은 계획대로 하지 않았다** — 계획서는 당시 판단의 기록으로
> 그대로 두고 무엇이 달랐는지만 여기 적는다.
>
> 1. **Task 2·3의 근거가 실행 중에 뒤집혔다.** 「`is_owner`에서 `admin`을
>    뗀다」의 근거였던 *"`owner`는 채널 생성자에게 고정되어 남이 줄 수 없다"*
>    가 사실이 아니었다 — `MemberRole::Owner`는 부여 가능한 값이고 개수 상한도
>    없어서, 선점자가 `admin` 대신 `owner`를 주면 같은 공격이 그대로
>    성립했다. 그래서 판정 근거를 역할이 아니라 불변 생성자
>    (`channels.created_by`, relay가 kind:39000에 싣는 `created_by` 태그)로
>    옮겼다(커밋 `4c8be34b`·`14925137`). Task 3 Step 2·3의 코드 스니펫은
>    이 정정 이전 모양이다. 정정 이력은
>    [`CATALOG_SECURITY.md`](../CATALOG_SECURITY.md) §5·§6.
> 2. **Task 6 Step 1의 E2E가 계획보다 넓다.** 계획은 provenance 이벤트의
>    `pubkey`만 단언했는데, 위 정정으로 판정이 실제로 읽는 값이 kind:39000의
>    `created_by`가 되었다. 그래서 신규 테스트는 선점자가 피해 관리자에게
>    **`owner`를 준 뒤에도** 두 값이 모두 선점자를 가리킨다는 것을 함께
>    고정한다.
>
> 계획에 없던 것도 하나 있다. 오픈 릴레이(`require_relay_membership=false`,
> 기본값)에서는 kind 13534가 아예 없어 명부 조회가 항상 비므로, 그것을 권한
> 거부로 읽으면 커뮤니티 소유자 본인이 잠긴다. 통과시키지도 않는다 — 구별되는
> 식별자 `catalog-membership-unavailable`로 거부한다(커밋 `f3dc2137`).

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** catalog 적용을 커뮤니티 관리자로 제한하고, provenance를 그 채널 owner가 서명한 것만 인정하게 한다.

**Architecture:** 세션 D가 provenance를 도출된 채널에 묶은 위에 서명자 조건을 더한다. 관리자 게이트는 relay-signed 커뮤니티 역할(`get_my_relay_membership`)을 근거로 Tauri command 진입에서 강제한다. relay는 건드리지 않는다.

**Tech Stack:** Rust 1.88 / `schoolx-catalog` 크레이트 / Tauri 2 / React 19

## Global Constraints

- 작업 위치는 **메인 체크아웃** `/Users/kim-yonghun/Development/schoolX_v2.0`, 브랜치 `codex/schoolx-2-foundation`. 워크트리에서는 `just desktop-tauri-fmt`가 실패해 pre-commit이 막힌다.
- 시작 전 `. ./bin/activate-hermit`.
- `unsafe` 금지. 프로덕션 경로에 새 `unwrap()`/`expect()` 금지 — `?`와 에러 타입을 쓴다.
- 새 public API에는 doc comment를 단다.
- 데스크톱 텍스트 크기는 rem 토큰만 (`text-base`, `text-sm`, `text-xs`, `text-2xs`). 임의 리터럴은 `pnpm check:px-text`가 막는다.
- i18n 키를 더할 때는 `en`, `ko`를 **한 번에** 바꾼다. 한쪽만 바꾸면 `fallbackLng`가 구제하지 못하고 한국어에 원시 키가 노출된다.
- `steps`에 값을 더하지 않는다. 이 계획은 와이어 포맷을 바꾸지 않는다.
- 파일 1000줄 상한. 걸리면 한계를 올리지 말고 줄인다.
- 스펙: [`docs/schoolx-2/CATALOG_SECURITY.md`](../CATALOG_SECURITY.md) (커밋 `266f63a4`).

---

## File Structure

| 파일 | 책임 |
|---|---|
| `crates/schoolx-catalog/src/effects.rs` | `fetch_provenance` 반환에 서명자 추가, fake 갱신 |
| `crates/schoolx-catalog/src/preflight.rs` | owner 아닌 서명자의 provenance 폐기 |
| `desktop/src-tauri/src/commands/workspace_catalog.rs` | 서명자 전달, `is_owner`에서 admin 제거, 관리자 게이트 |
| `desktop/src/features/settings/ui/SettingsPanels.tsx` | 섹션에 `featureGate` |
| `desktop/src/shared/i18n/locales/{en,ko}.ts` | 게이트 실패 문구 |
| `desktop/src/features/settings/ui/WorkspaceCatalogSettingsCard.tsx` | 권한 없음 상태 표시 |
| `crates/buzz-test-client/tests/e2e_workspace_catalog.rs` | 비관리자 차단 E2E |

---

## Task 1: provenance에 서명자를 실어 나른다

**Files:**
- Modify: `crates/schoolx-catalog/src/effects.rs`
- Modify: `crates/schoolx-catalog/src/preflight.rs`

**Interfaces:**
- Consumes: 현재 `fetch_provenance(&self, catalog_id: &str) -> Result<Vec<(Uuid, Provenance)>, EffectError>`
- Produces: `fetch_provenance(&self, catalog_id: &str) -> Result<Vec<ProvenanceRecord>, EffectError>` where
  ```rust
  pub struct ProvenanceRecord {
      pub channel_id: Uuid,
      pub signer: String,
      pub provenance: Provenance,
  }
  ```

튜플이 3개가 되는 순간 위치 인자가 읽히지 않는다. 이름 있는 구조체로 바꾼다.

- [x] **Step 1: 실패하는 테스트를 쓴다**

`crates/schoolx-catalog/src/effects.rs`의 기존 `#[cfg(test)] mod fake` 아래 테스트 모듈이 없으면 파일 끝에 만든다.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_record_carries_channel_and_signer() {
        let record = ProvenanceRecord {
            channel_id: Uuid::nil(),
            signer: "abc123".into(),
            provenance: Provenance {
                catalog_id: "schoolx.default".into(),
                catalog_version: 1,
                item_key: "meeting".into(),
                generation: 1,
                steps: crate::provenance::StepStates::default(),
                applied_at: "2026-08-01T00:00:00Z".into(),
            },
        };
        assert_eq!(record.channel_id, Uuid::nil());
        assert_eq!(record.signer, "abc123");
        assert_eq!(record.provenance.item_key, "meeting");
    }
}
```

- [x] **Step 2: 테스트가 실패하는지 확인한다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && cargo test -p schoolx-catalog provenance_record_carries`
Expected: FAIL — `ProvenanceRecord` 가 없어 컴파일 에러

- [x] **Step 3: 타입을 만들고 trait을 바꾼다**

`effects.rs`의 `CatalogEffects` trait 위에 추가한다.

```rust
/// relay에서 읽어 온 provenance 한 건과, 그것을 검증하는 데 필요한 맥락.
///
/// 내용만으로는 신뢰할 수 없다 — 어느 채널에 실려 있었는지(`channel_id`)와
/// 누가 서명했는지(`signer`)가 있어야 §5의 두 조건을 검사할 수 있다.
/// 튜플로 두면 세 값의 순서를 호출부가 외워야 하므로 이름을 붙인다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenanceRecord {
    /// 이 이벤트가 실려 있던 채널 (`h` 태그).
    pub channel_id: Uuid,
    /// 이 이벤트에 서명한 pubkey (hex).
    pub signer: String,
    /// 이벤트 content.
    pub provenance: Provenance,
}
```

`CatalogEffects::fetch_provenance`의 반환 타입을 `Result<Vec<ProvenanceRecord>, EffectError>`로 바꾸고, doc comment에 한 줄 더한다.

```rust
    /// 읽을 수 있는 이 catalog의 provenance 이벤트 전부.
    ///
    /// 채널 스코프라 비멤버인 항목은 결과에 나타나지 않는다. 이건 버그가
    /// 아니라 보안 계약이다.
    ///
    /// 각 레코드는 실려 있던 채널과 서명자를 함께 담는다. 둘 다
    /// `preflight`가 §5의 검증에 쓴다 — 채널 결합만으로는 그 채널을 선점한
    /// 사람이 발행한 증명서를 걸러내지 못한다.
    async fn fetch_provenance(
        &self,
        catalog_id: &str,
    ) -> Result<Vec<ProvenanceRecord>, EffectError>;
```

- [x] **Step 4: fake를 맞춘다**

`mod fake` 안에서 `provenance` 필드의 타입을 바꾼다.

```rust
        /// `(channel_id, signer, provenance)`. 실제 relay에서 provenance
        /// 이벤트는 채널 스코프이고 서명자가 있다. 셋을 분리해 두면 테스트가
        /// "채널은 맞는데 서명자가 다른" 상태를 만들 수 있다 — 그게 선점
        /// 공격의 모양이다.
        pub provenance: Mutex<Vec<(Uuid, String, Provenance)>>,
```

`fetch_provenance` 구현을 맞춘다.

```rust
        async fn fetch_provenance(
            &self,
            catalog_id: &str,
        ) -> Result<Vec<ProvenanceRecord>, EffectError> {
            self.take_failure("fetch_provenance")?;
            let live: HashSet<Uuid> = self
                .channels
                .lock()
                .expect("lock")
                .iter()
                .map(|c| c.id)
                .collect();
            Ok(self
                .provenance
                .lock()
                .expect("lock")
                .iter()
                .filter(|(channel_id, _, p)| {
                    p.catalog_id == catalog_id && live.contains(channel_id)
                })
                .map(|(channel_id, signer, provenance)| ProvenanceRecord {
                    channel_id: *channel_id,
                    signer: signer.clone(),
                    provenance: provenance.clone(),
                })
                .collect())
        }
```

`publish_provenance`가 `provenance` 저장소에 넣는 부분도 3-튜플로 맞춘다. 서명자는 fake의 「나」를 뜻하는 상수를 쓴다 — `mod fake` 위쪽에 추가한다.

```rust
    /// fake에서 「현재 사용자」의 pubkey. 실제 값은 의미 없고, 테스트가
    /// 「나」와 「남」을 구별할 수 있으면 된다.
    pub(crate) const FAKE_ME: &str = "me";
```

`publish_provenance`의 retain/push를 이렇게 바꾼다.

```rust
            let mut store = self.provenance.lock().expect("lock");
            store.retain(|(_, _, p)| p.d_tag() != provenance.d_tag());
            store.push((channel_id, FAKE_ME.to_string(), provenance.clone()));
```

`seed_canvas`와 나란히 시딩 헬퍼를 더한다.

```rust
        /// 이전 실행이 남긴 증명서를 심는다. `signer`로 「내가 남긴 것」과
        /// 「남이 남긴 것」을 구별한다.
        pub(crate) fn seed_provenance(
            &self,
            channel_id: Uuid,
            signer: &str,
            provenance: Provenance,
        ) {
            self.provenance
                .lock()
                .expect("lock")
                .push((channel_id, signer.to_string(), provenance));
        }
```

- [x] **Step 5: preflight의 호출부를 맞춘다**

`preflight.rs`에서 `fetch_provenance`의 결과를 쓰는 곳을 새 타입에 맞춘다. 이 단계에서는 **동작을 바꾸지 않는다** — 튜플 분해를 필드 접근으로 바꾸기만 한다. 서명자 검사는 Task 2에서 더한다.

찾아야 할 것은 두 가지다. `(channel_id, p)` 또는 `(_, p)` 형태로 구조 분해하는 자리를 `record.channel_id` / `record.provenance`로 바꾼다. 그리고 `derive_channel_id`가 예측하는 채널과 대조하는 세션 D의 검사 — 거기서 튜플의 첫 요소를 쓰던 것이 `record.channel_id`가 된다. 그 검사 자체는 지우지 않는다.

기존 테스트의 `seed_applied` 계열 헬퍼도 `seed_provenance(channel_id, FAKE_ME, ...)`를 쓰도록 바꾼다. 기존 테스트의 의미는 그대로다 — 지금까지의 모든 시딩은 「내가 남긴 증명서」였다.

**컴파일러가 남은 자리를 전부 알려준다.** 반환 타입이 바뀌었으므로 고치지 않은 호출부는 빌드가 실패한다. 경고를 무시하고 넘어갈 수 있는 자리가 없다.

- [x] **Step 6: 테스트가 통과하는지 확인한다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && cargo test -p schoolx-catalog`
Expected: PASS — 기존 테스트 전부 + 새 테스트 1개. 동작은 아직 안 바뀌었으므로 어떤 기존 단언도 깨지면 안 된다.

- [x] **Step 7: 커밋한다**

```bash
git add crates/schoolx-catalog
git commit -m "refactor(schoolx-2): 세션 E1 — provenance 레코드에 서명자를 싣는다"
```

---

## Task 2: owner 아닌 서명자의 provenance를 버린다

**Files:**
- Modify: `crates/schoolx-catalog/src/effects.rs`
- Modify: `crates/schoolx-catalog/src/preflight.rs`

**Interfaces:**
- Consumes: `ProvenanceRecord`, `FakeEffects::seed_provenance`
- Produces: `CatalogEffects::channel_owner(&self, channel_id: Uuid) -> Result<Option<String>, EffectError>`

`is_owner`는 「내가 owner인가」만 답한다. 여기서 필요한 것은 「이 채널의 owner가 **누구인가**」다.

- [x] **Step 1: 실패하는 테스트를 쓴다**

`preflight.rs`의 테스트 모듈에 추가한다.

```rust
    #[tokio::test]
    async fn provenance_signed_by_a_non_owner_is_ignored() {
        let fx = FakeEffects::new();
        let channel_id =
            derive_channel_id("wss://relay.test", "schoolx.default", "meeting", 1);
        // 채널은 존재하고 owner는 나다.
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: channel_id,
            name: "메인 회의방".into(),
        });
        fx.owned.lock().expect("lock").insert(channel_id);
        // 그런데 증명서는 다른 사람이 서명했다.
        fx.seed_provenance(
            channel_id,
            "someone-else",
            provenance_with_generation("meeting", 1, done()),
        );

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        // 그 증명서는 없는 것으로 친다 — 이름이 catalog 값 그대로이므로
        // 동명 충돌로 떨어진다.
        assert_eq!(find(&items, "meeting").decision, Decision::Conflict);
    }

    #[tokio::test]
    async fn provenance_signed_by_the_owner_is_honoured() {
        let fx = FakeEffects::new();
        let channel_id =
            derive_channel_id("wss://relay.test", "schoolx.default", "meeting", 1);
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: channel_id,
            name: "메인 회의방".into(),
        });
        fx.owned.lock().expect("lock").insert(channel_id);
        fx.set_channel_owner(channel_id, "owner-pubkey");
        fx.seed_provenance(
            channel_id,
            "owner-pubkey",
            provenance_with_generation("meeting", 1, done()),
        );

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        assert_eq!(find(&items, "meeting").decision, Decision::NoChange);
    }
```

- [x] **Step 2: 테스트가 실패하는지 확인한다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && cargo test -p schoolx-catalog provenance_signed_by`
Expected: FAIL — `set_channel_owner`가 없어 컴파일 에러

- [x] **Step 3: trait에 owner 조회를 더한다**

`effects.rs`의 `CatalogEffects`에 추가한다.

```rust
    /// 이 채널의 owner pubkey. owner를 알 수 없으면 `None`.
    ///
    /// `is_owner`와 다르다. 저쪽은 「내가 owner인가」이고 이쪽은 「owner가
    /// 누구인가」다. provenance 검증은 후자를 필요로 한다 — 증명서를 남긴
    /// 사람이 그 채널의 owner였는지 물어야 하기 때문이다.
    ///
    /// `Ok(None)`은 「채널은 있는데 owner를 특정할 수 없다」이지 오류가
    /// 아니다. 그 경우 그 채널의 증명서는 전부 버린다 — 검증할 수 없는 것을
    /// 통과시키지 않는다.
    async fn channel_owner(&self, channel_id: Uuid) -> Result<Option<String>, EffectError>;
```

- [x] **Step 4: fake에 owner 저장소와 시더를 더한다**

`FakeEffects`에 필드를 더한다.

```rust
        /// 채널별 owner pubkey. `owned`(내가 owner인 채널)와 분리한다 —
        /// 「내가 owner다」와 「owner가 누구다」는 다른 질문이고, 선점 공격은
        /// 그 차이에서 산다.
        pub owners: Mutex<HashMap<Uuid, String>>,
```

시더와 구현을 더한다.

```rust
        /// 이 채널의 owner를 지정한다.
        pub(crate) fn set_channel_owner(&self, channel_id: Uuid, pubkey: &str) {
            self.owners
                .lock()
                .expect("lock")
                .insert(channel_id, pubkey.to_string());
        }
```

```rust
        async fn channel_owner(&self, channel_id: Uuid) -> Result<Option<String>, EffectError> {
            self.take_failure("channel_owner")?;
            Ok(self.owners.lock().expect("lock").get(&channel_id).cloned())
        }
```

`create_channel`이 성공할 때 `owners`에도 `FAKE_ME`를 넣는다 — 만든 사람이 owner이므로.

- [x] **Step 5: preflight에서 검사한다**

`preflight.rs`에서 provenance를 쓰기 전에 서명자를 확인한다.

```rust
    // §5: 증명서는 (1) 도출식이 예측하는 채널에 실려 있고 (2) 그 채널의
    // 현재 owner가 서명한 것만 인정한다. (1)만으로는 그 채널을 선점한
    // 사람이 자기 채널 안에서 발행한 증명서를 거르지 못한다 — 정말 그
    // 채널에 있기 때문이다.
    //
    // owner를 특정할 수 없으면 버린다. 검증할 수 없는 것을 통과시키지
    // 않는다.
    let mut honoured: Vec<&ProvenanceRecord> = Vec::new();
    for record in &provenance {
        match effects.channel_owner(record.channel_id).await? {
            Some(owner) if owner == record.signer => honoured.push(record),
            _ => {}
        }
    }
```

이후 판정 로직은 `provenance` 대신 `honoured`를 본다. 채널 결합 검사(세션 D)는 그대로 둔다 — 두 조건은 각자 다른 공격을 막으므로 하나가 다른 하나를 대신하지 않는다.

- [x] **Step 6: 테스트가 통과하는지 확인한다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && cargo test -p schoolx-catalog`
Expected: PASS — 새 테스트 2개 포함 전부

기존 테스트가 깨지면 그 테스트의 시딩이 owner를 지정하지 않아서다. `set_channel_owner(channel_id, FAKE_ME)`를 더한다 — 지금까지의 모든 시딩은 「내가 만들고 내가 남긴」 상태를 뜻했다.

- [x] **Step 7: 판별력을 실증한다**

`preflight.rs`에서 서명자 검사를 임시로 지우고 (`honoured`를 `provenance` 전체로) 테스트를 돌려 `provenance_signed_by_a_non_owner_is_ignored`가 실패하는지 확인한 뒤 정확히 되돌린다. 보고서에 기록한다.

- [x] **Step 8: 커밋한다**

```bash
git add crates/schoolx-catalog
git commit -m "fix(schoolx-2): 세션 E1 — 채널 owner가 서명한 증명서만 인정한다"
```

---

## Task 3: 어댑터가 서명자와 owner를 넘긴다

**Files:**
- Modify: `desktop/src-tauri/src/commands/workspace_catalog.rs`

**Interfaces:**
- Consumes: `ProvenanceRecord`, `CatalogEffects::channel_owner`
- Produces: 없음 (기존 command 시그니처 유지)

- [x] **Step 1: `fetch_provenance`가 서명자를 싣게 한다**

이 어댑터는 이미 `h` 태그를 뽑고 있다. 같은 자리에서 `ev.pubkey`를 함께 담는다. 이벤트에서 `h` 태그를 뽑는 기존 헬퍼를 찾아 그 반환에 서명자를 더하고, `ProvenanceRecord`를 만들어 돌려준다. 서명자는 `ev.pubkey.to_hex()`다.

`h` 태그가 없거나 파싱되지 않는 이벤트는 지금처럼 버린다.

- [x] **Step 2: `channel_owner`를 구현한다**

`is_owner`가 이미 `get_channel_members`를 부른다. 같은 응답에서 `role == "owner"`인 첫 멤버의 pubkey를 돌려준다.

```rust
    async fn channel_owner(&self, channel_id: Uuid) -> Result<Option<String>, EffectError> {
        let response = crate::commands::channels::get_channel_members(
            channel_id.to_string(),
            self.state.clone(),
        )
        .await
        .map_err(EffectError)?;
        Ok(response
            .members
            .iter()
            .find(|m| m.role == "owner")
            .map(|m| m.pubkey.clone()))
    }
```

- [x] **Step 3: `is_owner`에서 admin을 뗀다**

현재 구현은 `m.role == "owner" || m.role == "admin"`이다. `owner`만 남기고, 왜 바뀌었는지 주석으로 남긴다.

```rust
    async fn is_owner(&self, channel_id: Uuid) -> Result<bool, EffectError> {
        let keys = self.state.signing_keys().map_err(EffectError)?;
        let me = keys.public_key().to_hex();
        let response = crate::commands::channels::get_channel_members(
            channel_id.to_string(),
            self.state.clone(),
        )
        .await
        .map_err(EffectError)?;
        // `owner`만 받는다. relay 전반의 쓰기 권한 판정은 `admin`도 같은
        // 등급으로 보지만, 채택이 묻는 것은 「여기서 쓸 수 있는가」가 아니라
        // 「이 방이 우리 것인가」다. `admin`은 남이 줄 수 있고 수신자 동의도
        // 필요 없으므로, 도출 ID를 선점한 사람이 피해자에게 `admin`을 주는
        // 것만으로 이 게이트를 통과시킬 수 있다. `owner`는 생성자에게
        // 고정되어 그럴 수 없다. 설계 근거: docs/schoolx-2/CATALOG_SECURITY.md §6.
        Ok(response
            .members
            .iter()
            .any(|m| m.pubkey == me && m.role == "owner"))
    }
```

- [x] **Step 4: 검증한다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && cargo test --manifest-path desktop/src-tauri/Cargo.toml`
Expected: PASS

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && just desktop-tauri-fmt && cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets -- -D warnings`
Expected: 둘 다 exit 0

포그라운드로 돌린다. 데스크톱 빌드는 10~15분 걸린다 — 백그라운드로 던지고 알림을 기다리지 않는다.

- [x] **Step 5: 커밋한다**

```bash
git add desktop/src-tauri
git commit -m "fix(schoolx-2): 세션 E1 — 채택 게이트에서 admin을 떼고 서명자를 넘긴다"
```

---

## Task 4: 관리자 게이트

**Files:**
- Modify: `desktop/src-tauri/src/commands/workspace_catalog.rs`

**Interfaces:**
- Consumes: `crate::commands::relay_members::get_my_relay_membership`
- Produces: 두 command가 관리자가 아니면 실패

- [x] **Step 1: 게이트 함수를 쓴다**

`get_my_relay_membership`은 `{"member": {...}}` 또는 `{"member": null}`을 돌려준다. 멤버 객체에 `role` 필드가 있다.

```rust
/// catalog 적용은 커뮤니티 관리자만 할 수 있다.
///
/// 근거가 되는 역할은 relay가 서명한 kind 13534 목록에서 온다 —
/// 클라이언트가 만드는 값이 아니라 위조할 수 없다. 채널 레벨 역할이 아니라
/// 커뮤니티 레벨 역할을 본다: 이 동작은 커뮤니티 전체에 기본 업무방을
/// 만드는 일이기 때문이다.
///
/// preflight도 막는다. 미리보기만으로도 어떤 항목이 이미 적용됐는지가
/// 드러나고 그것은 private 채널의 존재 정보다.
///
/// 이 게이트는 클라이언트 측이다. 직접 relay에 채널 생성 이벤트를 쏘는
/// 것은 막지 못하며 막으려는 대상도 아니다 — 채널을 만드는 것은 모든
/// 구성원의 정상 권한이고, 여기서 막는 것은 「catalog 적용으로 기본 업무방
/// 일습을 만드는 것」이다. 설계 근거: docs/schoolx-2/CATALOG_SECURITY.md §3·§4.
async fn require_community_admin(state: &State<'_, AppState>) -> Result<(), String> {
    let membership =
        crate::commands::relay_members::get_my_relay_membership(state.clone()).await?;
    let role = membership
        .get("member")
        .and_then(|m| m.get("role"))
        .and_then(|r| r.as_str());
    if role_may_apply(role) {
        Ok(())
    } else {
        Err("catalog-admin-required".to_string())
    }
}

/// 이 커뮤니티 역할이 catalog를 적용할 수 있는가.
///
/// relay I/O에서 분리해 두어 판정만 단위 테스트할 수 있게 한다. 모르는
/// 역할은 거부한다 — 나중에 역할이 추가되어도 자동으로 권한을 얻지 않아야
/// 한다.
fn role_may_apply(role: Option<&str>) -> bool {
    matches!(role, Some("owner") | Some("admin"))
}
```

에러 문자열은 **문구가 아니라 식별자**다. 사용자에게 보일 문장은 프론트엔드가 지역화한다 — 어댑터의 한국어 하드코딩이 영어 로케일 사용자에게 새는 기존 문제를 되풀이하지 않는다.

- [x] **Step 2: 두 command에 건다**

`preflight_workspace_catalog`와 `apply_workspace_catalog` 각각의 첫 줄에서 부른다.

```rust
    require_community_admin(&state).await?;
```

게이트가 실패하면 `?`가 즉시 반환하므로 어떤 부분 결과도 나가지 않는다.

- [x] **Step 3: 테스트를 더한다**

같은 파일의 테스트 모듈에 역할 판정만 떼어 검사하는 테스트를 더한다. `get_my_relay_membership`은 relay를 타므로 여기서는 판정 부분을 순수 함수로 분리해 시험한다 — `require_community_admin`에서 role 문자열을 받아 판정하는 부분을 `fn role_may_apply(role: Option<&str>) -> bool`로 빼고 그것을 시험한다.

```rust
    #[test]
    fn only_community_owner_and_admin_may_apply() {
        assert!(role_may_apply(Some("owner")));
        assert!(role_may_apply(Some("admin")));
        assert!(!role_may_apply(Some("member")));
        assert!(!role_may_apply(None));
        // 모르는 역할이 생기면 거부한다 — 새 역할이 자동으로 권한을 얻지
        // 않아야 한다.
        assert!(!role_may_apply(Some("guest")));
    }
```

- [x] **Step 4: 검증한다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && cargo test --manifest-path desktop/src-tauri/Cargo.toml workspace_catalog`
Expected: PASS

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && just desktop-tauri-fmt && cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets -- -D warnings`
Expected: 둘 다 exit 0

- [x] **Step 5: 커밋한다**

```bash
git add desktop/src-tauri
git commit -m "feat(schoolx-2): 세션 E1 — catalog 적용을 커뮤니티 관리자로 제한한다"
```

---

## Task 5: 화면이 권한 없음을 설명한다

**Files:**
- Modify: `desktop/src/features/settings/ui/SettingsPanels.tsx`
- Modify: `desktop/src/shared/i18n/locales/en.ts`
- Modify: `desktop/src/shared/i18n/locales/ko.ts`
- Modify: `desktop/src/features/settings/ui/WorkspaceCatalogSettingsCard.tsx`

- [x] **Step 1: i18n 키를 양쪽에 더한다**

`en.ts`의 `catalog` 블록에 추가한다.

```ts
    adminRequired:
      "Only a community owner or admin can apply the default workspace. Ask an administrator to run it.",
```

`ko.ts`의 `catalog` 블록에 같은 자리에 추가한다.

```ts
    adminRequired:
      "기본 워크스페이스는 커뮤니티 소유자나 관리자만 적용할 수 있습니다. 관리자에게 요청하세요.",
```

`catalog`는 이미 등록된 네임스페이스이므로 `APP_I18N_NAMESPACES`는 바꾸지 않는다.

- [x] **Step 2: 섹션에 featureGate를 건다**

`SettingsPanels.tsx`의 `workspace-catalog` 서술자에 추가한다.

```tsx
    featureGate: "workspace-catalog",
```

이건 메뉴를 숨길 뿐 보안이 아니다 — 실제 게이트는 Task 4의 command 쪽이다. 그 사실을 서술자 근처 주석으로 남긴다.

- [x] **Step 3: 카드가 게이트 실패를 설명한다**

`WorkspaceCatalogSettingsCard.tsx`에서 preflight 쿼리의 에러가 `catalog-admin-required`이면 `t("catalog.adminRequired")`를 띄우고 항목 목록과 적용 버튼을 감춘다. 다른 에러는 지금처럼 원문을 보여준다.

```tsx
const isAdminRequired = (error: unknown) =>
  error instanceof Error && error.message.includes("catalog-admin-required");
```

경고 스타일은 같은 파일의 `user_action` alert가 쓰는 amber 처리(`border-amber-500/30 bg-amber-500/10`)를 그대로 쓴다. 텍스트 크기는 rem 토큰만 쓴다.

- [x] **Step 4: 검증한다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && pnpm --dir desktop typecheck && pnpm --dir desktop check && pnpm --dir desktop test`
Expected: 전부 PASS. i18n parity 테스트가 `en`/`ko` 키 구조 일치를 확인한다.

포그라운드로 돌린다.

- [x] **Step 5: 커밋한다**

```bash
git add desktop/src
git commit -m "feat(schoolx-2): 세션 E1 — 권한이 없으면 카드가 이유를 설명한다"
```

---

## Task 6: live relay E2E와 최종 게이트

**Files:**
- Modify: `crates/buzz-test-client/tests/e2e_workspace_catalog.rs`
- Modify: `docs/schoolx-2/IMPLEMENTATION_HANDOFF.md`
- Modify: `docs/schoolx-2/SECURITY_CONTRACT.md`
- Modify: `docs/schoolx-2/BASELINE.md`

- [x] **Step 1: 선점 시나리오 E2E를 더한다**

`e2e_workspace_catalog.rs`에 테스트를 더한다. 기존 하네스를 그대로 재사용한다 — 새 하네스를 만들지 않는다.

`squatted_channel_provenance_is_signed_by_the_squatter`: 사용자 A가 채널을 만들고(그러면 A가 owner) 그 안에 kind 39500을 발행한다. 사용자 B를 그 채널의 멤버로 추가한다. B가 같은 필터로 읽으면 이벤트가 보이고 **그 이벤트의 pubkey가 A**임을 단언한다. 이것이 어댑터가 서명자를 뽑을 수 있다는 사실과, 그 서명자가 B가 아니라는 사실을 relay 수준에서 고정한다.

이 테스트는 relay 동작만 검증한다. 「그래서 B의 preflight가 그걸 버린다」는 크레이트 단위 테스트가 이미 덮는다 — 여기서 saga를 부르지 않는다.

- [x] **Step 2: E2E를 돌린다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && just test-e2e e2e_workspace_catalog`
Expected: PASS

포그라운드로 돌린다. Postgres·Redis·MinIO가 필요하고 `just test-e2e`가 띄운다. 포트 3000이 다른 프로세스에 잡혀 relay가 bind에 실패하면, 그 프로세스를 죽이지 말고 `BUZZ_BIND_ADDR`와 `RELAY_URL`을 **함께** 빈 포트로 덮어 같은 순서를 다시 돌린 뒤 무엇을 돌렸는지 정확히 적는다.

- [x] **Step 3: 보안 계약 회귀를 확인한다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && just test-e2e e2e_access_matrix`
Expected: PASS — 17/17

이 계약에는 CI job이 없어 사람이 돌릴 때만 돈다.

- [x] **Step 4: 문서를 갱신한다**

`SECURITY_CONTRACT.md` §5에 세션 D가 기록한 두 gap을 **닫힌 것으로** 바꾼다. 무엇이 닫혔고 무엇이 남았는지 정확히 적는다 — 클라이언트 측 게이트라 직접 relay 호출은 여전히 막지 못한다는 사실(스펙 §4)이 남는 조건이다.

`IMPLEMENTATION_HANDOFF.md`의 세션 E 범위에서 이 두 항목을 완료로 옮기고, E2·E3가 남았음을 분명히 한다. 세션 A·B·D가 쓴 형식을 따른다.

`BASELINE.md`에 이번에 돌린 명령과 결과, 실행 시각을 기록한다.

- [x] **Step 5: 전체 게이트를 돌린다**

`just ci`는 하네스의 10분 제한에 걸리므로 구성 레시피를 하나씩 포그라운드로 돌린다: `fmt-check`, `clippy`, `desktop-check`, `desktop-tauri-fmt-check`, `desktop-tauri-clippy`, `web-check`, `mobile-check`, `test-unit`, `desktop-test`, `desktop-build`, `desktop-tauri-check`, `desktop-tauri-test`, `web-build`, `mobile-test`.

각각의 결과를 보고서에 적는다. 실패하면 원인이 이번 변경인지 기존 조건인지 판별해 밝힌다.

- [x] **Step 6: 제품 식별자 검사를 돌린다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && just schoolx-upstream-check`
Expected: 3/3 PASS

- [x] **Step 7: 커밋한다**

```bash
git add crates/buzz-test-client docs/schoolx-2
git commit -m "docs(schoolx-2): 세션 E1 — catalog 권한 구현 결과 기록"
```
