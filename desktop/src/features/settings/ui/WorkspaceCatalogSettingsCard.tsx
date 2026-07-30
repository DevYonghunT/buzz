import { AlertCircle, RefreshCw } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

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

export function WorkspaceCatalogSettingsCard() {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [ledger, setLedger] = useState<CatalogLedger | null>(null);

  const preflight = useWorkspaceCatalogPreflightQuery();
  const apply = useApplyWorkspaceCatalogMutation();

  const items = preflight.data ?? [];

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
    apply.mutate([...selected], {
      onSuccess: (result) => {
        setLedger(result);
        setSelected(new Set());
      },
    });
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
              onToggle={toggle}
              selected={selected.has(item.item_key)}
            />
          ))}
        </div>
      )}

      <Button
        className="mt-6"
        data-testid="catalog-apply"
        disabled={selected.size === 0 || apply.isPending}
        onClick={handleApply}
        type="button"
      >
        {apply.isPending ? t("catalog.applying") : t("catalog.apply")}
      </Button>

      {apply.isError ? (
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
  onToggle,
  selected,
}: {
  item: CatalogPreflightItem;
  ledgerItem: CatalogLedgerItem | undefined;
  locked: boolean;
  onToggle: (item: CatalogPreflightItem) => void;
  selected: boolean;
}) {
  const { t } = useTranslation();
  const checkboxId = `workspace-catalog-item-${item.item_key}`;

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
              {item.item_key}
            </label>
            <Badge variant={DECISION_BADGE_VARIANT[item.decision]}>
              {t(`catalog.decision.${item.decision}`)}
            </Badge>
          </div>
          {item.renamed ? (
            <p className="text-2xs text-muted-foreground">
              {t("catalog.renamed")}
            </p>
          ) : null}
          {isOpenVisibility(item) ? (
            <Alert data-testid={`catalog-open-warning-${item.item_key}`}>
              <AlertDescription className="space-y-1">
                <p>{t("catalog.openWarningScope")}</p>
                <p>{t("catalog.openWarningAgents")}</p>
              </AlertDescription>
            </Alert>
          ) : null}
        </div>
      </div>

      {ledgerItem ? (
        <div className="mt-3 space-y-2 border-border/50 border-t pt-3">
          <Badge variant={OUTCOME_BADGE_VARIANT[ledgerItem.outcome]}>
            {t(`catalog.outcome.${ledgerItem.outcome}`)}
          </Badge>
          {ledgerItem.user_action ? (
            <Alert
              className="border-amber-500/30 bg-amber-500/10"
              data-testid={`catalog-user-action-${item.item_key}`}
            >
              <AlertDescription className="text-amber-800 dark:text-amber-300">
                {t(`catalog.userAction.${ledgerItem.user_action}`)}
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
