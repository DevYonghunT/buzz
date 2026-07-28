//! SchoolX 워크스페이스 catalog — 읽기 전용 내장 정의, preflight 판정,
//! idempotent saga, machine-readable result ledger.
//!
//! 설계는 `docs/schoolx-2/WORKSPACE_CATALOG.md`.

pub mod catalog;
pub mod channel_id;
pub mod effects;
pub mod provenance;

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
