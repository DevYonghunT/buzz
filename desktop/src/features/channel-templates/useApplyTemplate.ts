import { useQueryClient } from "@tanstack/react-query";

import {
  createChannelManagedAgents,
  type CreateChannelManagedAgentInput,
} from "@/features/agents/channelAgents";
import {
  useAvailableAcpRuntimes,
  usePersonasQuery,
  useTeamsQuery,
} from "@/features/agents/hooks";
import { resolvePersonaRuntime } from "@/features/agents/lib/resolvePersonaRuntime";
import { resolveTeamPersonas } from "@/features/agents/lib/teamPersonas";
import { useLastRuntime } from "@/features/agents/lib/useLastRuntime";
import { useChannelTemplatesQuery } from "@/features/channel-templates/hooks";
import { setCanvas } from "@/shared/api/tauri";
import type { AcpRuntime, ChannelTemplate } from "@/shared/api/types";

/**
 * TemplateBackend omits `config` — supply an empty object for provider backends.
 */
function toManagedBackend(
  backend: ChannelTemplate["agents"]["personas"][number]["backend"],
): CreateChannelManagedAgentInput["backend"] {
  if (!backend || backend.type === "local") return { type: "local" };
  return { type: "provider", id: backend.id, config: {} };
}

function resolveTemplateModel(
  templateModel: string | null,
  personaModel: string | null,
): { model: string | undefined; conflict: boolean } {
  const requested = templateModel?.trim() || null;
  const inherited = personaModel?.trim() || null;
  return {
    // Linked persona models are authoritative in the current mint contract.
    // Templates may fill a blank model, but must not silently override one.
    model: inherited ?? requested ?? undefined,
    conflict: Boolean(requested && inherited && requested !== inherited),
  };
}

async function warnTemplateApply(message: string) {
  const { toast } = await import("sonner");
  toast.warning(message);
}

export function useApplyTemplate() {
  const queryClient = useQueryClient();
  const channelTemplatesQuery = useChannelTemplatesQuery();
  const acpRuntimesQuery = useAvailableAcpRuntimes();
  const personasQuery = usePersonasQuery();
  const teamsQuery = useTeamsQuery();
  const { lastRuntimeId } = useLastRuntime();

  async function applyCanvas(
    templateId: string | undefined,
    channelId: string,
    channelName: string,
  ) {
    if (!templateId) return;
    const templatesResult = await channelTemplatesQuery.refetch();
    if (templatesResult.isError || !templatesResult.data) {
      await warnTemplateApply(
        "The channel was created, but its template could not be loaded to apply the canvas.",
      );
      return;
    }
    const template = templatesResult.data.find((t) => t.id === templateId);
    if (!template) {
      await warnTemplateApply(
        "The channel was created, but the selected template is no longer available.",
      );
      return;
    }
    if (!template.canvasTemplate) return;
    const content = template.canvasTemplate
      .replace(/\{channel\.name\}/g, channelName)
      .replace(/\{template\.name\}/g, template.name);
    try {
      await setCanvas({ channelId, content });
    } catch (error) {
      await warnTemplateApply(
        error instanceof Error
          ? `The channel was created, but its canvas could not be applied: ${error.message}`
          : "The channel was created, but its canvas could not be applied.",
      );
    }
  }

  async function applyAgents(
    templateId: string | undefined,
    channelId: string,
  ) {
    if (!templateId) return;
    const templatesResult = await channelTemplatesQuery.refetch();
    if (templatesResult.isError || !templatesResult.data) {
      await warnTemplateApply(
        "The channel was created, but its template could not be loaded to apply agents.",
      );
      return;
    }
    const template = templatesResult.data.find((t) => t.id === templateId);
    if (!template) {
      await warnTemplateApply(
        "The channel was created, but the selected template is no longer available.",
      );
      return;
    }
    const { personas: templatePersonas, teams: templateTeams } =
      template.agents;
    if (templatePersonas.length === 0 && templateTeams.length === 0) return;

    const [personasResult, teamsResult, runtimesResult] = await Promise.all([
      personasQuery.refetch(),
      teamsQuery.refetch(),
      acpRuntimesQuery.refetch(),
    ]);
    const failedCatalogs = [
      personasResult.isError ? "personas" : null,
      teamsResult.isError ? "teams" : null,
      runtimesResult.isError ? "ACP runtimes" : null,
    ].filter((value): value is string => value !== null);
    if (
      failedCatalogs.length > 0 ||
      !personasResult.data ||
      !teamsResult.data ||
      !runtimesResult.data
    ) {
      await warnTemplateApply(
        `The channel was created, but template agents could not be loaded${
          failedCatalogs.length > 0 ? ` (${failedCatalogs.join(", ")})` : ""
        }.`,
      );
      return;
    }

    const allPersonas = personasResult.data;
    const allTeams = teamsResult.data;
    const runtimes = runtimesResult.data.filter(
      (runtime): runtime is AcpRuntime => runtime.availability === "available",
    );
    if (runtimes.length === 0) {
      await warnTemplateApply(
        "The channel was created, but template agents need an available ACP runtime.",
      );
      return;
    }

    // Resolve default provider: user's last-used preference, or first available
    const defaultProvider =
      runtimes.find((p) => p.id === lastRuntimeId) ?? runtimes[0] ?? null;
    if (!defaultProvider) return;

    const seenDeploymentKeys = new Set<string>();
    const inputs: CreateChannelManagedAgentInput[] = [];
    const unresolved: string[] = [];

    // Direct personas from template
    for (const entry of templatePersonas) {
      const persona = allPersonas.find((p) => p.id === entry.personaId);
      if (!persona) {
        unresolved.push(`persona ${entry.personaId}`);
        continue;
      }
      const deploymentKey = `direct:${persona.id}`;
      if (seenDeploymentKeys.has(deploymentKey)) continue;
      seenDeploymentKeys.add(deploymentKey);
      const resolved = resolvePersonaRuntime(
        entry.runtime ?? persona.runtime,
        runtimes,
        defaultProvider,
      );
      if (!resolved.runtime || resolved.isOverridden) {
        unresolved.push(
          `runtime ${entry.runtime ?? persona.runtime ?? "(default)"} for persona ${persona.id}`,
        );
        continue;
      }
      const resolvedModel = resolveTemplateModel(entry.model, persona.model);
      if (resolvedModel.conflict) {
        unresolved.push(
          `model ${entry.model} conflicts with persona ${persona.id}`,
        );
        continue;
      }
      inputs.push({
        runtime: resolved.runtime,
        name: persona.displayName,
        personaId: persona.id,
        harnessOverride:
          persona.runtime == null ||
          (entry.runtime != null && entry.runtime !== persona.runtime),
        systemPrompt: persona.systemPrompt,
        avatarUrl: persona.avatarUrl ?? undefined,
        model: resolvedModel.model,
        role: "bot",
        backend: toManagedBackend(entry.backend),
        respondTo: persona.respondTo ?? undefined,
        respondToAllowlist: persona.respondToAllowlist,
        parallelism: persona.parallelism ?? undefined,
        ensurePersonaMembership: true,
      });
    }

    // Team-expanded personas (skip dupes)
    for (const teamEntry of templateTeams) {
      const team = allTeams.find((t) => t.id === teamEntry.teamId);
      if (!team) {
        unresolved.push(`team ${teamEntry.teamId}`);
        continue;
      }
      const { missingPersonaIds, resolvedPersonas } = resolveTeamPersonas(
        team,
        allPersonas,
      );
      unresolved.push(
        ...missingPersonaIds.map(
          (personaId) => `persona ${personaId} in team ${team.id}`,
        ),
      );
      for (const persona of resolvedPersonas) {
        const deploymentKey = `team:${team.id}:${persona.id}`;
        if (seenDeploymentKeys.has(deploymentKey)) continue;
        seenDeploymentKeys.add(deploymentKey);
        const resolved = resolvePersonaRuntime(
          teamEntry.runtime ?? persona.runtime,
          runtimes,
          defaultProvider,
        );
        if (!resolved.runtime || resolved.isOverridden) {
          unresolved.push(
            `runtime ${teamEntry.runtime ?? persona.runtime ?? "(default)"} for persona ${persona.id} in team ${team.id}`,
          );
          continue;
        }
        const resolvedModel = resolveTemplateModel(
          teamEntry.model,
          persona.model,
        );
        if (resolvedModel.conflict) {
          unresolved.push(
            `model ${teamEntry.model} conflicts with persona ${persona.id} in team ${team.id}`,
          );
          continue;
        }
        inputs.push({
          runtime: resolved.runtime,
          name: persona.displayName,
          personaId: persona.id,
          teamId: team.id,
          harnessOverride:
            persona.runtime == null ||
            (teamEntry.runtime != null &&
              teamEntry.runtime !== persona.runtime),
          systemPrompt: persona.systemPrompt,
          avatarUrl: persona.avatarUrl ?? undefined,
          model: resolvedModel.model,
          role: "bot",
          backend: toManagedBackend(teamEntry.backend),
          respondTo: persona.respondTo ?? undefined,
          respondToAllowlist: persona.respondToAllowlist,
          parallelism: persona.parallelism ?? undefined,
          ensurePersonaMembership: true,
        });
      }
    }

    if (unresolved.length > 0) {
      await warnTemplateApply(
        `Some template entries were not found and were skipped: ${unresolved.join(", ")}`,
      );
    }
    if (inputs.length === 0) return;

    try {
      const result = await createChannelManagedAgents(channelId, inputs);
      if (result.failures.length > 0) {
        const details = result.failures
          .slice(0, 3)
          .map((failure) => `${failure.name}: ${failure.error}`)
          .join("; ");
        await warnTemplateApply(
          `${result.failures.length} template agent${
            result.failures.length === 1 ? "" : "s"
          } could not be applied${details ? ` — ${details}` : ""}`,
        );
      }
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: ["channels", channelId, "members"],
        }),
        queryClient.invalidateQueries({ queryKey: ["managed-agents"] }),
        queryClient.invalidateQueries({ queryKey: ["relay-agents"] }),
      ]);
    } catch (error) {
      await warnTemplateApply(
        error instanceof Error
          ? `The channel was created, but its template agents could not be applied: ${error.message}`
          : "The channel was created, but its template agents could not be applied.",
      );
    }
  }

  return { applyCanvas, applyAgents };
}
