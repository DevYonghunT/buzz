//! `buzz catalog` — 읽기 전용 워크스페이스 catalog 조회.
//!
//! 적용은 데스크톱에만 있다. 여기서는 데스크톱과 CLI가 같은 컴파일 내장
//! 정의(`schoolx_catalog::builtin()`)를 읽는다는 것을 확인한다. relay
//! 접속도, 인증도, 쓰기도 하지 않는다.

use serde::Serialize;

use crate::error::CliError;
use crate::OutputFormat;

/// `buzz catalog list`가 출력하는 catalog 항목 하나.
///
/// `schoolx_catalog::CatalogItem`의 모든 필드를 그대로 옮긴다. 아래 `render`
/// 함수 내 exhaustive destructuring에서 CatalogItem의 각 필드를 명시적으로
/// 이름 지어 매핑한다. 새 필드가 추가되면 패턴 매칭이 실패하므로, 이 struct을
/// 맞춰 수정해야 한다는 컴파일 오류가 강제된다.
#[derive(Serialize)]
struct CatalogItemOut<'a> {
    item_key: &'a str,
    name: &'a str,
    description: &'a str,
    channel_type: &'a str,
    visibility: &'a str,
    canvas: &'a str,
}

/// `--format compact`용 축약 표현 — 항목을 식별하는 데 필요한 최소한만
/// 담는다. `channels list --format compact`가 `channel_id`+`name`으로
/// 줄이는 것과 같은 규칙이다.
#[derive(Serialize)]
struct CatalogItemCompactOut<'a> {
    item_key: &'a str,
    name: &'a str,
}

/// `format`에 맞춰 내장 catalog를 JSON 문자열로 렌더링한다.
///
/// 순수 함수다 — relay나 인증이 필요 없는 이유는 `schoolx_catalog::builtin()`
/// 자체가 앱에 컴파일되어 들어간 정적 데이터이기 때문이다.
fn render(catalog: &schoolx_catalog::Catalog, format: &OutputFormat) -> Result<String, CliError> {
    match format {
        OutputFormat::Compact => {
            let items: Vec<CatalogItemCompactOut<'_>> = catalog
                .items
                .iter()
                .map(|item| {
                    // Exhaustive destructuring: CatalogItem의 모든 필드를 명시적으로
                    // 이름 짓는다. 새 필드가 추가되면 패턴 매칭이 실패한다.
                    let schoolx_catalog::CatalogItem {
                        item_key,
                        name,
                        description: _,
                        channel_type: _,
                        visibility: _,
                        canvas: _,
                    } = item;
                    CatalogItemCompactOut { item_key, name }
                })
                .collect();
            serde_json::to_string(&items)
        }
        OutputFormat::Json => {
            let items: Vec<CatalogItemOut<'_>> = catalog
                .items
                .iter()
                .map(|item| {
                    // Exhaustive destructuring: CatalogItem의 모든 필드를 명시적으로
                    // 이름 짓는다. 새 필드가 추가되면 패턴 매칭이 실패한다.
                    let schoolx_catalog::CatalogItem {
                        item_key,
                        name,
                        description,
                        channel_type,
                        visibility,
                        canvas,
                    } = item;
                    CatalogItemOut {
                        item_key,
                        name,
                        description,
                        channel_type,
                        visibility: visibility.as_str(),
                        canvas,
                    }
                })
                .collect();
            serde_json::to_string(&items)
        }
    }
    .map_err(|e| CliError::Other(format!("catalog 직렬화 실패: {e}")))
}

/// 내장 catalog 항목을 JSON 배열로 출력한다.
///
/// relay 연결도 `BUZZ_PRIVATE_KEY`도 요구하지 않는다. 항목을 실제 채널에
/// 적용하는 것은 데스크톱 전용이다 (`apply_workspace_catalog`) — 이 명령은
/// 그 적용 대상이 되는 내장 정의를 읽기만 한다.
pub fn list(format: &OutputFormat) -> Result<(), CliError> {
    let catalog = schoolx_catalog::builtin();
    println!("{}", render(catalog, format)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_reads_the_same_builtin_catalog() {
        let catalog = schoolx_catalog::builtin();
        assert_eq!(catalog.catalog_id, "schoolx.default");
        let keys: Vec<&str> = catalog.items.iter().map(|i| i.item_key.as_str()).collect();
        assert_eq!(keys, vec!["meeting", "planning"]);
    }

    #[test]
    fn full_json_includes_every_catalog_item_field() {
        let out = render(schoolx_catalog::builtin(), &OutputFormat::Json).unwrap();
        let items: serde_json::Value = serde_json::from_str(&out).unwrap();
        let first = &items[0];
        for field in [
            "item_key",
            "name",
            "description",
            "channel_type",
            "visibility",
            "canvas",
        ] {
            assert!(
                first.get(field).is_some(),
                "expected full JSON to include `{field}`, got {first}"
            );
        }
    }

    #[test]
    fn full_json_contains_both_rooms_with_korean_names() {
        let out = render(schoolx_catalog::builtin(), &OutputFormat::Json).unwrap();
        assert!(out.contains("메인 회의방"), "missing meeting room name");
        assert!(out.contains("기획"), "missing planning room name");
    }

    #[test]
    fn compact_json_reduces_to_item_key_and_name() {
        let out = render(schoolx_catalog::builtin(), &OutputFormat::Compact).unwrap();
        let items: serde_json::Value = serde_json::from_str(&out).unwrap();
        let first = &items[0];
        assert!(first.get("item_key").is_some());
        assert!(first.get("name").is_some());
        assert!(
            first.get("canvas").is_none(),
            "compact output should not carry the (potentially large) canvas body"
        );
        assert!(first.get("description").is_none());
        assert!(first.get("channel_type").is_none());
        assert!(first.get("visibility").is_none());
    }
}
