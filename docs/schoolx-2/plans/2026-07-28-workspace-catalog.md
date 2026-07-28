# 워크스페이스 catalog 구현 계획 (세션 D)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 관리자가 선택한 SchoolX 기본 업무방(메인 회의방·기획)을 재현 가능하고 복구 가능하게 적용한다.

**Architecture:** 새 워크스페이스 크레이트 `schoolx-catalog`가 읽기 전용 catalog, preflight 판정, idempotent saga, result ledger를 담는다. relay I/O는 `CatalogEffects` trait로 주입해 fault injection을 live relay 없이 검증한다. provenance는 새 Nostr kind 39500(채널 스코프 addressable)으로 relay에 남는다.

**Tech Stack:** Rust 1.88 / serde / uuid v5 / async-trait / Tauri 2 / React 19 / i18next

## Global Constraints

- 작업 위치는 **메인 체크아웃** `/Users/kim-yonghun/Development/schoolX_v2.0`, 브랜치 `codex/schoolx-2-foundation`. 워크트리에서는 `just desktop-tauri-fmt`가 실패해 pre-commit이 막힌다.
- 시작 전 `. ./bin/activate-hermit`.
- `unsafe` 금지. 프로덕션 경로에 새 `unwrap()`/`expect()` 금지 — `?`와 에러 타입을 쓴다.
- 새 public API에는 doc comment를 단다.
- SchoolX 전용 Nostr kind 예약 대역은 **39500–39599**. SQL 마이그레이션 예약 대역은 `9001+`.
- 데스크톱 텍스트 크기는 rem 토큰만 — `text-[13px]` 같은 임의 리터럴은 CI(`pnpm check:px-text`)가 막는다.
- i18n 네임스페이스를 추가할 때는 `en`, `ko`, `APP_I18N_NAMESPACES` **세 곳을 한 번에** 바꾼다. 한쪽만 바꾸면 한국어에서 원시 키가 노출된다.
- 채널 공개 범위 기본값은 `private`. `open`으로 바꾸는 UI에는 §9의 두 문장을 반드시 띄운다.
- 스펙: [`docs/schoolx-2/WORKSPACE_CATALOG.md`](../WORKSPACE_CATALOG.md) (커밋 `0244560d`).

---

## File Structure

| 파일 | 책임 |
|---|---|
| `crates/schoolx-catalog/Cargo.toml` | 크레이트 매니페스트 |
| `crates/schoolx-catalog/catalog.json` | 읽기 전용 내장 정의 |
| `crates/schoolx-catalog/src/lib.rs` | 모듈 선언과 re-export |
| `crates/schoolx-catalog/src/catalog.rs` | `Catalog`, `CatalogItem`, `builtin()` |
| `crates/schoolx-catalog/src/channel_id.rs` | 결정론적 채널 ID 도출 |
| `crates/schoolx-catalog/src/provenance.rs` | kind 39500 표현과 직렬화 |
| `crates/schoolx-catalog/src/effects.rs` | `CatalogEffects` trait와 fake |
| `crates/schoolx-catalog/src/preflight.rs` | 판정 로직 |
| `crates/schoolx-catalog/src/saga.rs` | 단계 실행기 |
| `crates/schoolx-catalog/src/ledger.rs` | machine-readable 결과 |
| `crates/buzz-core/src/kind.rs` | kind 39500 상수와 예약 대역 |
| `crates/buzz-relay/src/handlers/ingest.rs` | scope 매핑과 h-tag 게이트 |
| `desktop/src-tauri/src/commands/workspace_catalog.rs` | effects 구현 + Tauri command |
| `desktop/src/features/workspace-catalog/` | 미리보기·적용·결과 UI |
| `crates/buzz-cli/src/commands/workspace_catalog.rs` | 읽기 전용 CLI |
| `crates/buzz-test-client/tests/e2e_workspace_catalog.rs` | live relay E2E |

---

## Task 1: `schoolx-catalog` 크레이트와 내장 catalog

**Files:**
- Create: `crates/schoolx-catalog/Cargo.toml`
- Create: `crates/schoolx-catalog/catalog.json`
- Create: `crates/schoolx-catalog/src/lib.rs`
- Create: `crates/schoolx-catalog/src/catalog.rs`
- Modify: `Cargo.toml` (workspace members)

**Interfaces:**
- Consumes: 없음
- Produces: `schoolx_catalog::catalog::{Catalog, CatalogItem, Visibility, builtin}`
  - `pub fn builtin() -> &'static Catalog`
  - `Catalog { catalog_id: String, catalog_version: u32, items: Vec<CatalogItem> }`
  - `CatalogItem { item_key: String, name: String, description: String, channel_type: String, visibility: Visibility, canvas: String }`
  - `Visibility` = `Private | Open`, `as_str() -> &'static str`

- [ ] **Step 1: 워크스페이스에 크레이트를 등록한다**

`Cargo.toml`의 `members` 배열에서 `"crates/buzz-dev-mcp",` 다음 줄에 추가한다.

```toml
    "crates/schoolx-catalog",
```

- [ ] **Step 2: 매니페스트를 만든다**

`crates/schoolx-catalog/Cargo.toml`:

```toml
[package]
name = "schoolx-catalog"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
uuid = { workspace = true, features = ["v5"] }
async-trait = "0.1"
```

`uuid`의 워크스페이스 기본 feature는 `["v4", "serde"]`뿐이라 `v5`를 여기서 더한다. feature는 가산적이므로 다른 크레이트에 영향이 없다.

- [ ] **Step 3: catalog 정의를 만든다**

`crates/schoolx-catalog/catalog.json`:

```json
{
  "catalog_id": "schoolx.default",
  "catalog_version": 1,
  "items": [
    {
      "item_key": "meeting",
      "name": "메인 회의방",
      "description": "전사 회의, 종합 보고, 음성 Huddle",
      "channel_type": "stream",
      "visibility": "private",
      "canvas": "# 메인 회의방\n\n## 이 방의 목적\n전사 회의와 종합 보고를 진행합니다.\n\n## 운영 규칙\n- 결정은 결정 기록 아래에 남깁니다.\n- 회의 요약은 사람이 확인한 뒤 이 캔버스에 올립니다.\n\n## 결정 기록\n\n## 주간 요약\n\n## 할 일\n"
    },
    {
      "item_key": "planning",
      "name": "기획",
      "description": "사업 기획, 일정, 제안",
      "channel_type": "stream",
      "visibility": "private",
      "canvas": "# 기획\n\n## 이 방의 목적\n사업 기획과 일정, 제안을 다룹니다.\n\n## 운영 규칙\n- 안건은 포럼 글로 올리고 토론은 스레드로 유지합니다.\n- 확정된 내용만 결정 기록에 옮깁니다.\n\n## 결정 기록\n\n## 주간 요약\n\n## 할 일\n"
    }
  ]
}
```

- [ ] **Step 4: 실패하는 테스트를 쓴다**

`crates/schoolx-catalog/src/catalog.rs`:

```rust
//! 앱에 컴파일되어 들어가는 읽기 전용 워크스페이스 catalog.

use serde::{Deserialize, Serialize};

/// 채널 공개 범위. 기본값은 항상 `Private`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// 멤버만 읽고 쓴다.
    Private,
    /// 인증된 사용자면 멤버가 아니어도 읽고 쓴다.
    Open,
}

impl Visibility {
    /// Nostr 태그와 DB enum이 쓰는 표준 문자열.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Open => "open",
        }
    }
}

/// catalog 항목 하나 — 업무방 하나에 대응한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogItem {
    /// catalog 안에서 영구히 고정되는 키. 이름이 바뀌어도 이건 바뀌지 않는다.
    pub item_key: String,
    /// 채널 표시 이름.
    pub name: String,
    /// 채널 설명.
    pub description: String,
    /// `stream` 또는 `forum`.
    pub channel_type: String,
    /// 적용 시 기본 공개 범위.
    pub visibility: Visibility,
    /// 시작 캔버스 본문.
    pub canvas: String,
}

/// 버전이 있는 내장 catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    /// catalog 자체의 안정 식별자.
    pub catalog_id: String,
    /// 단조 증가하는 버전. 앱 버전과 함께 올라간다.
    pub catalog_version: u32,
    /// 제공되는 항목들.
    pub items: Vec<CatalogItem>,
}

impl Catalog {
    /// `item_key`로 항목을 찾는다.
    pub fn item(&self, item_key: &str) -> Option<&CatalogItem> {
        self.items.iter().find(|i| i.item_key == item_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_is_stable() {
        let catalog = crate::builtin();
        assert_eq!(catalog.catalog_id, "schoolx.default");
        assert_eq!(catalog.catalog_version, 1);

        let keys: Vec<&str> = catalog.items.iter().map(|i| i.item_key.as_str()).collect();
        assert_eq!(keys, vec!["meeting", "planning"]);
    }

    #[test]
    fn every_builtin_item_is_private() {
        for item in &crate::builtin().items {
            assert_eq!(
                item.visibility,
                Visibility::Private,
                "{} must ship private",
                item.item_key
            );
        }
    }

    #[test]
    fn every_builtin_item_has_a_starting_canvas() {
        for item in &crate::builtin().items {
            assert!(
                !item.canvas.trim().is_empty(),
                "{} must ship a canvas",
                item.item_key
            );
        }
    }
}
```

- [ ] **Step 5: 테스트가 실패하는지 확인한다**

Run: `cargo test -p schoolx-catalog`
Expected: FAIL — `crate::builtin` 이 없어 컴파일 에러

- [ ] **Step 6: `builtin()`을 구현한다**

`crates/schoolx-catalog/src/lib.rs`:

```rust
//! SchoolX 워크스페이스 catalog — 읽기 전용 내장 정의, preflight 판정,
//! idempotent saga, machine-readable result ledger.
//!
//! 설계는 `docs/schoolx-2/WORKSPACE_CATALOG.md`.

pub mod catalog;

pub use catalog::{Catalog, CatalogItem, Visibility};

use std::sync::OnceLock;

const CATALOG_JSON: &str = include_str!("../catalog.json");

/// 앱에 컴파일되어 들어간 catalog. 디스크에 쓰이지 않으므로 드리프트할 수 없다.
///
/// # Panics
///
/// `catalog.json`이 깨졌을 때만 패닉한다. 파일이 크레이트에 컴파일되어
/// 들어가므로 이건 빌드 시점 실수이지 런타임 조건이 아니며, Step 4의
/// 테스트가 모든 빌드에서 이 경로를 지나간다.
pub fn builtin() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(CATALOG_JSON).expect("compiled-in catalog.json must parse")
    })
}
```

- [ ] **Step 7: 테스트가 통과하는지 확인한다**

Run: `cargo test -p schoolx-catalog`
Expected: PASS — 3 tests

- [ ] **Step 8: 커밋한다**

```bash
git add Cargo.toml Cargo.lock crates/schoolx-catalog
git commit -m "feat(schoolx-2): 세션 D — 내장 워크스페이스 catalog 크레이트"
```

---

## Task 2: 결정론적 채널 ID 도출

**Files:**
- Create: `crates/schoolx-catalog/src/channel_id.rs`
- Modify: `crates/schoolx-catalog/src/lib.rs`

**Interfaces:**
- Consumes: 없음
- Produces: `schoolx_catalog::channel_id::derive_channel_id(relay_scope: &str, catalog_id: &str, item_key: &str, generation: u32) -> uuid::Uuid`

`generation`은 사용자가 삭제된 항목의 재생성을 **명시적으로 선택했을 때만** 올라간다. 첫 적용은 항상 `1`이다. `catalog_version`은 넣지 않는다 — 버전이 올라가도 같은 항목은 같은 방이어야 한다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

`crates/schoolx-catalog/src/channel_id.rs`:

```rust
//! catalog 항목에서 채널 UUID를 결정론적으로 도출한다.
//!
//! `desktop/src-tauri/src/commands/channels.rs`의 `starter_channel_uuid()`와
//! 같은 패턴이다. 결정론적 ID 덕분에 provenance를 남기기 직전에 앱이 죽어도
//! 두 번째 생성 시도가 relay에서 거부되어 채널이 둘로 늘지 않는다.

use uuid::Uuid;

/// SchoolX catalog 채널 ID 전용 UUIDv5 네임스페이스.
const CATALOG_CHANNEL_NAMESPACE: Uuid = Uuid::from_bytes([
    0x9f, 0x2c, 0x41, 0x7a, 0x53, 0x8e, 0x4d, 0x11, 0xb6, 0x30, 0x7c, 0x21, 0x0a, 0x4e, 0x88, 0x35,
]);

/// catalog 항목의 채널 UUID를 도출한다.
///
/// `generation`은 사용자가 삭제된 항목의 재생성을 명시적으로 선택했을 때만
/// 증가한다. relay가 채널을 soft delete하고 ID를 계속 점유하므로, 삭제된
/// 항목을 같은 ID로 되살릴 수는 없다.
pub fn derive_channel_id(
    relay_scope: &str,
    catalog_id: &str,
    item_key: &str,
    generation: u32,
) -> Uuid {
    let name = format!(
        "schoolx-catalog:v1:{}:{}:{}:{}",
        relay_scope.trim(),
        catalog_id,
        item_key,
        generation
    );
    Uuid::new_v5(&CATALOG_CHANNEL_NAMESPACE, name.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::derive_channel_id;

    #[test]
    fn same_inputs_give_same_id() {
        let a = derive_channel_id("wss://relay.example", "schoolx.default", "meeting", 1);
        let b = derive_channel_id("wss://relay.example", "schoolx.default", "meeting", 1);
        assert_eq!(a, b);
    }

    #[test]
    fn generation_changes_the_id() {
        let gen1 = derive_channel_id("wss://relay.example", "schoolx.default", "meeting", 1);
        let gen2 = derive_channel_id("wss://relay.example", "schoolx.default", "meeting", 2);
        assert_ne!(gen1, gen2);
    }

    #[test]
    fn relay_scope_changes_the_id() {
        let a = derive_channel_id("wss://a.example", "schoolx.default", "meeting", 1);
        let b = derive_channel_id("wss://b.example", "schoolx.default", "meeting", 1);
        assert_ne!(a, b);
    }

    #[test]
    fn item_key_changes_the_id() {
        let a = derive_channel_id("wss://relay.example", "schoolx.default", "meeting", 1);
        let b = derive_channel_id("wss://relay.example", "schoolx.default", "planning", 1);
        assert_ne!(a, b);
    }

    #[test]
    fn relay_scope_is_trimmed() {
        let a = derive_channel_id("  wss://relay.example  ", "schoolx.default", "meeting", 1);
        let b = derive_channel_id("wss://relay.example", "schoolx.default", "meeting", 1);
        assert_eq!(a, b);
    }
}
```

- [ ] **Step 2: 테스트가 실패하는지 확인한다**

`crates/schoolx-catalog/src/lib.rs`의 `pub mod catalog;` 아래에 추가한다.

```rust
pub mod channel_id;
```

Run: `cargo test -p schoolx-catalog channel_id`
Expected: PASS — 도출 함수를 테스트와 같은 파일에 이미 썼으므로 바로 통과한다. 실패하면 `uuid`의 `v5` feature가 빠진 것이다.

- [ ] **Step 3: 커밋한다**

```bash
git add crates/schoolx-catalog
git commit -m "feat(schoolx-2): 세션 D — 결정론적 catalog 채널 ID"
```

---

## Task 3: provenance 이벤트 표현

**Files:**
- Create: `crates/schoolx-catalog/src/provenance.rs`
- Modify: `crates/schoolx-catalog/src/lib.rs`

**Interfaces:**
- Consumes: 없음
- Produces:
  - `schoolx_catalog::provenance::KIND_WORKSPACE_PROVENANCE: u32` (= 39500)
  - `Provenance { catalog_id, catalog_version, item_key, generation, steps: StepStates, applied_at }`
  - `StepStates { channel: StepStatus, canvas: StepStatus, membership: StepStatus }`
  - `StepStatus` = `Pending | Done | Failed`
  - `Provenance::d_tag(&self) -> String`
  - `Provenance::is_complete(&self) -> bool`

- [ ] **Step 1: 실패하는 테스트를 쓴다**

`crates/schoolx-catalog/src/provenance.rs`:

```rust
//! kind 39500 — 워크스페이스 template provenance manifest.
//!
//! 채널 스코프 addressable 이벤트다. `d` 태그가 `<catalog_id>:<item_key>`라
//! 항목당 정확히 하나이고, NIP-33 LWW가 적용되어 재시도해도 이벤트가 쌓이지
//! 않는다. `h` 태그가 채널 ID라 private 채널 ACL이 그대로 걸린다.
//!
//! relay가 kind 39000을 DB 컬럼에서만 재구성하므로 채널 생성 이벤트에 실은
//! provenance 태그는 보존되지 않는다. 그래서 별도 이벤트가 필요하다.

use serde::{Deserialize, Serialize};

/// 워크스페이스 template provenance manifest.
///
/// SchoolX 예약 대역 39500–39599의 첫 번째 kind. 예약 대역을 두는 이유는 SQL
/// 마이그레이션 `9001+`와 같다 — upstream이 같은 번호를 쓰면 조용히 충돌하고,
/// 충돌은 컴파일 타임에 잡히지 않는다.
pub const KIND_WORKSPACE_PROVENANCE: u32 = 39500;

/// saga 단계 하나의 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    /// 아직 실행되지 않았다.
    Pending,
    /// 성공했다. 재시도는 이 단계를 건너뛴다.
    Done,
    /// 실행했고 실패했다. 재시도가 다시 시도한다.
    Failed,
}

/// 세 단계의 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepStates {
    /// 채널 생성.
    pub channel: StepStatus,
    /// 시작 캔버스 적용.
    pub canvas: StepStatus,
    /// 적용자가 owner로 들어갔는지 확인.
    pub membership: StepStatus,
}

impl Default for StepStates {
    fn default() -> Self {
        Self {
            channel: StepStatus::Pending,
            canvas: StepStatus::Pending,
            membership: StepStatus::Pending,
        }
    }
}

/// kind 39500 이벤트의 content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// 적용에 쓰인 catalog의 안정 식별자.
    pub catalog_id: String,
    /// 적용에 쓰인 catalog 버전.
    pub catalog_version: u32,
    /// 적용된 항목의 안정 키.
    pub item_key: String,
    /// 채널 ID 도출에 쓰인 세대. 명시적 재생성에서만 증가한다.
    pub generation: u32,
    /// 단계별 상태.
    pub steps: StepStates,
    /// 마지막 갱신 시각 (RFC 3339).
    pub applied_at: String,
}

impl Provenance {
    /// 이 항목의 addressable `d` 태그 값.
    pub fn d_tag(&self) -> String {
        d_tag(&self.catalog_id, &self.item_key)
    }

    /// 세 단계가 모두 `Done`인가.
    pub fn is_complete(&self) -> bool {
        self.steps.channel == StepStatus::Done
            && self.steps.canvas == StepStatus::Done
            && self.steps.membership == StepStatus::Done
    }
}

/// `<catalog_id>:<item_key>` — addressable `d` 태그 값.
pub fn d_tag(catalog_id: &str, item_key: &str) -> String {
    format!("{catalog_id}:{item_key}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Provenance {
        Provenance {
            catalog_id: "schoolx.default".into(),
            catalog_version: 1,
            item_key: "meeting".into(),
            generation: 1,
            steps: StepStates::default(),
            applied_at: "2026-07-28T09:00:00Z".into(),
        }
    }

    #[test]
    fn kind_is_in_the_reserved_schoolx_band() {
        assert!((39500..=39599).contains(&KIND_WORKSPACE_PROVENANCE));
    }

    #[test]
    fn kind_is_addressable() {
        // NIP-33 parameterized replaceable 범위여야 LWW가 적용된다.
        assert!((30000..=39999).contains(&KIND_WORKSPACE_PROVENANCE));
    }

    #[test]
    fn d_tag_pairs_catalog_and_item() {
        assert_eq!(sample().d_tag(), "schoolx.default:meeting");
    }

    #[test]
    fn fresh_provenance_is_not_complete() {
        assert!(!sample().is_complete());
    }

    #[test]
    fn all_done_is_complete() {
        let mut p = sample();
        p.steps = StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Done,
            membership: StepStatus::Done,
        };
        assert!(p.is_complete());
    }

    #[test]
    fn partial_steps_are_not_complete() {
        let mut p = sample();
        p.steps = StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Failed,
            membership: StepStatus::Pending,
        };
        assert!(!p.is_complete());
    }

    #[test]
    fn round_trips_through_json() {
        let p = sample();
        let json = serde_json::to_string(&p).expect("serialize");
        let back: Provenance = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, back);
    }
}
```

- [ ] **Step 2: 모듈을 선언한다**

`crates/schoolx-catalog/src/lib.rs`의 `pub mod channel_id;` 아래에 추가한다.

```rust
pub mod provenance;
```

- [ ] **Step 3: 테스트가 통과하는지 확인한다**

Run: `cargo test -p schoolx-catalog provenance`
Expected: PASS — 7 tests

- [ ] **Step 4: 커밋한다**

```bash
git add crates/schoolx-catalog
git commit -m "feat(schoolx-2): 세션 D — kind 39500 provenance 표현"
```

---

## Task 4: relay가 kind 39500을 받게 한다

**Files:**
- Modify: `crates/buzz-core/src/kind.rs`
- Modify: `crates/buzz-relay/src/handlers/ingest.rs`

**Interfaces:**
- Consumes: 없음 (kind 번호는 Task 3과 같은 값을 독립적으로 선언한다 — `buzz-core`는 `schoolx-catalog`에 의존하지 않는다)
- Produces: `buzz_core::kind::KIND_WORKSPACE_PROVENANCE: u32` (= 39500)

NIP-33 LWW 저장은 `ingest.rs`의 `is_parameterized_replaceable` 분기가 **이미 일반적으로 처리한다**. 별도 저장 코드가 필요 없다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

`crates/buzz-relay/src/handlers/ingest.rs`의 기존 `mod tests` 안에 추가한다.

```rust
    #[test]
    fn workspace_provenance_requires_h_channel_scope() {
        assert!(requires_h_channel_scope(
            buzz_core::kind::KIND_WORKSPACE_PROVENANCE
        ));
    }

    #[test]
    fn workspace_provenance_is_a_channel_write() {
        let event = test_event(buzz_core::kind::KIND_WORKSPACE_PROVENANCE, vec![]);
        assert_eq!(
            required_scope_for_kind(buzz_core::kind::KIND_WORKSPACE_PROVENANCE, &event),
            Ok(Scope::ChannelsWrite)
        );
    }
```

`test_event`는 이 모듈의 기존 헬퍼다. 이름이 다르면 같은 파일의 다른 `required_scope_for_kind` 테스트가 쓰는 헬퍼에 맞춘다.

- [ ] **Step 2: 테스트가 실패하는지 확인한다**

Run: `cargo test -p buzz-relay workspace_provenance`
Expected: FAIL — `KIND_WORKSPACE_PROVENANCE`가 없어 컴파일 에러

- [ ] **Step 3: kind 상수를 추가한다**

`crates/buzz-core/src/kind.rs`에서 `pub const KIND_DM_VISIBILITY: u32 = 30622;` 블록 다음, `PARAM_REPLACEABLE_KIND_MIN` 선언 앞에 추가한다.

```rust
// SchoolX 예약 대역 (39500–39599)
//
// 이 대역은 SchoolX 포크 전용이다. upstream이 같은 번호를 쓰면 조용히
// 충돌하며, sqlx 마이그레이션 중복과 마찬가지로 컴파일 타임에 잡히지 않는다.
// 새 SchoolX kind는 반드시 이 대역에서 고른다.

/// 워크스페이스 template provenance manifest (채널 스코프 addressable).
///
/// `d` = `<catalog_id>:<item_key>`, `h` = 적용으로 만들어진 channel ID.
/// content는 catalog 버전과 단계별 적용 상태를 담는다. SchoolX 전용 필드를
/// 넣지 않으므로 다른 Buzz 배포도 그대로 쓸 수 있다.
/// 설계: `docs/schoolx-2/WORKSPACE_CATALOG.md`.
pub const KIND_WORKSPACE_PROVENANCE: u32 = 39500;
```

- [ ] **Step 4: scope 매핑에 추가한다**

`crates/buzz-relay/src/handlers/ingest.rs`에서 이 줄을 찾는다.

```rust
        KIND_NIP29_CREATE_GROUP | KIND_CANVAS => Ok(Scope::ChannelsWrite),
```

이렇게 바꾼다.

```rust
        KIND_NIP29_CREATE_GROUP | KIND_CANVAS | KIND_WORKSPACE_PROVENANCE => {
            Ok(Scope::ChannelsWrite)
        }
```

같은 파일 위쪽의 `buzz_core::kind::{...}` import 목록에 `KIND_WORKSPACE_PROVENANCE`를 알파벳 순서에 맞게 추가한다.

- [ ] **Step 5: h-tag 게이트에 추가한다**

`requires_h_channel_scope`의 `matches!` 목록에서 `| KIND_CANVAS` 다음 줄에 추가한다.

```rust
            | KIND_WORKSPACE_PROVENANCE
```

- [ ] **Step 6: 테스트가 통과하는지 확인한다**

Run: `cargo test -p buzz-relay workspace_provenance`
Expected: PASS — 2 tests

- [ ] **Step 7: 회귀가 없는지 확인한다**

Run: `cargo test -p buzz-core && cargo test -p buzz-relay --lib`
Expected: PASS — 기존 테스트가 전부 통과

- [ ] **Step 8: 커밋한다**

```bash
git add crates/buzz-core crates/buzz-relay
git commit -m "feat(schoolx-2): 세션 D — relay가 kind 39500 provenance를 받는다"
```

---

## Task 5: effects trait과 fake

**Files:**
- Create: `crates/schoolx-catalog/src/effects.rs`
- Modify: `crates/schoolx-catalog/src/lib.rs`

**Interfaces:**
- Consumes: `provenance::Provenance`
- Produces:
  - `trait CatalogEffects` (async, `Send + Sync`)
    - `async fn relay_scope(&self) -> String`
    - `async fn list_channels(&self) -> Result<Vec<ChannelRef>, EffectError>`
    - `async fn fetch_provenance(&self, catalog_id: &str) -> Result<Vec<Provenance>, EffectError>`
    - `async fn create_channel(&self, spec: ChannelSpec) -> Result<CreateOutcome, EffectError>`
    - `async fn set_canvas(&self, channel_id: Uuid, content: &str) -> Result<(), EffectError>`
    - `async fn is_owner(&self, channel_id: Uuid) -> Result<bool, EffectError>`
    - `async fn publish_provenance(&self, channel_id: Uuid, p: &Provenance) -> Result<(), EffectError>`
    - `async fn now_rfc3339(&self) -> String`
  - `ChannelRef { id: Uuid, name: String }`
  - `ChannelSpec { id: Uuid, name: String, description: String, channel_type: String, visibility: Visibility }`
  - `CreateOutcome` = `Created | Duplicate`
  - `EffectError(String)`
  - `#[cfg(test)] FakeEffects` — 실패 주입이 가능한 인메모리 구현

- [ ] **Step 1: trait과 fake를 쓴다**

`crates/schoolx-catalog/src/effects.rs`:

```rust
//! relay I/O 경계.
//!
//! saga가 이 trait 뒤에서만 relay와 이야기하므로, fault injection 테스트가
//! live relay 없이 돈다. 실제 구현은 데스크톱 Tauri 백엔드에 있다.

use crate::catalog::Visibility;
use crate::provenance::Provenance;
use uuid::Uuid;

/// effect 실행 실패. 메시지는 사용자에게 그대로 보일 수 있다.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct EffectError(pub String);

/// 접근 가능한 채널 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRef {
    /// 채널 UUID.
    pub id: Uuid,
    /// 현재 표시 이름. 사용자가 바꿨을 수 있다.
    pub name: String,
}

/// 채널 생성 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSpec {
    /// 결정론적으로 도출된 채널 UUID.
    pub id: Uuid,
    /// 채널 이름.
    pub name: String,
    /// 채널 설명.
    pub description: String,
    /// `stream` 또는 `forum`.
    pub channel_type: String,
    /// 공개 범위.
    pub visibility: Visibility,
}

/// 채널 생성 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateOutcome {
    /// 새로 만들어졌다.
    Created,
    /// 이 UUID가 이미 점유돼 있다. relay가 채널을 soft delete하고 ID를 계속
    /// 점유하므로, 접근 가능 목록에 없다면 예전에 만들었다가 삭제된 것이다.
    Duplicate,
}

/// saga가 필요로 하는 relay 연산.
#[async_trait::async_trait]
pub trait CatalogEffects: Send + Sync {
    /// 채널 ID 도출에 쓰이는 relay 범위 문자열.
    async fn relay_scope(&self) -> String;

    /// 현재 사용자가 접근할 수 있는 채널. 삭제된 채널은 포함되지 않는다.
    async fn list_channels(&self) -> Result<Vec<ChannelRef>, EffectError>;

    /// 읽을 수 있는 이 catalog의 provenance 이벤트 전부.
    ///
    /// 채널 스코프라 비멤버인 항목은 결과에 나타나지 않는다. 이건 버그가
    /// 아니라 보안 계약이다.
    async fn fetch_provenance(&self, catalog_id: &str) -> Result<Vec<Provenance>, EffectError>;

    /// 채널을 만든다. 이미 점유된 ID면 `Duplicate`.
    async fn create_channel(&self, spec: ChannelSpec) -> Result<CreateOutcome, EffectError>;

    /// 시작 캔버스를 적용한다.
    async fn set_canvas(&self, channel_id: Uuid, content: &str) -> Result<(), EffectError>;

    /// 현재 사용자가 이 채널의 owner인가.
    async fn is_owner(&self, channel_id: Uuid) -> Result<bool, EffectError>;

    /// provenance 이벤트를 발행한다 (kind 39500).
    async fn publish_provenance(
        &self,
        channel_id: Uuid,
        provenance: &Provenance,
    ) -> Result<(), EffectError>;

    /// 현재 시각 (RFC 3339). 테스트가 고정할 수 있도록 주입한다.
    async fn now_rfc3339(&self) -> String;
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    /// 실패를 주입할 수 있는 인메모리 구현.
    #[derive(Default)]
    pub(crate) struct FakeEffects {
        pub channels: Mutex<Vec<ChannelRef>>,
        pub provenance: Mutex<Vec<Provenance>>,
        /// 이미 점유된 채널 UUID — soft delete된 것 포함.
        pub burned_ids: Mutex<HashSet<Uuid>>,
        pub canvases: Mutex<HashMap<Uuid, String>>,
        /// 이 이름의 연산을 한 번 실패시킨다.
        pub fail_once: Mutex<HashSet<String>>,
        pub published: Mutex<Vec<(Uuid, Provenance)>>,
    }

    impl FakeEffects {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        /// 다음 `op` 호출을 한 번 실패시킨다.
        pub(crate) fn fail_next(&self, op: &str) {
            self.fail_once.lock().expect("lock").insert(op.to_string());
        }

        fn take_failure(&self, op: &str) -> Result<(), EffectError> {
            let mut guard = self.fail_once.lock().expect("lock");
            if guard.remove(op) {
                return Err(EffectError(format!("injected failure: {op}")));
            }
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl CatalogEffects for FakeEffects {
        async fn relay_scope(&self) -> String {
            "wss://relay.test".into()
        }

        async fn list_channels(&self) -> Result<Vec<ChannelRef>, EffectError> {
            self.take_failure("list_channels")?;
            Ok(self.channels.lock().expect("lock").clone())
        }

        async fn fetch_provenance(&self, catalog_id: &str) -> Result<Vec<Provenance>, EffectError> {
            self.take_failure("fetch_provenance")?;
            Ok(self
                .provenance
                .lock()
                .expect("lock")
                .iter()
                .filter(|p| p.catalog_id == catalog_id)
                .cloned()
                .collect())
        }

        async fn create_channel(&self, spec: ChannelSpec) -> Result<CreateOutcome, EffectError> {
            self.take_failure("create_channel")?;
            if !self.burned_ids.lock().expect("lock").insert(spec.id) {
                return Ok(CreateOutcome::Duplicate);
            }
            self.channels.lock().expect("lock").push(ChannelRef {
                id: spec.id,
                name: spec.name,
            });
            Ok(CreateOutcome::Created)
        }

        async fn set_canvas(&self, channel_id: Uuid, content: &str) -> Result<(), EffectError> {
            self.take_failure("set_canvas")?;
            self.canvases
                .lock()
                .expect("lock")
                .insert(channel_id, content.to_string());
            Ok(())
        }

        async fn is_owner(&self, channel_id: Uuid) -> Result<bool, EffectError> {
            self.take_failure("is_owner")?;
            Ok(self
                .channels
                .lock()
                .expect("lock")
                .iter()
                .any(|c| c.id == channel_id))
        }

        async fn publish_provenance(
            &self,
            channel_id: Uuid,
            provenance: &Provenance,
        ) -> Result<(), EffectError> {
            self.take_failure("publish_provenance")?;
            self.published
                .lock()
                .expect("lock")
                .push((channel_id, provenance.clone()));
            // NIP-33 LWW: 같은 d 태그는 교체된다.
            let mut store = self.provenance.lock().expect("lock");
            store.retain(|p| p.d_tag() != provenance.d_tag());
            store.push(provenance.clone());
            Ok(())
        }

        async fn now_rfc3339(&self) -> String {
            "2026-07-28T09:00:00Z".into()
        }
    }
}
```

- [ ] **Step 2: 모듈을 선언한다**

`crates/schoolx-catalog/src/lib.rs`의 `pub mod provenance;` 아래에 추가한다.

```rust
pub mod effects;
```

- [ ] **Step 3: 컴파일과 테스트를 확인한다**

Run: `cargo test -p schoolx-catalog`
Expected: PASS — 기존 테스트가 전부 통과하고 새 컴파일 에러가 없다

- [ ] **Step 4: 커밋한다**

```bash
git add crates/schoolx-catalog
git commit -m "feat(schoolx-2): 세션 D — catalog effects 경계와 fake"
```

---

## Task 6: preflight 판정

**Files:**
- Create: `crates/schoolx-catalog/src/preflight.rs`
- Modify: `crates/schoolx-catalog/src/lib.rs`

**Interfaces:**
- Consumes: `catalog::{Catalog, CatalogItem}`, `effects::{CatalogEffects, ChannelRef}`, `provenance::Provenance`
- Produces:
  - `Decision` = `CreateOrRecreate | Resume | NoChange | Conflict | Retired`
  - `PreflightItem { item_key, decision, channel_id: Option<Uuid>, generation: u32, renamed: bool }`
  - `async fn preflight(catalog: &Catalog, effects: &dyn CatalogEffects) -> Result<Vec<PreflightItem>, EffectError>`

`Deleted`는 preflight 결과가 아니다 — 생성 시도가 `Duplicate`로 거부돼야만 확정되므로 saga(Task 7)에서 만들어진다. `renamed`는 판정이 아니라 별도 플래그다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

`crates/schoolx-catalog/src/preflight.rs`:

```rust
//! 적용 전 항목별 판정.
//!
//! 판정 근거는 이름이 아니라 provenance다. 이름은 `Conflict` 감지에만 쓴다.

use crate::catalog::Catalog;
use crate::channel_id::derive_channel_id;
use crate::effects::{CatalogEffects, EffectError};
use crate::provenance::Provenance;
use serde::Serialize;
use uuid::Uuid;

/// 항목 하나에 대한 판정.
///
/// `Serialize`는 Tauri command 반환 타입이라 필요하다 (Task 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// provenance가 없고 동명 채널도 없다. 생성을 시도한다. 거부되면
    /// 예전에 만들었다가 삭제된 것이다.
    CreateOrRecreate,
    /// provenance가 있고 일부 단계가 미완료다. 미완료 단계만 실행한다.
    Resume,
    /// provenance가 있고 전 단계가 완료다. 아무것도 하지 않는다.
    NoChange,
    /// provenance가 없는데 동명 채널이 있다. 자동 채택하지 않는다.
    Conflict,
    /// provenance는 있는데 catalog에서 항목이 빠졌다. 채널은 유지한다.
    Retired,
}

/// 항목 하나의 preflight 결과.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreflightItem {
    /// catalog 항목 키. `Retired`면 catalog에 더는 없는 키다.
    pub item_key: String,
    /// 판정.
    pub decision: Decision,
    /// 알려진 채널 ID. `CreateOrRecreate`면 앞으로 쓸 ID다.
    pub channel_id: Option<Uuid>,
    /// 채널 ID 도출에 쓸 세대.
    pub generation: u32,
    /// 사용자가 이름을 바꿨는가. 판정과 무관한 표시용 플래그다.
    pub renamed: bool,
}

/// catalog 전체를 판정한다.
pub async fn preflight(
    catalog: &Catalog,
    effects: &dyn CatalogEffects,
) -> Result<Vec<PreflightItem>, EffectError> {
    let relay_scope = effects.relay_scope().await;
    let channels = effects.list_channels().await?;
    let provenance = effects.fetch_provenance(&catalog.catalog_id).await?;

    let mut out = Vec::with_capacity(catalog.items.len());

    for item in &catalog.items {
        let known: Option<&Provenance> =
            provenance.iter().find(|p| p.item_key == item.item_key);

        match known {
            Some(p) => {
                let channel_id = derive_channel_id(
                    &relay_scope,
                    &catalog.catalog_id,
                    &item.item_key,
                    p.generation,
                );
                let live = channels.iter().find(|c| c.id == channel_id);
                out.push(PreflightItem {
                    item_key: item.item_key.clone(),
                    decision: if p.is_complete() {
                        Decision::NoChange
                    } else {
                        Decision::Resume
                    },
                    channel_id: Some(channel_id),
                    generation: p.generation,
                    renamed: live.is_some_and(|c| c.name != item.name),
                });
            }
            None => {
                let channel_id =
                    derive_channel_id(&relay_scope, &catalog.catalog_id, &item.item_key, 1);
                let name_taken = channels.iter().any(|c| c.name == item.name);
                out.push(PreflightItem {
                    item_key: item.item_key.clone(),
                    decision: if name_taken {
                        Decision::Conflict
                    } else {
                        Decision::CreateOrRecreate
                    },
                    channel_id: Some(channel_id),
                    generation: 1,
                    renamed: false,
                });
            }
        }
    }

    // catalog에서 빠졌는데 provenance가 남은 항목.
    for p in &provenance {
        if catalog.item(&p.item_key).is_none() {
            out.push(PreflightItem {
                item_key: p.item_key.clone(),
                decision: Decision::Retired,
                channel_id: None,
                generation: p.generation,
                renamed: false,
            });
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::fake::FakeEffects;
    use crate::effects::ChannelRef;
    use crate::provenance::{StepStates, StepStatus};

    fn done() -> StepStates {
        StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Done,
            membership: StepStatus::Done,
        }
    }

    fn provenance(item_key: &str, steps: StepStates) -> Provenance {
        Provenance {
            catalog_id: "schoolx.default".into(),
            catalog_version: 1,
            item_key: item_key.into(),
            generation: 1,
            steps,
            applied_at: "2026-07-28T09:00:00Z".into(),
        }
    }

    fn find<'a>(items: &'a [PreflightItem], key: &str) -> &'a PreflightItem {
        items
            .iter()
            .find(|i| i.item_key == key)
            .expect("item present")
    }

    /// 이미 적용된 항목을 시드한다.
    ///
    /// provenance는 채널 스코프 이벤트라 채널이 사라지면 읽을 수 없다. fake도
    /// 그렇게 동작하므로 — `fetch_provenance`가 `channels`에 살아 있는 채널의
    /// 항목만 돌려준다 — 시딩은 반드시 채널과 짝으로 해야 한다. provenance만
    /// 넣으면 preflight에는 "적용한 적 없음"으로 보인다.
    fn seed_applied(fx: &FakeEffects, item_key: &str, name: &str, steps: StepStates) -> Uuid {
        let channel_id = derive_channel_id("wss://relay.test", "schoolx.default", item_key, 1);
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: channel_id,
            name: name.into(),
        });
        fx.provenance
            .lock()
            .expect("lock")
            .push((channel_id, provenance(item_key, steps)));
        channel_id
    }

    #[tokio::test]
    async fn fresh_install_creates_everything() {
        let fx = FakeEffects::new();
        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        assert_eq!(items.len(), 2);
        for item in &items {
            assert_eq!(item.decision, Decision::CreateOrRecreate);
            assert_eq!(item.generation, 1);
        }
    }

    #[tokio::test]
    async fn completed_item_is_no_change() {
        let fx = FakeEffects::new();
        seed_applied(&fx, "meeting", "메인 회의방", done());

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        assert_eq!(find(&items, "meeting").decision, Decision::NoChange);
        assert_eq!(
            find(&items, "planning").decision,
            Decision::CreateOrRecreate
        );
    }

    #[tokio::test]
    async fn partial_item_resumes() {
        let fx = FakeEffects::new();
        seed_applied(
            &fx,
            "meeting",
            "메인 회의방",
            StepStates {
                channel: StepStatus::Done,
                canvas: StepStatus::Failed,
                membership: StepStatus::Pending,
            },
        );

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        assert_eq!(find(&items, "meeting").decision, Decision::Resume);
    }

    #[tokio::test]
    async fn same_name_without_provenance_is_a_conflict() {
        let fx = FakeEffects::new();
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: Uuid::new_v4(),
            name: "기획".into(),
        });

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        assert_eq!(find(&items, "planning").decision, Decision::Conflict);
        assert_eq!(
            find(&items, "meeting").decision,
            Decision::CreateOrRecreate
        );
    }

    #[tokio::test]
    async fn rename_is_a_flag_not_a_decision() {
        let fx = FakeEffects::new();
        // catalog 이름은 "메인 회의방"인데 멤버가 바꿔 놓은 상태.
        seed_applied(
            &fx,
            "meeting",
            "2026 전체회의",
            StepStates {
                channel: StepStatus::Done,
                canvas: StepStatus::Failed,
                membership: StepStatus::Pending,
            },
        );

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        let meeting = find(&items, "meeting");
        // 이름을 바꿨어도 미완료면 재시도 대상이다.
        assert_eq!(meeting.decision, Decision::Resume);
        assert!(meeting.renamed);
    }

    #[tokio::test]
    async fn item_dropped_from_catalog_is_retired() {
        let fx = FakeEffects::new();
        // catalog에 없는 항목이지만 예전 버전에서 적용된 채로 남아 있다.
        seed_applied(&fx, "finance", "재무", done());

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        assert_eq!(find(&items, "finance").decision, Decision::Retired);
    }
}
```

- [ ] **Step 2: 모듈을 선언하고 dev-dependency를 더한다**

`crates/schoolx-catalog/src/lib.rs`에 추가한다.

```rust
pub mod preflight;
```

`crates/schoolx-catalog/Cargo.toml`에 추가한다.

```toml
[dev-dependencies]
tokio = { workspace = true, features = ["macros", "rt"] }
```

- [ ] **Step 3: 테스트가 통과하는지 확인한다**

Run: `cargo test -p schoolx-catalog preflight`
Expected: PASS — 6 tests

- [ ] **Step 4: 커밋한다**

```bash
git add crates/schoolx-catalog Cargo.lock
git commit -m "feat(schoolx-2): 세션 D — catalog preflight 판정"
```

---

## Task 7: saga 실행기와 result ledger

**Files:**
- Create: `crates/schoolx-catalog/src/ledger.rs`
- Create: `crates/schoolx-catalog/src/saga.rs`
- Modify: `crates/schoolx-catalog/src/lib.rs`

**Interfaces:**
- Consumes: Task 1·2·3·5·6의 전부
- Produces:
  - `Outcome` = `Applied | Unchanged | Partial | Blocked`
  - `UserAction` = `ConfirmRecreate | ResolveConflict`
  - `LedgerItem { item_key, decision, channel_id, generation, steps, outcome, user_action, error }`
  - `Ledger { catalog_id, catalog_version, items: Vec<LedgerItem> }`
  - `async fn apply(catalog, effects, selected: &[String]) -> Result<Ledger, EffectError>`

`apply`는 `selected`에 든 `item_key`만 처리한다. 한 항목의 실패가 다른 항목을 막지 않는다.

- [ ] **Step 1: ledger 타입을 쓴다**

스펙 §10의 예시 JSON은 `steps`를 배열(`[{"step":"channel","status":"done"}, …]`)로 그렸지만, 여기서는 Task 3의 `StepStates` 구조체를 그대로 쓴다 (`{"channel":"done","canvas":"failed","membership":"pending"}`). 단계 집합이 고정이라 배열일 이유가 없고, provenance와 ledger가 같은 타입을 공유해 어긋날 수 없다. Task 12 Step 7에서 스펙을 이 모양으로 맞춘다.

`crates/schoolx-catalog/src/ledger.rs`:

```rust
//! 적용 실행의 machine-readable 결과. UI와 CLI가 같은 것을 읽는다.

use crate::preflight::Decision;
use crate::provenance::StepStates;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 항목 하나의 최종 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// 이번 실행에서 무언가 바꿨고 전 단계가 끝났다.
    Applied,
    /// 이미 끝나 있어 아무것도 하지 않았다.
    Unchanged,
    /// 일부 단계가 실패했다. 재시도가 이어서 한다.
    Partial,
    /// 사용자 조치 없이는 진행할 수 없다.
    Blocked,
}

/// 사람이 결정해야 하는 것.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserAction {
    /// 예전에 만들었다가 삭제된 항목이다. 다시 만들지 물어본다.
    ConfirmRecreate,
    /// provenance 없는 동명 채널이 있다. 어떻게 할지 물어본다.
    ResolveConflict,
}

/// 항목 하나의 결과.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerItem {
    /// catalog 항목 키.
    pub item_key: String,
    /// preflight 판정을 문자열로 남긴 것.
    pub decision: String,
    /// 관련 채널 ID.
    pub channel_id: Option<Uuid>,
    /// 쓰인 세대.
    pub generation: u32,
    /// 단계별 최종 상태.
    pub steps: StepStates,
    /// 최종 상태.
    pub outcome: Outcome,
    /// 필요한 사용자 조치.
    pub user_action: Option<UserAction>,
    /// 실패 사유.
    pub error: Option<String>,
}

/// 적용 실행 하나의 결과 전체.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    /// 적용에 쓰인 catalog.
    pub catalog_id: String,
    /// 적용에 쓰인 catalog 버전.
    pub catalog_version: u32,
    /// 항목별 결과.
    pub items: Vec<LedgerItem>,
}

/// `Decision`의 안정적인 문자열 표현. UI와 CLI가 이 값을 읽는다.
pub fn decision_label(decision: Decision) -> &'static str {
    match decision {
        Decision::CreateOrRecreate => "create_or_recreate",
        Decision::Resume => "resume",
        Decision::NoChange => "no_change",
        Decision::Conflict => "conflict",
        Decision::Retired => "retired",
    }
}
```

- [ ] **Step 2: 실패하는 saga 테스트를 쓴다**

`crates/schoolx-catalog/src/saga.rs`:

```rust
//! idempotent saga 실행기.
//!
//! 단계는 채널 생성 → 시작 캔버스 → owner 확인이다. 각 단계는 provenance를
//! 보고 완료면 건너뛰고, 실행하고, provenance를 갱신한다.
//!
//! **실패해도 되돌리지 않는다.** 채널을 만든 뒤 캔버스에서 실패하면 채널을
//! 지우지 않고 상태만 기록한다. 재시도가 캔버스부터 이어서 한다.

use crate::catalog::Catalog;
use crate::channel_id::derive_channel_id;
use crate::effects::{CatalogEffects, ChannelSpec, CreateOutcome, EffectError};
use crate::ledger::{decision_label, Ledger, LedgerItem, Outcome, UserAction};
use crate::preflight::{preflight, Decision, PreflightItem};
use crate::provenance::{Provenance, StepStates, StepStatus};

/// 선택한 항목을 적용한다.
///
/// `selected`에 없는 항목은 건드리지 않는다. 한 항목의 실패가 다른 항목을
/// 막지 않는다.
pub async fn apply(
    catalog: &Catalog,
    effects: &dyn CatalogEffects,
    selected: &[String],
) -> Result<Ledger, EffectError> {
    let plan = preflight(catalog, effects).await?;
    let relay_scope = effects.relay_scope().await;
    let now = effects.now_rfc3339().await;

    let mut items = Vec::new();

    for step in plan {
        if !selected.contains(&step.item_key) {
            continue;
        }
        items.push(apply_item(catalog, effects, &relay_scope, &now, step).await);
    }

    Ok(Ledger {
        catalog_id: catalog.catalog_id.clone(),
        catalog_version: catalog.catalog_version,
        items,
    })
}

async fn apply_item(
    catalog: &Catalog,
    effects: &dyn CatalogEffects,
    relay_scope: &str,
    now: &str,
    plan: PreflightItem,
) -> LedgerItem {
    let decision = decision_label(plan.decision).to_string();

    let blocked = |action: UserAction| LedgerItem {
        item_key: plan.item_key.clone(),
        decision: decision.clone(),
        channel_id: plan.channel_id,
        generation: plan.generation,
        steps: StepStates::default(),
        outcome: Outcome::Blocked,
        user_action: Some(action),
        error: None,
    };

    match plan.decision {
        Decision::Conflict => return blocked(UserAction::ResolveConflict),
        Decision::Retired | Decision::NoChange => {
            return LedgerItem {
                item_key: plan.item_key.clone(),
                decision,
                channel_id: plan.channel_id,
                generation: plan.generation,
                steps: StepStates {
                    channel: StepStatus::Done,
                    canvas: StepStatus::Done,
                    membership: StepStatus::Done,
                },
                outcome: Outcome::Unchanged,
                user_action: None,
                error: None,
            }
        }
        Decision::CreateOrRecreate | Decision::Resume => {}
    }

    let Some(item) = catalog.item(&plan.item_key) else {
        return blocked(UserAction::ResolveConflict);
    };

    let channel_id = derive_channel_id(
        relay_scope,
        &catalog.catalog_id,
        &plan.item_key,
        plan.generation,
    );

    let mut provenance = Provenance {
        catalog_id: catalog.catalog_id.clone(),
        catalog_version: catalog.catalog_version,
        item_key: plan.item_key.clone(),
        generation: plan.generation,
        steps: StepStates::default(),
        applied_at: now.to_string(),
    };

    if plan.decision == Decision::Resume {
        if let Ok(existing) = effects.fetch_provenance(&catalog.catalog_id).await {
            if let Some(p) = existing.iter().find(|p| p.item_key == plan.item_key) {
                provenance.steps = p.steps;
            }
        }
    }

    let mut error: Option<String> = None;

    // 단계 1 — 채널 생성.
    if provenance.steps.channel != StepStatus::Done {
        match effects
            .create_channel(ChannelSpec {
                id: channel_id,
                name: item.name.clone(),
                description: item.description.clone(),
                channel_type: item.channel_type.clone(),
                visibility: item.visibility,
            })
            .await
        {
            Ok(CreateOutcome::Created) => provenance.steps.channel = StepStatus::Done,
            Ok(CreateOutcome::Duplicate) => {
                // ID가 이미 점유돼 있는데 접근 가능 목록에 없다 —
                // 예전에 만들었다가 삭제된 항목이다. 자동 재생성하지 않는다.
                return LedgerItem {
                    item_key: plan.item_key.clone(),
                    decision: "deleted".to_string(),
                    channel_id: Some(channel_id),
                    generation: plan.generation,
                    steps: provenance.steps,
                    outcome: Outcome::Blocked,
                    user_action: Some(UserAction::ConfirmRecreate),
                    error: None,
                };
            }
            Err(e) => {
                provenance.steps.channel = StepStatus::Failed;
                error = Some(e.0);
            }
        }
    }

    // 단계 2 — 시작 캔버스.
    if error.is_none() && provenance.steps.canvas != StepStatus::Done {
        match effects.set_canvas(channel_id, &item.canvas).await {
            Ok(()) => provenance.steps.canvas = StepStatus::Done,
            Err(e) => {
                provenance.steps.canvas = StepStatus::Failed;
                error = Some(e.0);
            }
        }
    }

    // 단계 3 — owner 확인.
    if error.is_none() && provenance.steps.membership != StepStatus::Done {
        match effects.is_owner(channel_id).await {
            Ok(true) => provenance.steps.membership = StepStatus::Done,
            Ok(false) => {
                provenance.steps.membership = StepStatus::Failed;
                error = Some("적용자가 채널 owner가 아닙니다".to_string());
            }
            Err(e) => {
                provenance.steps.membership = StepStatus::Failed;
                error = Some(e.0);
            }
        }
    }

    // 어디까지 됐든 provenance를 남긴다. 이게 없으면 재시도가 처음부터 한다.
    if provenance.steps.channel == StepStatus::Done {
        if let Err(e) = effects.publish_provenance(channel_id, &provenance).await {
            error.get_or_insert(e.0);
        }
    }

    let complete = provenance.is_complete();
    LedgerItem {
        item_key: plan.item_key,
        decision,
        channel_id: Some(channel_id),
        generation: plan.generation,
        steps: provenance.steps,
        outcome: if complete {
            Outcome::Applied
        } else {
            Outcome::Partial
        },
        user_action: None,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::fake::FakeEffects;
    use crate::effects::ChannelRef;
    use crate::ledger::Outcome;

    fn both() -> Vec<String> {
        vec!["meeting".to_string(), "planning".to_string()]
    }

    fn item<'a>(ledger: &'a Ledger, key: &str) -> &'a LedgerItem {
        ledger
            .items
            .iter()
            .find(|i| i.item_key == key)
            .expect("item present")
    }

    #[tokio::test]
    async fn first_apply_creates_both_rooms() {
        let fx = FakeEffects::new();
        let ledger = apply(crate::builtin(), &fx, &both()).await.expect("apply");

        assert_eq!(ledger.items.len(), 2);
        for entry in &ledger.items {
            assert_eq!(entry.outcome, Outcome::Applied, "{}", entry.item_key);
        }
        assert_eq!(fx.channels.lock().expect("lock").len(), 2);
    }

    #[tokio::test]
    async fn second_apply_changes_nothing() {
        let fx = FakeEffects::new();
        apply(crate::builtin(), &fx, &both()).await.expect("first");
        let before = fx.channels.lock().expect("lock").len();
        let published_before = fx.published.lock().expect("lock").len();

        let ledger = apply(crate::builtin(), &fx, &both()).await.expect("second");

        for entry in &ledger.items {
            assert_eq!(entry.outcome, Outcome::Unchanged, "{}", entry.item_key);
        }
        assert_eq!(fx.channels.lock().expect("lock").len(), before);
        // 변경 없음이면 provenance를 다시 발행하지도 않는다. `published`는
        // 필터 없는 append-only 로그라 발행 횟수를 그대로 센다.
        assert_eq!(fx.published.lock().expect("lock").len(), published_before);
    }

    #[tokio::test]
    async fn only_selected_items_are_applied() {
        let fx = FakeEffects::new();
        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");

        assert_eq!(ledger.items.len(), 1);
        assert_eq!(ledger.items[0].item_key, "meeting");
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
    }

    #[tokio::test]
    async fn canvas_failure_leaves_the_channel_and_retry_finishes_it() {
        let fx = FakeEffects::new();
        fx.fail_next("set_canvas");

        let first = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("first");
        let entry = item(&first, "meeting");
        assert_eq!(entry.outcome, Outcome::Partial);
        assert_eq!(entry.steps.channel, StepStatus::Done);
        assert_eq!(entry.steps.canvas, StepStatus::Failed);
        // 보상하지 않는다 — 채널은 그대로 있다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);

        let second = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("retry");
        let entry = item(&second, "meeting");
        assert_eq!(entry.outcome, Outcome::Applied);
        // 채널이 중복 생성되지 않았다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
    }

    #[tokio::test]
    async fn channel_failure_retries_from_the_start_without_duplicates() {
        let fx = FakeEffects::new();
        fx.fail_next("create_channel");

        let first = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("first");
        assert_eq!(item(&first, "meeting").outcome, Outcome::Partial);
        assert_eq!(fx.channels.lock().expect("lock").len(), 0);

        let second = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("retry");
        assert_eq!(item(&second, "meeting").outcome, Outcome::Applied);
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
    }

    #[tokio::test]
    async fn deleted_channel_is_not_recreated() {
        let fx = FakeEffects::new();
        apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("first");

        // 사용자가 방을 지운다: 접근 가능 목록에서 사라지지만 ID는 계속 탄 채다.
        // provenance는 손대지 않는다 — 채널 스코프라 채널이 사라지는 것만으로
        // 읽을 수 없게 되고, fake도 그렇게 동작한다. 두 저장소를 같이 비우면
        // relay가 만들 수 없는 상태를 테스트가 대신 만들어 주는 셈이 된다.
        fx.channels.lock().expect("lock").clear();

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("after delete");
        let entry = item(&ledger, "meeting");
        assert_eq!(entry.outcome, Outcome::Blocked);
        assert_eq!(entry.user_action, Some(UserAction::ConfirmRecreate));
        assert_eq!(entry.decision, "deleted");
        // 자동으로 다시 만들지 않았다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 0);
    }

    #[tokio::test]
    async fn name_conflict_blocks_without_touching_anything() {
        let fx = FakeEffects::new();
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: uuid::Uuid::new_v4(),
            name: "기획".into(),
        });

        let ledger = apply(crate::builtin(), &fx, &["planning".to_string()])
            .await
            .expect("apply");
        let entry = item(&ledger, "planning");
        assert_eq!(entry.outcome, Outcome::Blocked);
        assert_eq!(entry.user_action, Some(UserAction::ResolveConflict));
        // 사용자 채널을 채택하지 않았다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
    }

    #[tokio::test]
    async fn one_item_failing_does_not_block_the_other() {
        let fx = FakeEffects::new();
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: uuid::Uuid::new_v4(),
            name: "기획".into(),
        });

        let ledger = apply(crate::builtin(), &fx, &both()).await.expect("apply");
        assert_eq!(item(&ledger, "planning").outcome, Outcome::Blocked);
        assert_eq!(item(&ledger, "meeting").outcome, Outcome::Applied);
    }

    #[tokio::test]
    async fn ledger_serializes_for_ui_and_cli() {
        let fx = FakeEffects::new();
        let ledger = apply(crate::builtin(), &fx, &both()).await.expect("apply");
        let json = serde_json::to_string(&ledger).expect("serialize");
        assert!(json.contains("\"outcome\":\"applied\""));
    }
}
```

- [ ] **Step 3: 모듈을 선언한다**

`crates/schoolx-catalog/src/lib.rs`에 추가한다.

```rust
pub mod ledger;
pub mod saga;
```

- [ ] **Step 4: 테스트가 통과하는지 확인한다**

Run: `cargo test -p schoolx-catalog`
Expected: PASS — 전체 테스트 통과. 특히 `second_apply_changes_nothing`, `canvas_failure_leaves_the_channel_and_retry_finishes_it`, `deleted_channel_is_not_recreated`가 Phase 3 완료 기준에 직접 대응한다.

- [ ] **Step 5: 커밋한다**

```bash
git add crates/schoolx-catalog
git commit -m "feat(schoolx-2): 세션 D — idempotent saga와 result ledger"
```

---

## Task 8: Tauri effects 구현과 command

**Files:**
- Create: `desktop/src-tauri/src/commands/workspace_catalog.rs`
- Modify: `desktop/src-tauri/Cargo.toml`
- Modify: `desktop/src-tauri/src/commands/mod.rs`
- Modify: `desktop/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `schoolx_catalog::{builtin, preflight::preflight, saga::apply, effects::CatalogEffects}`
- Produces: Tauri command `preflight_workspace_catalog() -> Vec<PreflightItemDto>`, `apply_workspace_catalog(selected: Vec<String>) -> Ledger`

확인된 기존 시그니처만 쓴다 — 새 헬퍼를 만들지 않는다.

| 호출 대상 | 위치 | 시그니처 |
|---|---|---|
| `query_relay`, `submit_event_with_keys`, `relay_api_base_url_with_override` | `crate::relay` | `channels.rs` 상단 import 그대로 |
| `get_channels` | `crate::commands::channels` | `(State<'_, AppState>) -> Result<Vec<ChannelInfo>, String>` |
| `get_channel_members` | `crate::commands::channels` | `(String, State<'_, AppState>) -> Result<ChannelMembersResponse, String>` |
| `set_canvas` | `crate::commands::canvas` | `(String, String, State<'_, AppState>) -> Result<serde_json::Value, String>` |
| `build_create_channel` | `crate::events` | `(Uuid, &str, &str, &str, Option<&str>, Option<i32>)` |

셋 다 `State<'_, AppState>`를 받는 Tauri command다. `ensure_starter_channels`가 `get_channels(state.clone())`으로 내부 호출하는 것과 같은 방식을 쓴다.

- [ ] **Step 1: 의존성을 더한다**

`desktop/src-tauri/Cargo.toml`의 `buzz_sdk_pkg` 줄 아래에 추가한다.

```toml
schoolx_catalog_pkg = { package = "schoolx-catalog", path = "../../crates/schoolx-catalog" }
```

`desktop/src-tauri`는 워크스페이스에서 `exclude`돼 있지만 path 의존은 그대로 동작한다.

- [ ] **Step 2: effects 구현과 command를 쓴다**

`desktop/src-tauri/src/commands/workspace_catalog.rs`를 만든다. 구조는 다음과 같다 — 각 메서드는 같은 디렉터리의 기존 채널 명령이 쓰는 헬퍼를 호출한다.

```rust
//! SchoolX 워크스페이스 catalog 적용.
//!
//! `schoolx-catalog` 크레이트의 saga를 실제 relay에 연결한다. 판정과 순서는
//! 전부 크레이트 쪽에 있고 여기에는 I/O만 있다.

use schoolx_catalog_pkg::effects::{
    CatalogEffects, ChannelRef, ChannelSpec, CreateOutcome, EffectError,
};
use schoolx_catalog_pkg::ledger::Ledger;
use schoolx_catalog_pkg::preflight::PreflightItem;
use schoolx_catalog_pkg::provenance::{Provenance, KIND_WORKSPACE_PROVENANCE};
use tauri::State;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::relay::{query_relay, relay_api_base_url_with_override, submit_event_with_keys};

struct RelayEffects<'a> {
    state: State<'a, AppState>,
}

#[async_trait::async_trait]
impl CatalogEffects for RelayEffects<'_> {
    async fn relay_scope(&self) -> String {
        relay_api_base_url_with_override(&self.state)
    }

    async fn list_channels(&self) -> Result<Vec<ChannelRef>, EffectError> {
        let channels = crate::commands::channels::get_channels(self.state.clone())
            .await
            .map_err(EffectError)?;
        Ok(channels
            .into_iter()
            .filter_map(|c| {
                Uuid::parse_str(&c.id)
                    .ok()
                    .map(|id| ChannelRef { id, name: c.name })
            })
            .collect())
    }

    async fn fetch_provenance(&self, catalog_id: &str) -> Result<Vec<Provenance>, EffectError> {
        let events = query_relay(
            &self.state,
            &[serde_json::json!({
                "kinds": [KIND_WORKSPACE_PROVENANCE],
                "limit": 200
            })],
        )
        .await
        .map_err(EffectError)?;

        Ok(events
            .iter()
            .filter_map(|ev| serde_json::from_str::<Provenance>(&ev.content).ok())
            .filter(|p| p.catalog_id == catalog_id)
            .collect())
    }

    async fn create_channel(&self, spec: ChannelSpec) -> Result<CreateOutcome, EffectError> {
        let keys = self.state.signing_keys().map_err(EffectError)?;
        let builder = crate::events::build_create_channel(
            spec.id,
            &spec.name,
            spec.visibility.as_str(),
            &spec.channel_type,
            Some(&spec.description),
            None,
        )
        .map_err(EffectError)?;

        match submit_event_with_keys(builder, &self.state, &keys, None).await {
            Ok(_) => {
                self.state
                    .mark_pending_owned_channel(&keys.public_key().to_hex(), &spec.id.to_string());
                Ok(CreateOutcome::Created)
            }
            Err(error) if error.contains("duplicate: channel already exists") => {
                Ok(CreateOutcome::Duplicate)
            }
            Err(error) => Err(EffectError(error)),
        }
    }

    async fn set_canvas(&self, channel_id: Uuid, content: &str) -> Result<(), EffectError> {
        crate::commands::canvas::set_canvas(
            channel_id.to_string(),
            content.to_string(),
            self.state.clone(),
        )
        .await
        .map(|_| ())
        .map_err(EffectError)
    }

    async fn is_owner(&self, channel_id: Uuid) -> Result<bool, EffectError> {
        let keys = self.state.signing_keys().map_err(EffectError)?;
        let me = keys.public_key().to_hex();
        let response = crate::commands::channels::get_channel_members(
            channel_id.to_string(),
            self.state.clone(),
        )
        .await
        .map_err(EffectError)?;
        Ok(response
            .members
            .iter()
            .any(|m| m.pubkey == me && (m.role == "owner" || m.role == "admin")))
    }

    async fn publish_provenance(
        &self,
        channel_id: Uuid,
        provenance: &Provenance,
    ) -> Result<(), EffectError> {
        let keys = self.state.signing_keys().map_err(EffectError)?;
        let content = serde_json::to_string(provenance)
            .map_err(|e| EffectError(format!("provenance 직렬화 실패: {e}")))?;
        let builder = nostr::EventBuilder::new(
            nostr::Kind::Custom(KIND_WORKSPACE_PROVENANCE as u16),
            content,
        )
        .tags(vec![
            nostr::Tag::parse(["d", &provenance.d_tag()])
                .map_err(|e| EffectError(format!("d 태그: {e}")))?,
            nostr::Tag::parse(["h", &channel_id.to_string()])
                .map_err(|e| EffectError(format!("h 태그: {e}")))?,
        ]);

        submit_event_with_keys(builder, &self.state, &keys, None)
            .await
            .map(|_| ())
            .map_err(EffectError)
    }

    async fn now_rfc3339(&self) -> String {
        chrono::Utc::now().to_rfc3339()
    }
}

/// catalog 적용 전 항목별 판정을 돌려준다.
#[tauri::command]
pub async fn preflight_workspace_catalog(
    state: State<'_, AppState>,
) -> Result<Vec<PreflightItem>, String> {
    let effects = RelayEffects { state };
    schoolx_catalog_pkg::preflight::preflight(schoolx_catalog_pkg::builtin(), &effects)
        .await
        .map_err(|e| e.0)
}

/// 선택한 catalog 항목을 적용하고 result ledger를 돌려준다.
#[tauri::command]
pub async fn apply_workspace_catalog(
    selected: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Ledger, String> {
    let effects = RelayEffects { state };
    schoolx_catalog_pkg::saga::apply(schoolx_catalog_pkg::builtin(), &effects, &selected)
        .await
        .map_err(|e| e.0)
}
```

**두 가지만 확인하고 넘어간다.**

1. `crate::commands::canvas::set_canvas`와 `channels::{get_channels, get_channel_members}`는 `#[tauri::command]`이지만 보통의 함수이기도 하다. 모듈 밖에서 부르려면 `pub`이어야 하는데 셋 다 이미 `pub async fn`이다.
2. `Provenance`가 `chrono` 없이 시각을 문자열로만 다루므로, `now_rfc3339`에 쓸 `chrono`가 `desktop/src-tauri/Cargo.toml`에 이미 있는지 확인한다. 없으면 `std::time::SystemTime`으로 RFC 3339를 만든다.

- [ ] **Step 3: command를 등록한다**

`desktop/src-tauri/src/commands/mod.rs`에 추가한다.

```rust
pub mod workspace_catalog;
```

`desktop/src-tauri/src/lib.rs`의 `invoke_handler` 목록에서 `ensure_starter_channels,` 다음 줄에 추가한다.

```rust
            preflight_workspace_catalog,
            apply_workspace_catalog,
```

같은 파일 위쪽의 `use commands::{...}` 목록에도 두 이름을 더한다.

- [ ] **Step 4: 빌드하고 테스트한다**

Run: `cargo test --manifest-path desktop/src-tauri/Cargo.toml`
Expected: PASS — 컴파일되고 기존 테스트가 통과

- [ ] **Step 5: 포맷을 맞춘다**

Run: `just desktop-tauri-fmt`
Expected: 변경 없음 또는 자동 수정. 워크트리가 아닌 메인 체크아웃에서 돌려야 한다.

- [ ] **Step 6: 커밋한다**

```bash
git add desktop/src-tauri
git commit -m "feat(schoolx-2): 세션 D — catalog 적용 Tauri command"
```

---

## Task 9: 설정 화면 카드와 i18n

**Files:**
- Create: `desktop/src/shared/api/tauriWorkspaceCatalog.ts`
- Create: `desktop/src/features/workspace-catalog/hooks.ts`
- Create: `desktop/src/features/settings/ui/WorkspaceCatalogSettingsCard.tsx`
- Modify: `desktop/src/shared/i18n/locales/en.ts`
- Modify: `desktop/src/shared/i18n/locales/ko.ts`
- Modify: `desktop/src/shared/i18n/resources.ts`
- Modify: `desktop/src/features/settings/ui/SettingsPanels.tsx`
- Modify: `desktop/src/features/settings/ui/SettingsView.tsx`

**Interfaces:**
- Consumes: Tauri command `preflight_workspace_catalog`, `apply_workspace_catalog`
- Produces: 설정 섹션 `workspace-catalog`

- [ ] **Step 1: i18n 네임스페이스를 세 곳에 한 번에 추가한다**

`desktop/src/shared/i18n/locales/en.ts`에 최상위 키를 더한다.

```ts
  catalog: {
    title: "SchoolX default workspace",
    description:
      "Create the standard SchoolX rooms. Nothing is created until you apply.",
    apply: "Apply selected",
    applying: "Applying…",
    openWarningScope:
      "Every signed-in user can read and write without being a member.",
    openWarningAgents:
      "Managed agents still need to be added explicitly before they can join.",
    decision: {
      create_or_recreate: "Will be created",
      resume: "Will resume",
      no_change: "Already applied",
      conflict: "Needs your decision",
      retired: "No longer offered",
      deleted: "Previously deleted",
    },
    outcome: {
      applied: "Applied",
      unchanged: "Unchanged",
      partial: "Partly applied",
      blocked: "Needs your decision",
    },
    userAction: {
      confirm_recreate:
        "You deleted this room before. Create it again?",
      resolve_conflict:
        "A room with this name already exists. SchoolX will not adopt it automatically.",
    },
    renamed: "Renamed by a member",
  },
```

`desktop/src/shared/i18n/locales/ko.ts`에 같은 구조로 더한다.

```ts
  catalog: {
    title: "SchoolX 기본 워크스페이스",
    description:
      "SchoolX 표준 업무방을 만듭니다. 적용을 누르기 전에는 아무것도 생성되지 않습니다.",
    apply: "선택 항목 적용",
    applying: "적용하는 중…",
    openWarningScope:
      "모든 로그인 사용자가 멤버가 아니어도 읽고 쓸 수 있습니다.",
    openWarningAgents:
      "관리형 에이전트는 명시적으로 추가된 경우에만 접근합니다.",
    decision: {
      create_or_recreate: "새로 만듭니다",
      resume: "이어서 진행합니다",
      no_change: "이미 적용됨",
      conflict: "확인이 필요합니다",
      retired: "더는 제공하지 않습니다",
      deleted: "이전에 삭제됨",
    },
    outcome: {
      applied: "적용 완료",
      unchanged: "변경 없음",
      partial: "일부만 적용",
      blocked: "확인이 필요합니다",
    },
    userAction: {
      confirm_recreate: "이전에 삭제한 방입니다. 다시 만들까요?",
      resolve_conflict:
        "같은 이름의 방이 이미 있습니다. SchoolX가 임의로 채택하지 않습니다.",
    },
    renamed: "멤버가 이름을 변경함",
  },
```

`desktop/src/shared/i18n/resources.ts`의 `APP_I18N_NAMESPACES`에 더한다.

```ts
  "catalog",
```

세 곳을 한 번에 바꾸지 않으면 한국어에서 `catalog.title` 같은 원시 키가 화면에 뜬다. `fallbackLng`가 구제하지 못한다.

- [ ] **Step 2: 네임스페이스 parity 테스트를 돌린다**

Run: `pnpm --dir desktop test resources`
Expected: PASS — `en`/`ko` 키 구조가 일치

- [ ] **Step 3: API 래퍼를 쓴다**

`desktop/src/shared/api/tauriWorkspaceCatalog.ts`:

```ts
import { invokeTauri } from "@/shared/api/tauri";

export type CatalogDecision =
  | "create_or_recreate"
  | "resume"
  | "no_change"
  | "conflict"
  | "retired"
  | "deleted";

export type CatalogOutcome = "applied" | "unchanged" | "partial" | "blocked";

export type CatalogUserAction = "confirm_recreate" | "resolve_conflict";

export type CatalogPreflightItem = {
  item_key: string;
  decision: CatalogDecision;
  channel_id: string | null;
  generation: number;
  renamed: boolean;
};

export type CatalogStepStatus = "pending" | "done" | "failed";

export type CatalogStepStates = {
  channel: CatalogStepStatus;
  canvas: CatalogStepStatus;
  membership: CatalogStepStatus;
};

export type CatalogLedgerItem = {
  item_key: string;
  decision: CatalogDecision;
  channel_id: string | null;
  generation: number;
  steps: CatalogStepStates;
  outcome: CatalogOutcome;
  user_action: CatalogUserAction | null;
  error: string | null;
};

export type CatalogLedger = {
  catalog_id: string;
  catalog_version: number;
  items: CatalogLedgerItem[];
};

export async function preflightWorkspaceCatalog(): Promise<
  CatalogPreflightItem[]
> {
  return invokeTauri<CatalogPreflightItem[]>("preflight_workspace_catalog");
}

export async function applyWorkspaceCatalog(
  selected: string[],
): Promise<CatalogLedger> {
  return invokeTauri<CatalogLedger>("apply_workspace_catalog", { selected });
}
```

- [ ] **Step 4: 설정 섹션을 등록한다**

`desktop/src/features/settings/ui/SettingsPanels.tsx`에서 네 곳을 바꾼다.

`SettingsSection` 유니온에 `"channel-templates"` 다음 줄로 추가한다.

```ts
  | "workspace-catalog"
```

`SETTINGS_SECTION_VALUES` 배열에 `"channel-templates",` 다음 줄로 추가한다.

```ts
  "workspace-catalog",
```

섹션 서술자 배열에서 `channel-templates` 항목 다음에 추가한다.

```tsx
  {
    value: "workspace-catalog",
    label: "settings.sections.workspaceCatalog",
    icon: LayoutTemplate,
  },
```

패널 `switch`에서 `case "channel-templates":` 다음에 추가한다.

```tsx
    case "workspace-catalog":
      return <WorkspaceCatalogSettingsCard />;
```

파일 상단 import에 더한다.

```ts
import { WorkspaceCatalogSettingsCard } from "./WorkspaceCatalogSettingsCard";
```

`desktop/src/shared/i18n/locales/{en,ko}.ts`의 `settings.sections`에 `workspaceCatalog` 키를 더한다 (`en`: `"SchoolX workspace"`, `ko`: `"SchoolX 워크스페이스"`).

`desktop/src/features/settings/ui/SettingsView.tsx`의 섹션 그룹에서 `"channel-templates"` 옆에 `"workspace-catalog"`를 더한다.

- [ ] **Step 5: 카드 컴포넌트를 쓴다**

`desktop/src/features/settings/ui/WorkspaceCatalogSettingsCard.tsx`:

```tsx
import { useMutation, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import {
  applyWorkspaceCatalog,
  type CatalogLedger,
  type CatalogPreflightItem,
  preflightWorkspaceCatalog,
} from "@/shared/api/tauriWorkspaceCatalog";

/** 사용자가 손대면 안 되는 판정 — 이미 끝났거나 더는 제공하지 않는 항목. */
const LOCKED_DECISIONS = new Set(["no_change", "retired"]);

export function WorkspaceCatalogSettingsCard() {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [ledger, setLedger] = useState<CatalogLedger | null>(null);

  const preflight = useQuery({
    queryKey: ["workspace-catalog", "preflight"],
    queryFn: preflightWorkspaceCatalog,
  });

  const apply = useMutation({
    mutationFn: (keys: string[]) => applyWorkspaceCatalog(keys),
    onSuccess: async (result) => {
      setLedger(result);
      await preflight.refetch();
    },
  });

  function toggle(item: CatalogPreflightItem) {
    if (LOCKED_DECISIONS.has(item.decision)) return;
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(item.item_key)) next.delete(item.item_key);
      else next.add(item.item_key);
      return next;
    });
  }

  const items = preflight.data ?? [];

  return (
    <section className="min-w-0" data-testid="settings-workspace-catalog">
      <h2 className="font-semibold text-base">{t("catalog.title")}</h2>
      <p className="mt-2 text-gray-600 text-sm dark:text-gray-400">
        {t("catalog.description")}
      </p>

      <ul className="mt-6 space-y-3">
        {items.map((item) => {
          const locked = LOCKED_DECISIONS.has(item.decision);
          const result = ledger?.items.find(
            (entry) => entry.item_key === item.item_key,
          );
          return (
            <li
              key={item.item_key}
              className="rounded-2xl border border-gray-200 p-4 dark:border-gray-700"
              data-testid={`catalog-item-${item.item_key}`}
            >
              <label className="flex items-start gap-3">
                <input
                  type="checkbox"
                  className="mt-1"
                  checked={selected.has(item.item_key)}
                  disabled={locked}
                  onChange={() => toggle(item)}
                />
                <span className="min-w-0">
                  <span className="block font-medium text-base">
                    {item.item_key}
                  </span>
                  <span className="mt-1 block text-gray-600 text-sm dark:text-gray-400">
                    {t(`catalog.decision.${item.decision}`)}
                  </span>
                  {item.renamed ? (
                    <span className="mt-1 block text-2xs text-gray-500">
                      {t("catalog.renamed")}
                    </span>
                  ) : null}
                </span>
              </label>

              {result ? (
                <div className="mt-3 text-sm">
                  <p>{t(`catalog.outcome.${result.outcome}`)}</p>
                  {result.user_action ? (
                    <p className="mt-1 text-amber-700 dark:text-amber-500">
                      {t(`catalog.userAction.${result.user_action}`)}
                    </p>
                  ) : null}
                  {result.error ? (
                    <p className="mt-1 text-red-700 dark:text-red-400">
                      {result.error}
                    </p>
                  ) : null}
                </div>
              ) : null}
            </li>
          );
        })}
      </ul>

      <button
        type="button"
        className="mt-6 rounded-full bg-gray-900 px-5 py-2 font-medium text-sm text-white disabled:opacity-50 dark:bg-gray-100 dark:text-gray-900"
        disabled={selected.size === 0 || apply.isPending}
        onClick={() => apply.mutate([...selected])}
        data-testid="catalog-apply"
      >
        {apply.isPending ? t("catalog.applying") : t("catalog.apply")}
      </button>
    </section>
  );
}
```

**공개 범위 경고는 아직 붙이지 않는다.** 내장 catalog 두 항목이 모두 `private`이고 이번 세션에는 공개 범위를 바꾸는 UI가 없다. `open` 선택 UI를 더하는 작업에서 `t("catalog.openWarningScope")`와 `t("catalog.openWarningAgents")` **두 문장을 함께** 붙인다 — 문자열은 Step 1에서 이미 준비해 뒀다. 한 문장만 띄우면 스펙 §9 위반이다.

카드 구조와 여백은 같은 디렉터리의 `ChannelTemplatesSettingsCard.tsx`를 참고해 맞춘다. 텍스트 크기는 rem 토큰만 쓴다 — `pnpm check:px-text`가 임의 리터럴을 막는다.

- [ ] **Step 6: 프론트엔드 검사를 돌린다**

Run: `pnpm --dir desktop typecheck && pnpm --dir desktop check && pnpm --dir desktop test`
Expected: PASS — 타입·린트·테스트 전부 통과. `pnpm check:px-text`가 임의 텍스트 크기 리터럴을 잡으면 rem 토큰으로 바꾼다.

- [ ] **Step 7: 커밋한다**

```bash
git add desktop/src
git commit -m "feat(schoolx-2): 세션 D — 워크스페이스 catalog 설정 화면"
```

---

## Task 10: 읽기 전용 CLI

**Files:**
- Create: `crates/buzz-cli/src/commands/workspace_catalog.rs`
- Modify: `crates/buzz-cli/src/commands/mod.rs`
- Modify: `crates/buzz-cli/Cargo.toml`
- Modify: `crates/buzz-cli/src/main.rs` (또는 clap 서브커맨드가 선언된 파일)

**Interfaces:**
- Consumes: `schoolx_catalog::builtin()`
- Produces: `buzz catalog list`

적용은 데스크톱에만 둔다. CLI는 공유 크레이트가 실제로 공유되는지 검증하는 역할이다.

- [ ] **Step 1: 의존성을 더한다**

`crates/buzz-cli/Cargo.toml`의 `[dependencies]`에 추가한다.

```toml
schoolx-catalog = { path = "../schoolx-catalog" }
```

- [ ] **Step 2: 서브커맨드를 쓴다**

`crates/buzz-cli/src/commands/workspace_catalog.rs`:

```rust
//! `buzz catalog` — 읽기 전용 워크스페이스 catalog 조회.
//!
//! 적용은 데스크톱에만 있다. 여기서는 데스크톱과 CLI가 같은 컴파일 내장
//! 정의를 읽는다는 것을 확인한다.

use serde::Serialize;

use crate::error::CliError;

#[derive(Serialize)]
struct CatalogItemOut<'a> {
    item_key: &'a str,
    name: &'a str,
    description: &'a str,
    channel_type: &'a str,
    visibility: &'a str,
}

/// 내장 catalog 항목을 JSON 배열로 출력한다.
pub fn list() -> Result<(), CliError> {
    let catalog = schoolx_catalog::builtin();
    let items: Vec<CatalogItemOut<'_>> = catalog
        .items
        .iter()
        .map(|item| CatalogItemOut {
            item_key: &item.item_key,
            name: &item.name,
            description: &item.description,
            channel_type: &item.channel_type,
            visibility: item.visibility.as_str(),
        })
        .collect();

    let json = serde_json::to_string_pretty(&items)
        .map_err(|e| CliError::Other(format!("catalog 직렬화 실패: {e}")))?;
    println!("{json}");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn cli_reads_the_same_builtin_catalog() {
        let catalog = schoolx_catalog::builtin();
        assert_eq!(catalog.catalog_id, "schoolx.default");
        let keys: Vec<&str> = catalog.items.iter().map(|i| i.item_key.as_str()).collect();
        assert_eq!(keys, vec!["meeting", "planning"]);
    }
}
```

**구현 시 확인할 것:** `CliError`의 실제 variant 이름을 `crates/buzz-cli/src/error.rs`에서 확인하고 맞춘다. `Other`가 없으면 같은 파일의 다른 명령이 쓰는 일반 실패 variant를 쓴다.

- [ ] **Step 3: 서브커맨드를 등록한다**

`crates/buzz-cli/src/commands/mod.rs`에 추가한다.

```rust
pub mod workspace_catalog;
```

clap 정의는 `crates/buzz-cli/src/lib.rs`에 있다 (`main.rs`는 4줄짜리 진입점이다). 거기 명령 enum에 서브커맨드를 더한다.

```rust
    /// SchoolX 기본 워크스페이스 catalog 조회
    Catalog {
        #[command(subcommand)]
        command: CatalogCommand,
    },
```

```rust
#[derive(clap::Subcommand)]
enum CatalogCommand {
    /// 내장 catalog 항목을 출력한다
    List,
}
```

디스패치에서 부른다.

```rust
        Commands::Catalog { command } => match command {
            CatalogCommand::List => commands::workspace_catalog::list(),
        },
```

enum·디스패치의 정확한 이름(`Commands` 등)은 `lib.rs`의 기존 서브커맨드를 보고 맞춘다.

- [ ] **Step 4: 테스트하고 직접 돌려본다**

Run: `cargo test -p buzz-cli && cargo run -p buzz-cli -- catalog list`
Expected: PASS, 그리고 `meeting`·`planning` 두 항목이 담긴 JSON 배열이 출력된다

- [ ] **Step 5: 커밋한다**

```bash
git add crates/buzz-cli Cargo.lock
git commit -m "feat(schoolx-2): 세션 D — 읽기 전용 catalog CLI"
```

---

## Task 11: live relay E2E

**Files:**
- Create: `crates/buzz-test-client/tests/e2e_workspace_catalog.rs`

**Interfaces:**
- Consumes: kind 39500 (Task 4), `schoolx_catalog::provenance` (Task 3)
- Produces: 없음

여기서만 확인할 수 있는 것: relay가 kind 39500을 실제로 받는가, 채널 스코프 ACL이 비멤버를 막는가, soft-delete된 채널 ID로 재생성이 거부되는가. 나머지는 전부 Task 7의 단위 테스트가 이미 덮는다.

- [ ] **Step 1: 테스트를 쓴다**

`crates/buzz-test-client/tests/e2e_workspace_catalog.rs`를 만든다.

**먼저 `e2e_access_matrix.rs`를 읽고 그 하네스를 그대로 재사용한다** — relay 기동, 키 생성, NIP-42 인증, 채널 생성, 이벤트 발행, REQ 헬퍼가 이미 거기 있다. 아래 네 테스트를 그 헬퍼 이름에 맞춰 쓴다. 새 하네스를 만들지 않는다.

| 테스트 | 시나리오 | 단언 |
|---|---|---|
| `provenance_round_trips_through_the_relay` | private 채널 생성 → kind 39500 발행 (`d = "schoolx.default:meeting"`, `h = <channel_id>`, content = `Provenance` JSON) → 같은 사용자가 `{"kinds":[39500],"#d":["schoolx.default:meeting"]}`로 REQ | 이벤트 1개, content가 발행한 것과 동일 |
| `second_publish_replaces_the_first` | 같은 `d`로 `steps`만 바꿔 다시 발행 → 같은 REQ | 이벤트가 **1개**이고 `steps`가 두 번째 값 (NIP-33 LWW) |
| `non_member_cannot_read_provenance` | 다른 인증 사용자가 같은 REQ | 결과가 **비어 있음**. 새면 세션 A 계약이 깨진다 |
| `deleted_channel_id_is_burned` | 채널 생성 → kind 9008로 삭제 → 같은 UUID로 kind 9007 재발행 | `duplicate: channel already exists`로 거부. 이게 삭제 감지의 근거다 |

content에 넣을 JSON은 Task 3의 `Provenance` 구조 그대로다.

```json
{
  "catalog_id": "schoolx.default",
  "catalog_version": 1,
  "item_key": "meeting",
  "generation": 1,
  "steps": { "channel": "done", "canvas": "pending", "membership": "pending" },
  "applied_at": "2026-07-28T09:00:00Z"
}
```

`buzz-test-client`가 `schoolx-catalog`에 의존할 필요는 없다 — JSON 리터럴로 충분하고, 오히려 relay가 받는 것이 크레이트 타입과 무관하게 검증된다.

- [ ] **Step 2: 인프라를 띄우고 돌린다**

Run: `just test-e2e e2e_workspace_catalog`
Expected: PASS — 3 tests. live relay E2E는 Postgres·Redis·MinIO가 필요하며 `just test-e2e`가 띄운다.

- [ ] **Step 3: 커밋한다**

```bash
git add crates/buzz-test-client
git commit -m "test(schoolx-2): 세션 D — catalog provenance live relay E2E"
```

---

## Task 12: 문서 갱신과 최종 게이트

**Files:**
- Modify: `docs/schoolx-2/IMPLEMENTATION_HANDOFF.md`
- Modify: `docs/schoolx-2/BASELINE.md`
- Modify: `docs/schoolx-2/WORKSPACE_CATALOG.md`

- [ ] **Step 1: 전체 게이트를 돌린다**

Run: `just ci`
Expected: PASS — fmt + clippy + 데스크톱 lint + 단위 테스트 + 빌드

실패하면 고친 뒤 다시 돌린다. clippy 통과가 fmt 통과를 뜻하지 않으므로 둘 다 확인한다.

- [ ] **Step 2: 보안 계약이 깨지지 않았는지 확인한다**

Run: `just test-e2e e2e_access_matrix`
Expected: PASS — 17 tests. 이 계약에는 CI job이 없어서 사람이 돌릴 때만 돈다.

- [ ] **Step 3: 제품 식별자 검사를 돌린다**

Run: `just schoolx-upstream-check`
Expected: PASS — 새로 추가한 경로에 `xyz.block.buzz.app`, `~/.buzz`, `buzz://`, `"Buzz"` 리터럴이 없어야 한다

- [ ] **Step 4: 핸드오프 문서를 갱신한다**

`docs/schoolx-2/IMPLEMENTATION_HANDOFF.md`에서:

- "현재 구현 snapshot"의 Phase 상태에 Phase 3 진행 상황을 더한다.
- "구현되어 있는 것"에 내장 catalog, kind 39500 provenance, idempotent saga, result ledger를 더한다.
- "아직 구현 또는 검증되지 않은 것"에서 "versioned workspace catalog, provenance, idempotent saga"를 남은 범위(나머지 8개 업무방, 에이전트 provisioning)로 좁힌다.
- "### 세션 D" 절에 세션 A·B·C와 같은 형식으로 **완료 표시와 이후 세션이 전제해야 할 사실**을 적는다. 최소한 다음 세 가지를 담는다.
  1. relay는 kind 39000을 DB 컬럼에서만 재구성한다. 채널 생성 이벤트에 실은 태그는 보존되지 않는다.
  2. 채널 삭제는 soft delete이고 조회가 `deleted_at`으로 걸러지므로 삭제된 채널의 provenance는 읽을 수 없다. 삭제 감지는 `ON CONFLICT DO NOTHING`이 채널 ID를 영구히 태운다는 사실에 기댄다.
  3. SchoolX 전용 Nostr kind는 예약 대역 `39500–39599`를 쓴다. 이유는 마이그레이션 `9001+`와 같다.
- 세션 D에서 넘긴 것을 적는다: 나머지 8개 업무방 콘텐츠(낮은 추론), 에이전트 provisioning(세션 E), CLI 적용 경로.

- [ ] **Step 5: BASELINE을 갱신한다**

`docs/schoolx-2/BASELINE.md`에 이번 세션에서 돌린 명령과 결과를 기록한다: `cargo test -p schoolx-catalog`, `just test-e2e e2e_workspace_catalog`, `just test-e2e e2e_access_matrix`, `just ci`. 실행 시각과 통과·실패 수를 함께 남긴다.

- [ ] **Step 6: Phase 3 완료 기준을 대조한다**

`DEVELOPMENT_PLAN.md` Phase 3의 완료 기준 7개를 하나씩 증거와 대조한다.

| 기준 | 증거 |
|---|---|
| 선택한 항목만 생성 | `only_selected_items_are_applied` |
| 두 번째 적용은 변경 없음 | `second_apply_changes_nothing` |
| 이름을 바꿔도 추적 | `rename_is_a_flag_not_a_decision` |
| provenance 없는 동명 채널 자동 채택 금지 | `name_conflict_blocks_without_touching_anything` |
| 단계 실패 후 재시도가 중복 없이 도달 | `canvas_failure_leaves_the_channel_and_retry_finishes_it`, `channel_failure_retries_from_the_start_without_duplicates` |
| upgrade가 사용자 수정본을 덮어쓰지 않음 | `item_dropped_from_catalog_is_retired`, catalog가 읽기 전용이라 사용자 저장소를 건드리지 않음 |
| 상태가 UI와 machine-readable 결과에 표시 | `ledger_serializes_for_ui_and_cli` + Task 9 카드 |

**증거가 없는 기준이 하나라도 있으면 Phase 3을 완료로 표시하지 않는다.** 대신 무엇이 남았는지 핸드오프에 적는다.

- [ ] **Step 7: 스펙에 구현 결과를 반영한다**

`docs/schoolx-2/WORKSPACE_CATALOG.md`에서 구현 중 설계와 달라진 부분이 있으면 고친다. 특히 §12의 코드 경로 표에 새로 만든 파일을 더한다.

- [ ] **Step 8: 커밋한다**

```bash
git add docs/schoolx-2
git commit -m "docs(schoolx-2): 세션 D — 워크스페이스 catalog 구현 결과 기록"
```
