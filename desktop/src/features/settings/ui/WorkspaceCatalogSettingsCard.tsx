import { AlertCircle, RefreshCw } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import {
  isCatalogAdminRequiredError,
  isCatalogMembershipUnavailableError,
} from "@/features/workspace-catalog/catalogError";
import {
  useApplyWorkspaceCatalogMutation,
  useWorkspaceCatalogPreflightQuery,
} from "@/features/workspace-catalog/hooks";
import type {
  CatalogDecision,
  CatalogLedger,
  CatalogLedgerItem,
  CatalogOutcome,
  CatalogPreflightItem,
  CatalogStepStatus,
  CatalogUserAction,
} from "@/shared/api/tauriWorkspaceCatalog";
import { cn } from "@/shared/lib/cn";
import { Alert, AlertDescription } from "@/shared/ui/alert";
import { Badge, type BadgeProps } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import { Skeleton } from "@/shared/ui/skeleton";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

/**
 * Decisions the user cannot toggle: the item is already fully applied
 * (`no_change`) or SchoolX no longer offers it (`retired`). Selecting either
 * would leave "apply selected" with nothing to do for that item.
 *
 * `conflict` is deliberately NOT locked — the saga safely reports it back as
 * `blocked` with `resolve_conflict` (see `apply_item` in saga.rs) instead of
 * touching anything, so letting the user select and submit it is harmless
 * and is how they discover the blocker.
 */
const LOCKED_DECISIONS = new Set<CatalogDecision>(["no_change", "retired"]);

/**
 * The two `user_action`s that "create it again" can answer.
 *
 * `resolve_conflict` is deliberately absent: that item is blocked by a
 * same-named room the catalog did not create, and moving to the next
 * generation would leave that room in place while adding a second one — the
 * conflict is what the administrator has to resolve, not route around.
 *
 * The two included cases are not equal. `confirm_recreate` has exactly one
 * sensible answer, so the button is the primary control. `request_ownership`
 * cannot tell a squatted derived id apart from an ordinary co-administrator's
 * room (`CATALOG_RECREATE.md` §4), so there the button is secondary and the
 * consequence is spelled out first.
 */
const RECREATABLE_ACTIONS = new Set<CatalogUserAction>([
  "confirm_recreate",
  "request_ownership",
]);

type BadgeVariant = NonNullable<BadgeProps["variant"]>;

const DECISION_BADGE_VARIANT: Record<CatalogDecision, BadgeVariant> = {
  create_or_recreate: "info",
  resume: "info",
  no_change: "secondary",
  conflict: "warning",
  retired: "secondary",
  deleted: "warning",
  adopted: "info",
  not_owned: "warning",
};

const OUTCOME_BADGE_VARIANT: Record<CatalogOutcome, BadgeVariant> = {
  applied: "success",
  unchanged: "secondary",
  partial: "warning",
  blocked: "warning",
};

/**
 * `CatalogPreflightItem` carries no `visibility` — `Visibility` lives only on
 * the Rust-side `CatalogItem` (`crates/schoolx-catalog/src/catalog.rs`) and is
 * not threaded through `preflight_workspace_catalog`. Every built-in item
 * ships `private` (`every_builtin_item_is_private` in catalog.rs pins this),
 * so this always evaluates to `false` today — there is no reachable path to
 * the warning it guards from the current catalog.
 *
 * Replace the body with `item.visibility === "open"` once a future catalog
 * item ships `open` visibility and that field is threaded through
 * `PreflightItem` -> `preflight_workspace_catalog` -> `CatalogPreflightItem`.
 * Do not ship that item without also wiring this check — an open room reads
 * as private-looking without it.
 */
function isOpenVisibility(_item: CatalogPreflightItem): boolean {
  return false;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * The two refusals `require_community_admin` (`commands/workspace_catalog.rs`)
 * can return, and how each reads.
 *
 * They are kept apart because they ask the user for different things:
 * `adminRequired` is actionable (go ask an administrator), while
 * `membershipUnavailable` means the relay publishes no community roles at all,
 * so there is no administrator to ask — that one is for whoever runs the
 * relay. Collapsing them would send a community owner on a dead-end errand.
 */
const GATE_REFUSALS = [
  {
    matches: isCatalogAdminRequiredError,
    copyKey: "catalog.adminRequired",
    testId: "catalog-admin-required",
  },
  {
    matches: isCatalogMembershipUnavailableError,
    copyKey: "catalog.membershipUnavailable",
    testId: "catalog-membership-unavailable",
  },
] as const;

type GateRefusal = (typeof GATE_REFUSALS)[number];

function gateRefusal(error: unknown): GateRefusal | null {
  return GATE_REFUSALS.find((refusal) => refusal.matches(error)) ?? null;
}

/**
 * The two {@link CatalogStepStatus} values worth a note next to the canvas
 * step, translated to a key under `catalog.canvasStep.*`.
 *
 * `"skipped"` and `"unrecognized"` both mean "the starter canvas was not
 * written by this run" but for different reasons the administrator needs to
 * read differently:
 * - `"skipped"`: the saga looked and found existing content, so it
 *   deliberately left the room alone. See `StepStatus::Skipped` in
 *   `provenance.rs` — reporting this the same as `"done"` would hide the
 *   one fact that matters here, that the team's content survived.
 * - `"unrecognized"`: a newer build of the app wrote a value this build
 *   does not know (`StepStatus::Unrecognized`, the `#[serde(other)]`
 *   catch-all). This build genuinely does not know what happened, so the
 *   copy must not claim either "kept" or "written" — only that a newer
 *   version recorded something here.
 *
 * `"pending"`, `"done"`, and `"failed"` get no separate note here: `"done"`
 * is the unremarkable default already implied by the outcome/decision
 * badges, and `"pending"`/`"failed"` are covered by the outcome badge and
 * `ledgerItem.error`.
 */
function canvasStepNoteKey(
  status: CatalogStepStatus | undefined,
): `catalog.canvasStep.${"skipped" | "unrecognized"}` | null {
  if (status === "skipped" || status === "unrecognized") {
    return `catalog.canvasStep.${status}`;
  }
  return null;
}

export function WorkspaceCatalogSettingsCard() {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [ledger, setLedger] = useState<CatalogLedger | null>(null);

  const preflight = useWorkspaceCatalogPreflightQuery();
  const apply = useApplyWorkspaceCatalogMutation();

  const items = preflight.data ?? [];
  // Preflight is the first call the panel makes, so this is where a refused
  // caller normally lands. `apply` is checked too because the two calls are
  // separate round-trips: a role can be revoked between them, and
  // `SettingsView`'s section filter is advisory, so it does not close that
  // window. The enforced gate is `require_community_admin` on the commands
  // themselves (`commands/workspace_catalog.rs`); this only decides how the
  // refusal reads.
  const refusal = gateRefusal(preflight.error) ?? gateRefusal(apply.error);

  function toggle(item: CatalogPreflightItem) {
    if (LOCKED_DECISIONS.has(item.decision)) return;
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(item.item_key)) next.delete(item.item_key);
      else next.add(item.item_key);
      return next;
    });
  }

  function handleApply() {
    apply.mutate(
      [...selected].map((item_key) => ({ item_key, recreate_from: null })),
      {
        onSuccess: (result) => {
          setLedger(result);
          setSelected(new Set());
        },
      },
    );
  }

  /**
   * "Create it again" for an item the last run could not touch.
   *
   * Sends back the generation the user was **shown**, not that plus one. The
   * backend only moves when its own preflight still reports that same
   * generation (`Selection::recreate_from` in `saga.rs`), which is what makes
   * a stale screen — or a second click, or another administrator who already
   * handled it — a no-op instead of an extra room. Incrementing here would
   * defeat that check.
   *
   * The selection checkboxes are left alone: this acts on one item and is not
   * the bulk "apply selected" path.
   */
  function handleRecreate(entry: CatalogLedgerItem) {
    apply.mutate(
      [{ item_key: entry.item_key, recreate_from: entry.generation }],
      {
        onSuccess: (result) => setLedger(result),
      },
    );
  }

  return (
    <section className="min-w-0" data-testid="settings-workspace-catalog">
      <SettingsSectionHeader
        description={t("catalog.description")}
        title={t("catalog.title")}
      />

      {preflight.isLoading ? (
        <div
          className="space-y-3"
          data-testid="settings-workspace-catalog-loading"
        >
          <Skeleton className="h-20 w-full rounded-xl" />
          <Skeleton className="h-20 w-full rounded-xl" />
        </div>
      ) : refusal ? (
        /*
          Not an error the user can act on by retrying — the answer will not
          change until someone grants them the role, or reconfigures the relay
          — so this gets the amber "you need to do something else" treatment
          used by `user_action` below, not the destructive styling with a Try
          again button.
        */
        <Alert
          className="border-amber-500/30 bg-amber-500/10"
          data-testid={refusal.testId}
        >
          <AlertDescription className="text-amber-800 dark:text-amber-300">
            {t(refusal.copyKey)}
          </AlertDescription>
        </Alert>
      ) : preflight.isError ? (
        <Alert className="space-y-2" variant="destructive">
          <div className="flex items-center gap-2">
            <AlertCircle className="h-4 w-4 shrink-0" />
            <AlertDescription>{errorMessage(preflight.error)}</AlertDescription>
          </div>
          <button
            className="flex items-center gap-1.5 font-medium text-xs hover:underline"
            onClick={() => void preflight.refetch()}
            type="button"
          >
            <RefreshCw className="h-3.5 w-3.5" />
            {t("settings.sidebar.tryAgain")}
          </button>
        </Alert>
      ) : (
        <div className="space-y-3">
          {items.map((item) => (
            <CatalogItemRow
              item={item}
              key={item.item_key}
              ledgerItem={ledger?.items.find(
                (entry) => entry.item_key === item.item_key,
              )}
              locked={LOCKED_DECISIONS.has(item.decision)}
              onRecreate={handleRecreate}
              onToggle={toggle}
              pending={apply.isPending}
              selected={selected.has(item.item_key)}
            />
          ))}
        </div>
      )}

      {/*
        The apply button is hidden rather than disabled when the backend has
        refused: a disabled button reads as "not yet", which is the wrong
        story — nothing the user does on this screen will enable it.
      */}
      {refusal ? null : (
        <Button
          className="mt-6"
          data-testid="catalog-apply"
          disabled={selected.size === 0 || apply.isPending}
          onClick={handleApply}
          type="button"
        >
          {apply.isPending ? t("catalog.applying") : t("catalog.apply")}
        </Button>
      )}

      {apply.isError && !refusal ? (
        <Alert className="mt-3" variant="destructive">
          <AlertDescription>{errorMessage(apply.error)}</AlertDescription>
        </Alert>
      ) : null}
    </section>
  );
}

function CatalogItemRow({
  item,
  ledgerItem,
  locked,
  onRecreate,
  onToggle,
  pending,
  selected,
}: {
  item: CatalogPreflightItem;
  ledgerItem: CatalogLedgerItem | undefined;
  locked: boolean;
  onRecreate: (entry: CatalogLedgerItem) => void;
  onToggle: (item: CatalogPreflightItem) => void;
  /** An apply (or recreate) is in flight — every row's button waits it out. */
  pending: boolean;
  selected: boolean;
}) {
  const { t } = useTranslation();
  const checkboxId = `workspace-catalog-item-${item.item_key}`;
  const canvasNoteKey = canvasStepNoteKey(ledgerItem?.steps.canvas);

  return (
    <div
      className="rounded-xl border border-border/70 p-4"
      data-testid={`catalog-item-${item.item_key}`}
    >
      <div className="flex items-start gap-3">
        <Checkbox
          checked={selected}
          className="mt-0.5"
          disabled={locked}
          id={checkboxId}
          onCheckedChange={() => onToggle(item)}
        />
        <div className="min-w-0 flex-1 space-y-1.5">
          <div className="flex flex-wrap items-center gap-2">
            <label
              className={cn("font-medium text-sm", !locked && "cursor-pointer")}
              htmlFor={checkboxId}
            >
              {/*
                catalog 표시 이름이다. `item_key`("meeting")를 그대로 보여주면
                catalog가 "메인 회의방"이라고 정한 방이 화면에서는 영문 슬러그로
                보인다. 이름은 Rust catalog에서 온다 — TS에서 키를 이름으로
                바꾸는 표를 따로 두지 않는다.

                `retired` 항목은 이름이 `null`이다 (catalog에서 빠져 이름이
                남아 있는 곳이 없다). 키로 조용히 메우지 않고 "모른다"를
                그대로 말한다 — 아래에 키를 따로 보여주므로 식별은 된다.
              */}
              {item.name ?? t("catalog.unnamedItem")}
            </label>
            <Badge variant={DECISION_BADGE_VARIANT[item.decision]}>
              {t(`catalog.decision.${item.decision}`)}
            </Badge>
          </div>
          {item.name === null ? (
            <p
              className="text-2xs text-muted-foreground"
              data-testid={`catalog-item-key-${item.item_key}`}
            >
              {t("catalog.itemKeyLabel", { key: item.item_key })}
            </p>
          ) : null}
          {item.renamed ? (
            <p
              className="text-2xs text-muted-foreground"
              data-testid={`catalog-renamed-${item.item_key}`}
            >
              {t("catalog.renamed")}
            </p>
          ) : null}
          {isOpenVisibility(item) ? (
            <Alert
              className="border-amber-500/30 bg-amber-500/10"
              data-testid={`catalog-open-warning-${item.item_key}`}
            >
              <AlertDescription className="space-y-1 text-amber-800 dark:text-amber-300">
                <p>{t("catalog.openWarningScope")}</p>
                <p>{t("catalog.openWarningAgents")}</p>
              </AlertDescription>
            </Alert>
          ) : null}
        </div>
      </div>

      {ledgerItem ? (
        <div className="mt-3 space-y-2 border-border/50 border-t pt-3">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant={OUTCOME_BADGE_VARIANT[ledgerItem.outcome]}>
              {t(`catalog.outcome.${ledgerItem.outcome}`)}
            </Badge>
            {/*
              `outcome` alone conflates two cases that both end in "applied":
              a room this run created vs. one it adopted from a prior partial
              run (`ADOPTED_DECISION` in ledger.rs). The decision badge is the
              same vocabulary already shown pre-apply above, so an
              administrator reads it the same way in both places.
            */}
            <Badge variant={DECISION_BADGE_VARIANT[ledgerItem.decision]}>
              {t(`catalog.decision.${ledgerItem.decision}`)}
            </Badge>
          </div>
          {canvasNoteKey ? (
            <p
              className="text-xs text-muted-foreground"
              data-testid={`catalog-canvas-note-${item.item_key}`}
            >
              {t(canvasNoteKey)}
            </p>
          ) : null}
          {ledgerItem.user_action ? (
            <Alert
              className="border-amber-500/30 bg-amber-500/10"
              data-testid={`catalog-user-action-${item.item_key}`}
            >
              <AlertDescription className="space-y-2 text-amber-800 dark:text-amber-300">
                <p>{t(`catalog.userAction.${ledgerItem.user_action}`)}</p>
                {/*
                  `request_ownership` keeps "ask the person who made it" as its
                  first line; the button below is the secondary way out, for
                  the case where that person does not exist because the derived
                  id was squatted. Stating the consequence before the control
                  is the whole safeguard here — the verdict itself cannot tell
                  the two situations apart (`CATALOG_RECREATE.md` §4).
                */}
                {ledgerItem.user_action === "request_ownership" ? (
                  <p className="text-xs">
                    {t("catalog.recreate.ownedByOther")}
                  </p>
                ) : null}
                {RECREATABLE_ACTIONS.has(ledgerItem.user_action) ? (
                  <Button
                    data-testid={`catalog-recreate-${item.item_key}`}
                    disabled={pending}
                    onClick={() => onRecreate(ledgerItem)}
                    size="sm"
                    type="button"
                    variant={
                      ledgerItem.user_action === "request_ownership"
                        ? "ghost"
                        : "default"
                    }
                  >
                    {pending
                      ? t("catalog.recreate.pending")
                      : t("catalog.recreate.action")}
                  </Button>
                ) : null}
              </AlertDescription>
            </Alert>
          ) : null}
          {ledgerItem.error ? (
            <Alert
              data-testid={`catalog-error-${item.item_key}`}
              variant="destructive"
            >
              <AlertDescription>{ledgerItem.error}</AlertDescription>
            </Alert>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
