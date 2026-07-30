import { invokeTauri } from "@/shared/api/tauri";

/**
 * 여덟 값이다. `preflight`는 앞의 다섯만 내고, `deleted`·`adopted`·`not_owned`는
 * 적용 시점에 saga가 확정한다. Rust 쪽 `Decision`의 serde 철자와
 * `ledger.rs`의 `*_DECISION` 상수가 같은 어휘를 이룬다
 * (`ledger_decision_vocabulary_is_pinned`가 그 순서와 철자를 고정한다).
 */
export type CatalogDecision =
  | "create_or_recreate"
  | "resume"
  | "no_change"
  | "conflict"
  | "retired"
  | "deleted"
  | "adopted"
  | "not_owned";

export type CatalogOutcome = "applied" | "unchanged" | "partial" | "blocked";

export type CatalogUserAction =
  | "confirm_recreate"
  | "resolve_conflict"
  | "request_ownership";

/**
 * catalog가 정한 표시 이름. 사람이 보는 자리에는 `item_key`가 아니라 이 값을
 * 쓴다 — 키는 내부 식별자라 그대로 렌더하면 `메인 회의방` 자리에 `meeting`이
 * 나온다.
 *
 * **`retired` 항목은 `null`이다.** 증명서는 남았는데 catalog 항목이 사라진
 * 경우라 이름이 남아 있는 곳이 없다. Rust 쪽이 일부러 `item_key`로 메우지
 * 않는다 (`crates/schoolx-catalog/src/preflight.rs`의 `PreflightItem::name`) —
 * 그러면 "이게 이름이다"와 "이름을 모른다"가 같은 값이 되기 때문이다. 여기서
 * `?? item.item_key`로 되돌리면 그 구별을 다시 없애는 것이니 하지 말 것.
 * 이름 조회를 TS에서 따로 하지도 않는다: catalog의 단일 소스는 Rust다.
 */
type CatalogItemName = string | null;

export type CatalogPreflightItem = {
  item_key: string;
  name: CatalogItemName;
  decision: CatalogDecision;
  channel_id: string | null;
  channel_present: boolean;
  generation: number;
  steps: CatalogStepStates;
  renamed: boolean;
};

/**
 * `crates/schoolx-catalog/src/provenance.rs`의 `StepStatus`.
 *
 * 네 값이다 — `"skipped"`는 캔버스 단계에만 쓰인다. saga가 쓰기 전에 그
 * 방의 현재 캔버스를 읽어(`read_canvas` guard), 지켜야 할 사용자 내용이
 * 이미 있으면 캔버스를 덮어쓰지 않고 `skipped`로 남긴다. `done`과 다르다:
 * `done`은 catalog 캔버스가 그 방에 들어갔다는 뜻이고, `skipped`는 들어가지
 * **않았다**는 뜻이다 — `StepStatus::is_settled()`가 재시도에서 이 둘을
 * 같이 "끝남"으로 묶어 취급하는 이유이기도 하다. `#[serde(rename_all =
 * "lowercase")]`가 실제로 내는 네 철자와 맞춘다.
 */
export type CatalogStepStatus = "pending" | "done" | "failed" | "skipped";

export type CatalogStepStates = {
  channel: CatalogStepStatus;
  canvas: CatalogStepStatus;
  membership: CatalogStepStatus;
};

export type CatalogLedgerItem = {
  item_key: string;
  /** preflight와 같은 값이다 — {@link CatalogItemName} 참고. */
  name: CatalogItemName;
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
