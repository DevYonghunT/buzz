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
///
/// `catalog_version`은 의도적으로 입력에서 제외한다 — catalog 버전이 올라가도
/// 기존 방(예: `meeting`)의 ID가 바뀌면 안 된다. 자세한 내용은
/// `docs/schoolx-2/WORKSPACE_CATALOG.md` §5 참고.
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
    fn catalog_id_changes_the_id() {
        let a = derive_channel_id("wss://relay.example", "schoolx.default", "meeting", 1);
        let b = derive_channel_id("wss://relay.example", "other.catalog", "meeting", 1);
        assert_ne!(a, b);
    }

    #[test]
    fn relay_scope_is_trimmed() {
        let a = derive_channel_id("  wss://relay.example  ", "schoolx.default", "meeting", 1);
        let b = derive_channel_id("wss://relay.example", "schoolx.default", "meeting", 1);
        assert_eq!(a, b);
    }

    /// Golden-value regression test. Pins the exact UUID produced for a
    /// fixed set of inputs so a change to the derivation formula (argument
    /// order in the `format!` call, the `:` separator, the `v1` tag, or the
    /// namespace bytes) fails loudly here instead of passing silently
    /// through the relational tests above.
    ///
    /// Changing this expected literal is a breaking change: it means the
    /// derivation changed and every already-created channel derived under
    /// the old formula is now orphaned (its stored channel ID no longer
    /// matches what this function would derive for the same catalog item).
    /// Do not update this literal to make a failing test pass — treat a
    /// failure here as a regression to fix, not a value to re-pin.
    #[test]
    fn golden_value_matches_known_uuid() {
        let id = derive_channel_id("wss://relay.example", "schoolx.default", "meeting", 1);
        assert_eq!(
            id.to_string(),
            "51130765-2dea-57e7-9162-1c902691325c",
            "golden UUID changed for fixed inputs (relay_scope=\"wss://relay.example\", \
             catalog_id=\"schoolx.default\", item_key=\"meeting\", generation=1) — \
             this orphans existing channels, see doc comment above"
        );
    }
}
