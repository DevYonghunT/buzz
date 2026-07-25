import type { ManagedAgent } from "@/shared/api/types";

/** Inline normalization — avoids runtime dependency on @/shared/lib/pubkey. */
function normalizePubkey(pubkey: string): string {
  return pubkey.trim().toLowerCase();
}

function commandBasename(command: string) {
  const normalized = command.trim().replace(/\\/g, "/");
  const parts = normalized.split("/");
  return parts[parts.length - 1] ?? normalized;
}

function normalizeCommandIdentity(command: string) {
  const lower = commandBasename(command).toLowerCase();
  if (lower === "claude-code-acp" || lower === "claude-agent-acp") {
    return "claude-acp";
  }
  return lower;
}

export function commandsMatch(left: string, right: string) {
  return normalizeCommandIdentity(left) === normalizeCommandIdentity(right);
}

export function parseTimestamp(value: string | null | undefined) {
  if (!value) {
    return 0;
  }

  const timestamp = Date.parse(value);
  return Number.isNaN(timestamp) ? 0 : timestamp;
}

export function pickPreferredManagedAgent(agents: ManagedAgent[]) {
  return [...agents].sort((left, right) => {
    const leftRunningScore =
      left.status === "running" || left.status === "deployed" ? 1 : 0;
    const rightRunningScore =
      right.status === "running" || right.status === "deployed" ? 1 : 0;
    if (leftRunningScore !== rightRunningScore) {
      return rightRunningScore - leftRunningScore;
    }

    return parseTimestamp(right.updatedAt) - parseTimestamp(left.updatedAt);
  })[0];
}

export function findReusablePersonaAgent(
  agents: ManagedAgent[],
  personaId: string,
  channelMemberPubkeys: ReadonlySet<string>,
  teamId: string | null = null,
): ManagedAgent | undefined {
  const candidates = agents.filter(
    (agent) =>
      agent.personaId === personaId &&
      (agent.teamId ?? null) === teamId &&
      !channelMemberPubkeys.has(normalizePubkey(agent.pubkey)),
  );
  return pickPreferredManagedAgent(candidates);
}

export type AttachedPersonaAgentSpec = {
  personaId: string;
  teamId?: string | null;
  runtimeCommand: string;
  runtimeArgs: readonly string[];
  runtimeMcpCommand?: string | null;
  backend?: ManagedAgent["backend"];
  model?: string;
  systemPrompt?: string;
  respondTo?: ManagedAgent["respondTo"];
  respondToAllowlist?: readonly string[];
  parallelism?: number | null;
};

function normalizeOptionalText(
  value: string | null | undefined,
): string | null {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
}

function backendsMatch(
  agentBackend: ManagedAgent["backend"],
  expectedBackend: ManagedAgent["backend"] | undefined,
): boolean {
  const expected = expectedBackend ?? { type: "local" };
  if (agentBackend.type !== expected.type) {
    return false;
  }
  return (
    agentBackend.type === "local" ||
    (expected.type === "provider" && agentBackend.id === expected.id)
  );
}

function allowlistsMatch(
  left: readonly string[],
  right: readonly string[],
): boolean {
  const normalize = (values: readonly string[]) =>
    [...new Set(values.map(normalizePubkey))].sort();
  const normalizedLeft = normalize(left);
  const normalizedRight = normalize(right);
  return (
    normalizedLeft.length === normalizedRight.length &&
    normalizedLeft.every((value, index) => value === normalizedRight[index])
  );
}

function orderedStringsMatch(
  left: readonly string[],
  right: readonly string[],
): boolean {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

/** Return whether an attached persona instance exactly satisfies a deployment. */
export function personaAgentMatchesDeployment(
  agent: ManagedAgent,
  spec: AttachedPersonaAgentSpec,
): boolean {
  const expectedRespondTo = spec.respondTo ?? "owner-only";
  return (
    agent.personaId === spec.personaId &&
    (agent.teamId ?? null) === (spec.teamId ?? null) &&
    !agent.personaOutOfDate &&
    !agent.personaOrphaned &&
    commandsMatch(agent.agentCommand, spec.runtimeCommand) &&
    orderedStringsMatch(agent.agentArgs, spec.runtimeArgs) &&
    normalizeOptionalText(agent.mcpCommand) ===
      normalizeOptionalText(spec.runtimeMcpCommand) &&
    backendsMatch(agent.backend, spec.backend) &&
    normalizeOptionalText(agent.model) === normalizeOptionalText(spec.model) &&
    normalizeOptionalText(agent.systemPrompt) ===
      normalizeOptionalText(spec.systemPrompt) &&
    agent.respondTo === expectedRespondTo &&
    (expectedRespondTo !== "allowlist" ||
      allowlistsMatch(
        agent.respondToAllowlist,
        spec.respondToAllowlist ?? [],
      )) &&
    (spec.parallelism == null || agent.parallelism === spec.parallelism)
  );
}

/** Find an exact deployment of a persona already attached to the channel. */
export function findAttachedPersonaAgent(
  agents: ManagedAgent[],
  spec: AttachedPersonaAgentSpec,
  channelMemberPubkeys: ReadonlySet<string>,
): ManagedAgent | undefined {
  return pickPreferredManagedAgent(
    agents.filter(
      (agent) =>
        personaAgentMatchesDeployment(agent, spec) &&
        channelMemberPubkeys.has(normalizePubkey(agent.pubkey)),
    ),
  );
}

/** Find an exact persona deployment that is not attached to the channel. */
export function findReusablePersonaDeploymentAgent(
  agents: ManagedAgent[],
  spec: AttachedPersonaAgentSpec,
  channelMemberPubkeys: ReadonlySet<string>,
): ManagedAgent | undefined {
  return pickPreferredManagedAgent(
    agents.filter(
      (agent) =>
        personaAgentMatchesDeployment(agent, spec) &&
        !channelMemberPubkeys.has(normalizePubkey(agent.pubkey)),
    ),
  );
}

export function findReusableGenericAgent(
  agents: ManagedAgent[],
  command: string,
  channelMemberPubkeys: ReadonlySet<string>,
): ManagedAgent | undefined {
  const candidates = agents.filter(
    (agent) =>
      !agent.personaId &&
      !agent.systemPrompt?.trim() &&
      commandsMatch(agent.agentCommand, command) &&
      !channelMemberPubkeys.has(normalizePubkey(agent.pubkey)),
  );
  return pickPreferredManagedAgent(candidates);
}

/**
 * Check if a reusable agent exists for the given input. Used by the UI to
 * surface the "reuse vs create new" guardrail before submission.
 */
export function findReusableAgent(
  agents: ManagedAgent[],
  channelMemberPubkeys: ReadonlySet<string>,
  input: {
    personaId?: string | null;
    systemPrompt?: string;
    command: string;
  },
): ManagedAgent | undefined {
  if (input.personaId) {
    return findReusablePersonaAgent(
      agents,
      input.personaId,
      channelMemberPubkeys,
      null,
    );
  }
  if (!input.systemPrompt?.trim()) {
    return findReusableGenericAgent(
      agents,
      input.command,
      channelMemberPubkeys,
    );
  }
  return undefined;
}
