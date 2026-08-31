import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { channelsQueryKey } from "@/features/channels/hooks";
import { getChannels, joinChannel } from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";

export type ProjectChannelAccessErrorCode =
  | "invite-required"
  | "channel-unavailable";

const PROJECT_CHANNEL_ACCESS_ERROR_MESSAGES: Record<
  ProjectChannelAccessErrorCode,
  string
> = {
  "invite-required":
    "This project is private. Ask a project channel owner or admin to invite this account before cloning.",
  "channel-unavailable":
    "This project's channel is unavailable. Ask the project owner to check the channel before cloning.",
};

/** A deterministic project-channel refusal that must stop remote Git access. */
export class ProjectChannelAccessError extends Error {
  readonly code: ProjectChannelAccessErrorCode;

  constructor(code: ProjectChannelAccessErrorCode) {
    super(PROJECT_CHANNEL_ACCESS_ERROR_MESSAGES[code]);
    this.name = "ProjectChannelAccessError";
    this.code = code;
  }
}

export type ProjectChannelAccessDecision =
  | { kind: "already-member"; channel: Channel }
  | { kind: "join"; channel: Channel | null }
  | { kind: "invite-required"; channel: Channel };

/**
 * Decide whether a project channel already grants Git access, can be joined,
 * or requires an invitation. A missing discovery row remains joinable here:
 * the relay is authoritative, and attempting kind:9021 safely distinguishes a
 * stale/missing open-channel snapshot from a private or deleted channel.
 */
export function projectChannelAccessDecision(
  channels: readonly Channel[],
  channelId: string,
): ProjectChannelAccessDecision {
  const channel = channels.find((candidate) => candidate.id === channelId);
  if (!channel) {
    return { kind: "join", channel: null };
  }
  if (channel.isMember) {
    return { kind: "already-member", channel };
  }
  if (channel.visibility === "private") {
    return { kind: "invite-required", channel };
  }
  return { kind: "join", channel };
}

export type ProjectChannelAccessDependencies = {
  getChannels: () => Promise<Channel[]>;
  joinChannel: (channelId: string) => Promise<void>;
};

export type ProjectChannelAccessResult = {
  channels: Channel[] | null;
  status: "already-member" | "joined" | "unbound";
};

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
  ) {
    return error.message;
  }
  return "";
}

/** Translate stable relay join refusals without parsing them in UI code. */
export function projectChannelJoinErrorCode(
  error: unknown,
): Extract<
  ProjectChannelAccessErrorCode,
  "invite-required" | "channel-unavailable"
> | null {
  const message = errorMessage(error).toLowerCase();
  if (message.includes("channel is private")) {
    return "invite-required";
  }
  if (message.includes("channel not found")) {
    return "channel-unavailable";
  }
  return null;
}

const defaultDependencies: ProjectChannelAccessDependencies = {
  getChannels,
  joinChannel,
};

/**
 * Ensure the current identity can read a project's relay-backed Git remote.
 *
 * Existing members are a no-op. Open non-member channels are joined and then
 * read back before the caller may start Git. The read-back refreshes cached
 * metadata only: membership projection can lag indefinitely, so an accepted
 * join proceeds to Git, whose smart-HTTP ACL remains authoritative. Private
 * and missing channels still fail when the relay rejects the join. Projects
 * without a channel binding keep the legacy path so older announcements can
 * reach the relay's existing no-binding remediation.
 */
export async function ensureProjectChannelAccess(
  channelId: string | null | undefined,
  dependencies: ProjectChannelAccessDependencies = defaultDependencies,
): Promise<ProjectChannelAccessResult> {
  if (!channelId) {
    return { channels: null, status: "unbound" };
  }

  const initialChannels = await dependencies.getChannels();
  const decision = projectChannelAccessDecision(initialChannels, channelId);
  if (decision.kind === "already-member") {
    return { channels: initialChannels, status: "already-member" };
  }
  if (decision.kind === "invite-required") {
    throw new ProjectChannelAccessError("invite-required");
  }

  try {
    await dependencies.joinChannel(channelId);
  } catch (error) {
    const code = projectChannelJoinErrorCode(error);
    if (code) {
      throw new ProjectChannelAccessError(code);
    }
    throw error;
  }

  let refreshedChannels: Channel[] | null = null;
  try {
    refreshedChannels = await dependencies.getChannels();
  } catch {
    // Cache reconciliation is best-effort. The clone/fetch request immediately
    // after this call is the authoritative read-access check.
  }

  return { channels: refreshedChannels, status: "joined" };
}

/**
 * React adapter that keeps the shared channel cache in sync with the latest
 * relay read and schedules the normal channel query to reconcile metadata.
 */
export function useEnsureProjectChannelAccess() {
  const queryClient = useQueryClient();

  return React.useCallback(
    async (channelId: string | null | undefined) => {
      const result = await ensureProjectChannelAccess(channelId);
      if (result.channels) {
        queryClient.setQueryData<Channel[]>(channelsQueryKey, result.channels);
      }
      if (result.status === "joined") {
        void queryClient.invalidateQueries({ queryKey: channelsQueryKey });
      }
      return result;
    },
    [queryClient],
  );
}
