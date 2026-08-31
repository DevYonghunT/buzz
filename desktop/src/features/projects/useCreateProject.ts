import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  channelsQueryKey,
  upsertCachedChannel,
} from "@/features/channels/hooks";
import {
  eventToProject,
  fetchProjects,
  type Project,
  projectsQueryKey,
} from "@/features/projects/hooks";
import { resolveProjectRelayBase } from "@/features/projects/projectRelayBase";
import {
  initializeProjectRepository,
  type ProjectRepoInitializeResult,
} from "@/shared/api/projectGit";
import { relayClient } from "@/shared/api/relayClient";
import { createChannel, signRelayEvent } from "@/shared/api/tauri";
import type { Channel, ChannelVisibility } from "@/shared/api/types";
import { KIND_REPO_ANNOUNCEMENT } from "@/shared/constants/kinds";

export type CreateProjectInput = {
  name: string;
  repositoryId: string;
  visibility: ChannelVisibility;
  description?: string;
  webUrl?: string;
};

export type CreateProjectResult = {
  channel: Channel;
  project: Project;
  repositoryInitialization: ProjectRepoInitializeResult | null;
  repositoryInitializationError: string | null;
};

const PROJECT_REPOSITORY_ID_PATTERN = /^[A-Za-z0-9._-]{1,64}$/;

/** Returns the first validation error for a relay-backed repository ID. */
export function projectRepositoryIdError(value: string): string | null {
  const repositoryId = value.trim();
  if (!repositoryId) {
    return "Repository ID is required.";
  }
  if (!PROJECT_REPOSITORY_ID_PATTERN.test(repositoryId)) {
    return "Repository ID must be 1–64 characters using only letters, numbers, dots, underscores, and hyphens.";
  }
  if (repositoryId.startsWith(".")) {
    return "Repository ID must not start with a dot.";
  }
  if (repositoryId.includes("..")) {
    return "Repository ID must not contain consecutive dots.";
  }
  return null;
}

/** Publishes a NIP-34 repo announcement so the project appears on the relay. */
async function createProject(
  input: CreateProjectInput,
  reposDir?: string | null,
): Promise<CreateProjectResult> {
  const name = input.name.trim();
  if (!name) {
    throw new Error("Project name is required.");
  }
  const repositoryId = input.repositoryId.trim();
  const repositoryIdError = projectRepositoryIdError(repositoryId);
  if (repositoryIdError) {
    throw new Error(repositoryIdError);
  }

  const existing = await fetchProjects();
  if (existing.some((project) => project.dtag === repositoryId)) {
    throw new Error(`Repository ID "${repositoryId}" is already in use.`);
  }

  const description = input.description?.trim() ?? "";
  const channel = await createChannel({
    channelType: "stream",
    description: description || undefined,
    name,
    visibility: input.visibility,
  });

  const tags: string[][] = [
    ["d", repositoryId],
    ["name", name],
    ["buzz-channel", channel.id],
  ];
  if (description) {
    tags.push(["description", description]);
  }
  const webUrl = input.webUrl?.trim();
  if (webUrl) {
    tags.push(["web", webUrl]);
  }

  const event = await signRelayEvent({
    kind: KIND_REPO_ANNOUNCEMENT,
    content: description,
    tags,
  });

  await relayClient.publishEvent(
    event,
    "Timed out creating project.",
    "Failed to create project.",
  );
  const relayBase = await resolveProjectRelayBase();
  const project = eventToProject(event, relayBase);

  let repositoryInitialization: ProjectRepoInitializeResult | null = null;
  let repositoryInitializationError: string | null = null;
  const cloneUrl = project.cloneUrls[0];
  if (cloneUrl) {
    try {
      repositoryInitialization = await initializeProjectRepository({
        reposDir,
        projectDtag: project.dtag,
        cloneUrl,
        defaultBranch: project.defaultBranch,
      });
    } catch (error) {
      // The channel and repository announcement already exist at this point.
      // Preserve that successful creation and let Code offer a safe retry.
      repositoryInitializationError =
        error instanceof Error
          ? error.message
          : "Failed to initialize the project repository.";
    }
  } else {
    repositoryInitializationError =
      "The project was created without a repository clone URL.";
  }

  return {
    channel,
    project,
    repositoryInitialization,
    repositoryInitializationError,
  };
}

/** Mutation that creates a project with its stream and updates both caches. */
export function useCreateProjectMutation(reposDir?: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: CreateProjectInput) => createProject(input, reposDir),
    onSuccess: ({ channel, project }) => {
      queryClient.setQueryData<Channel[]>(channelsQueryKey, (current) =>
        upsertCachedChannel(current, channel),
      );
      queryClient.setQueryData<Project[]>(projectsQueryKey, (current = []) => [
        project,
        ...current.filter((candidate) => candidate.id !== project.id),
      ]);
      void queryClient.invalidateQueries({ queryKey: projectsQueryKey });
      void queryClient.invalidateQueries({
        queryKey: ["projects", "local-repositories"],
      });
    },
    onSettled: () => {
      void queryClient.invalidateQueries({
        queryKey: channelsQueryKey,
        refetchType: "none",
      });
    },
  });
}
