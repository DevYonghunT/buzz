import {
  commandsMatch,
  findAttachedPersonaAgent,
  findReusableGenericAgent,
  findReusablePersonaAgent,
  findReusablePersonaDeploymentAgent,
  pickPreferredManagedAgent,
} from "@/features/agents/agentReuse";
export { findReusableAgent } from "@/features/agents/agentReuse";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { resolveManagedAgentAvatarUrl } from "@/features/agents/ui/managedAgentAvatar";
import {
  addChannelMembers,
  createManagedAgent,
  getChannelMembers,
  listManagedAgents,
  updateManagedAgent,
} from "@/shared/api/tauri";
import { startManagedAgent } from "@/shared/api/tauriManagedAgents";
import type {
  AcpRuntime,
  ChannelRole,
  ManagedAgent,
  ManagedAgentBackend,
  RespondToMode,
} from "@/shared/api/types";

type ChannelAgentRuntime = Pick<
  AcpRuntime,
  "id" | "label" | "command" | "defaultArgs" | "mcpCommand"
>;

export type AttachManagedAgentToChannelInput = {
  agent: ManagedAgent;
  role?: Exclude<ChannelRole, "owner">;
  ensureRunning?: boolean;
};

export type AttachManagedAgentToChannelResult = {
  agent: ManagedAgent;
  membershipAdded: boolean;
  started: boolean;
};

export type EnsureChannelAgentPresetInput = {
  runtime: ChannelAgentRuntime;
  role?: Exclude<ChannelRole, "owner">;
  ensureRunning?: boolean;
};

export type EnsureChannelAgentPresetResult =
  AttachManagedAgentToChannelResult & {
    created: boolean;
    runtimeId: string;
  };

export type CreateChannelManagedAgentInput = {
  runtime: ChannelAgentRuntime;
  name: string;
  systemPrompt?: string;
  avatarUrl?: string;
  personaId?: string | null;
  /** Team this instance is deployed from; prevents cross-team reuse. */
  teamId?: string | null;
  /**
   * True when `runtime` is a runtime the user deliberately picked to override
   * the persona (a deploy-dialog runtime selector), as opposed to a
   * missing-runtime fallback. Forwarded to the backend so a persona-backed
   * create only pins the harness for a deliberate override.
   */
  harnessOverride?: boolean;
  /** Preferred model ID from the persona. Passed to createManagedAgent. */
  model?: string;
  role?: Exclude<ChannelRole, "owner">;
  ensureRunning?: boolean;
  backend?: ManagedAgentBackend;
  /** Inbound author gate mode. Omitted = server default ("owner-only"). */
  respondTo?: RespondToMode;
  /** Hex pubkeys for allowlist mode. */
  respondToAllowlist?: string[];
  /** Persona-defined concurrency to preserve during template deployment. */
  parallelism?: number;
  /** Skip reuse logic and always create a fresh agent instance. */
  forceNewInstance?: boolean;
  /**
   * Treat an exact, current deployment of this persona already attached to the
   * channel as success. Durable cross-process idempotency still belongs to the
   * workspace-catalog manifest rather than this local batch guard.
   */
  ensurePersonaMembership?: boolean;
};

export type CreateChannelManagedAgentResult =
  AttachManagedAgentToChannelResult & {
    created: boolean;
    runtimeId: string;
  };

export type ProvisionChannelManagedAgentResult = {
  agent: ManagedAgent;
  created: boolean;
  runtimeId: string;
};

export type CreateChannelManagedAgentBatchFailure = {
  kind: "generic" | "persona";
  name: string;
  personaId: string | null;
  error: string;
};

export type CreateChannelManagedAgentsResult = {
  successes: CreateChannelManagedAgentResult[];
  failures: CreateChannelManagedAgentBatchFailure[];
};

export async function attachManagedAgentToChannel(
  channelId: string,
  input: AttachManagedAgentToChannelInput,
) {
  const role = input.role ?? "bot";
  const ensureRunning = input.ensureRunning ?? true;
  const agentPubkey = normalizePubkey(input.agent.pubkey);
  const membershipResult = await addChannelMembers({
    channelId,
    pubkeys: [input.agent.pubkey],
    role,
  });
  const membershipError = membershipResult.errors.find(
    (error) => normalizePubkey(error.pubkey) === agentPubkey,
  );
  if (membershipError) {
    throw new Error(membershipError.error);
  }
  const membershipAdded = membershipResult.added.some(
    (pubkey) => normalizePubkey(pubkey) === agentPubkey,
  );

  let agent = input.agent;
  let started = false;

  if (ensureRunning) {
    // Running agents (local or provider) auto-discover new channel membership
    // via the harness's membership notifications — no restart needed. Only
    // not-yet-running agents need a start/deploy call before the first mention
    // can reach them. For a local agent the status check and the start are both
    // pair-scoped to the active community: `agent.status` reflects that
    // community's (agent, relay) pair, and `startManagedAgent` spawns that same
    // pair — so this ensures the pair the caller is attaching to, never
    // another community's.
    const isRemote = input.agent.backend.type === "provider";
    if (isRemote && input.agent.status !== "deployed") {
      agent = await startManagedAgent(input.agent.pubkey);
      started = true;
    } else if (
      !isRemote &&
      input.agent.status !== "running" &&
      input.agent.status !== "deployed"
    ) {
      agent = await startManagedAgent(input.agent.pubkey);
      started = true;
    }
  }

  return {
    agent,
    membershipAdded,
    started,
  } satisfies AttachManagedAgentToChannelResult;
}

function buildChannelAgentName(runtimeId: string, runtimeLabel: string) {
  const normalizedRuntimeId = runtimeId.trim().toLowerCase();
  if (normalizedRuntimeId.length > 0) {
    return normalizedRuntimeId;
  }

  return runtimeLabel.trim().toLowerCase() || "agent";
}

function pickPreferredChannelPresetAgent(
  agents: ManagedAgent[],
  memberPubkeys: ReadonlySet<string>,
  runtimeCommand: string,
  expectedName: string,
) {
  const inChannelAgent = pickPreferredManagedAgent(
    agents.filter(
      (agent) =>
        commandsMatch(agent.agentCommand, runtimeCommand) &&
        memberPubkeys.has(normalizePubkey(agent.pubkey)),
    ),
  );
  if (inChannelAgent) {
    return inChannelAgent;
  }

  return pickPreferredManagedAgent(
    agents.filter(
      (agent) =>
        commandsMatch(agent.agentCommand, runtimeCommand) &&
        agent.name.trim().toLowerCase() === expectedName.trim().toLowerCase(),
    ),
  );
}

export async function ensureChannelAgentPresetInChannel(
  channelId: string,
  input: EnsureChannelAgentPresetInput,
): Promise<EnsureChannelAgentPresetResult> {
  const role = input.role ?? "bot";
  const ensureRunning = input.ensureRunning ?? true;
  const members = await getChannelMembers(channelId);
  const memberPubkeys = new Set(
    members.map((member) => normalizePubkey(member.pubkey)),
  );
  const managedAgents = await listManagedAgents();
  const expectedName = buildChannelAgentName(
    input.runtime.id,
    input.runtime.label,
  );
  const existingAgent = pickPreferredChannelPresetAgent(
    managedAgents,
    memberPubkeys,
    input.runtime.command,
    expectedName,
  );

  if (existingAgent) {
    const attached = await attachManagedAgentToChannel(channelId, {
      agent: existingAgent,
      role,
      ensureRunning,
    });
    return {
      ...attached,
      created: false,
      runtimeId: input.runtime.id,
    };
  }

  const created = await createManagedAgent({
    name: expectedName,
    acpCommand: "buzz-acp",
    agentCommand: input.runtime.command,
    agentArgs: input.runtime.defaultArgs,
    mcpCommand: input.runtime.mcpCommand ?? "",
    spawnAfterCreate: false,
  });
  const attached = await attachManagedAgentToChannel(channelId, {
    agent: created.agent,
    role,
    ensureRunning,
  });

  return {
    ...attached,
    created: true,
    runtimeId: input.runtime.id,
  };
}

export async function provisionChannelManagedAgent(
  input: CreateChannelManagedAgentInput,
  context?: {
    managedAgents?: ManagedAgent[];
    channelMemberPubkeys?: ReadonlySet<string>;
  },
): Promise<ProvisionChannelManagedAgentResult> {
  const trimmedName = input.name.trim();

  if (trimmedName.length === 0) {
    throw new Error("Agent name is required.");
  }

  // Smart reuse: if a managed agent with the same personaId already exists
  // and is not already in this channel, attach it instead of creating a new one.
  if (
    input.personaId &&
    !input.forceNewInstance &&
    context?.managedAgents &&
    context.channelMemberPubkeys
  ) {
    const reusable = input.ensurePersonaMembership
      ? findReusablePersonaDeploymentAgent(
          context.managedAgents,
          {
            personaId: input.personaId,
            teamId: input.teamId,
            runtimeCommand: input.runtime.command,
            runtimeArgs: input.runtime.defaultArgs,
            runtimeMcpCommand: input.runtime.mcpCommand,
            backend: input.backend,
            model: input.model,
            systemPrompt: input.systemPrompt,
            respondTo: input.respondTo,
            respondToAllowlist: input.respondToAllowlist,
            parallelism: input.parallelism,
          },
          context.channelMemberPubkeys,
        )
      : findReusablePersonaAgent(
          context.managedAgents,
          input.personaId,
          context.channelMemberPubkeys,
          input.teamId ?? null,
        );
    if (reusable) {
      // Apply the caller's respondTo settings so the user's permission
      // choice in the dialog is always honored, even when reusing.
      const needsRespondToUpdate = input.respondTo !== undefined;
      const updatedAgent = needsRespondToUpdate
        ? (
            await updateManagedAgent({
              pubkey: reusable.pubkey,
              respondTo: input.respondTo,
              respondToAllowlist:
                input.respondTo === "allowlist"
                  ? input.respondToAllowlist
                  : undefined,
            })
          ).agent
        : reusable;

      return {
        agent: updatedAgent,
        created: false,
        runtimeId: input.runtime.id,
      };
    }
  }

  // Generic agent reuse: if no persona is set and the system prompt is blank,
  // look for an existing agent with the same command and no custom prompt.
  if (
    !input.personaId &&
    !input.systemPrompt?.trim() &&
    !input.forceNewInstance &&
    context?.managedAgents &&
    context.channelMemberPubkeys
  ) {
    const reusable = findReusableGenericAgent(
      context.managedAgents,
      input.runtime.command,
      context.channelMemberPubkeys,
    );
    if (reusable) {
      const needsRespondToUpdate = input.respondTo !== undefined;
      const updatedAgent = needsRespondToUpdate
        ? (
            await updateManagedAgent({
              pubkey: reusable.pubkey,
              respondTo: input.respondTo,
              respondToAllowlist:
                input.respondTo === "allowlist"
                  ? input.respondToAllowlist
                  : undefined,
            })
          ).agent
        : reusable;

      return {
        agent: updatedAgent,
        created: false,
        runtimeId: input.runtime.id,
      };
    }
  }

  // Resolve the avatar for the channel-managed agent. Base64 data URIs (e.g.
  // from a persona PNG card import) are uploaded to a hosted URL the relay can
  // serve; percent-encoded emoji SVG data URLs pass through unchanged so the
  // selected emoji survives deployment. Shared with agent creation so both
  // paths handle emoji avatars identically.
  const resolvedAvatarUrl = await resolveManagedAgentAvatarUrl(input.avatarUrl);

  const isProviderMode = input.backend?.type === "provider";

  const created = await createManagedAgent({
    name: trimmedName,
    acpCommand: "buzz-acp",
    agentCommand: input.runtime.command,
    harnessOverride: input.harnessOverride ?? false,
    agentArgs: input.runtime.defaultArgs,
    mcpCommand: input.runtime.mcpCommand ?? "",
    personaId: input.personaId ?? undefined,
    teamId: input.teamId ?? undefined,
    systemPrompt: input.systemPrompt?.trim() || undefined,
    avatarUrl: resolvedAvatarUrl,
    model: input.model?.trim() || undefined,
    spawnAfterCreate: isProviderMode,
    startOnAppLaunch: isProviderMode ? false : undefined,
    backend: input.backend,
    respondTo: input.respondTo,
    respondToAllowlist: input.respondToAllowlist,
    parallelism: input.parallelism,
  });

  // Tauri returns Ok() even on deploy failure — spawnError carries the message.
  if (created.spawnError) {
    throw new Error(created.spawnError);
  }

  return {
    agent: created.agent,
    created: true,
    runtimeId: input.runtime.id,
  };
}

export async function createChannelManagedAgent(
  channelId: string,
  input: CreateChannelManagedAgentInput,
  context?: {
    managedAgents?: ManagedAgent[];
    channelMemberPubkeys?: ReadonlySet<string>;
  },
): Promise<CreateChannelManagedAgentResult> {
  const provisioned = await provisionChannelManagedAgent(input, context);
  const attached = await attachManagedAgentToChannel(channelId, {
    agent: provisioned.agent,
    role: input.role ?? "bot",
    ensureRunning: input.ensureRunning ?? true,
  });

  return {
    ...attached,
    created: provisioned.created,
    runtimeId: provisioned.runtimeId,
  };
}

export async function createChannelManagedAgents(
  channelId: string,
  inputs: readonly CreateChannelManagedAgentInput[],
): Promise<CreateChannelManagedAgentsResult> {
  // Fetch managed agents and channel members once for smart reuse checks.
  const [managedAgents, members] = await Promise.all([
    listManagedAgents(),
    getChannelMembers(channelId),
  ]);
  const channelMemberPubkeys = new Set(
    members.map((m) => normalizePubkey(m.pubkey)),
  );
  const context = { managedAgents, channelMemberPubkeys };

  // Sequential loop: each agent must be fully created and its relay membership
  // written before the next starts. Concurrent writes to the replaceable
  // kind:39002 membership event cause last-write-wins data loss.
  const successes: CreateChannelManagedAgentResult[] = [];
  const failures: CreateChannelManagedAgentBatchFailure[] = [];

  for (let i = 0; i < inputs.length; i++) {
    const input = inputs[i];
    try {
      if (
        input.ensurePersonaMembership &&
        input.personaId &&
        !input.forceNewInstance
      ) {
        const attached = findAttachedPersonaAgent(
          managedAgents,
          {
            personaId: input.personaId,
            teamId: input.teamId,
            runtimeCommand: input.runtime.command,
            runtimeArgs: input.runtime.defaultArgs,
            runtimeMcpCommand: input.runtime.mcpCommand,
            backend: input.backend,
            model: input.model,
            systemPrompt: input.systemPrompt,
            respondTo: input.respondTo,
            respondToAllowlist: input.respondToAllowlist,
            parallelism: input.parallelism,
          },
          channelMemberPubkeys,
        );
        if (attached) {
          let agent = attached;
          let started = false;
          const ensureRunning = input.ensureRunning ?? true;
          const isRunning =
            agent.status === "running" || agent.status === "deployed";
          // Retry a previous failed start, but preserve an intentional manual
          // stop. Without durable template provenance we cannot safely infer
          // that every stopped, matching instance is template-owned.
          if (ensureRunning && !isRunning && agent.lastError) {
            agent = await startManagedAgent(agent.pubkey);
            started = true;
          }
          successes.push({
            agent,
            membershipAdded: false,
            started,
            created: false,
            runtimeId: input.runtime.id,
          });
          continue;
        }

        const conflictingAttached = managedAgents.some(
          (agent) =>
            agent.personaId === input.personaId &&
            (agent.teamId ?? null) === (input.teamId ?? null) &&
            channelMemberPubkeys.has(normalizePubkey(agent.pubkey)),
        );
        if (conflictingAttached) {
          throw new Error(
            "A different deployment of this persona is already attached to the channel.",
          );
        }
      }
      const result = await createChannelManagedAgent(channelId, input, context);
      successes.push(result);
      const resultPubkey = normalizePubkey(result.agent.pubkey);
      const existingIndex = managedAgents.findIndex(
        (agent) => normalizePubkey(agent.pubkey) === resultPubkey,
      );
      if (existingIndex >= 0) {
        managedAgents[existingIndex] = result.agent;
      } else {
        managedAgents.push(result.agent);
      }
      channelMemberPubkeys.add(resultPubkey);
    } catch (error) {
      failures.push({
        kind: input.personaId ? "persona" : "generic",
        name: input.name.trim() || "agent",
        personaId: input.personaId ?? null,
        error: error instanceof Error ? error.message : "Failed to add agent.",
      });
    }
  }

  return { successes, failures };
}
