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
