import { useQuery } from "@tanstack/react-query";
import {
  ArrowLeft,
  ChevronRight,
  Code2,
  FolderGit2,
  LoaderCircle,
  SquareTerminal,
} from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useCommunities } from "@/features/communities/useCommunities";
import {
  useProjectLocalRepoSnapshotQuery,
  useProjectQuery,
} from "@/features/projects/hooks";
import {
  projectTerminalLabel,
  useOpenProjectTerminal,
} from "@/features/projects/ui/useOpenProjectTerminal";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { Button } from "@/shared/ui/button";
import { codeRepositoryQueryOptions } from "../state/codeSessionQueries";
import { CodeWorkspaceScreen } from "./CodeWorkspaceScreen";

const CODE_BASE_REF_RESOLUTION_ERROR =
  "failed to resolve SchoolX Code base ref";
const CODE_DEVELOPER_ID_APP_REQUIRED_ERROR =
  "SchoolX Code Git requires a Developer ID signed SchoolX application";

function isFirstCommitRequiredError(error: unknown): boolean {
  const message =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : typeof error === "object" &&
            error !== null &&
            "message" in error &&
            typeof error.message === "string"
          ? error.message
          : "";
  return message.includes(CODE_BASE_REF_RESOLUTION_ERROR);
}

function isDeveloperIdAppRequiredError(error: unknown): boolean {
  const message =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : typeof error === "object" &&
            error !== null &&
            "message" in error &&
            typeof error.message === "string"
          ? error.message
          : "";
  return message.includes(CODE_DEVELOPER_ID_APP_REQUIRED_ERROR);
}

function CodeBootstrapState({
  action,
  announcementRole,
  description,
  loading = false,
  title,
}: {
  action?: React.ReactNode;
  announcementRole?: "alert" | "status";
  description: string;
  loading?: boolean;
  title: string;
}) {
  return (
    <div
      aria-busy={loading || undefined}
      className="flex min-h-0 flex-1 flex-col items-center justify-center px-6 py-12 text-center"
      role={announcementRole ?? (loading ? "status" : undefined)}
    >
      {loading ? (
        <LoaderCircle className="size-8 animate-spin text-muted-foreground motion-reduce:animate-none" />
      ) : (
        <Code2 className="size-8 text-muted-foreground/40" />
      )}
      <h2 className="mt-3 text-balance text-sm font-semibold">{title}</h2>
      <p className="mt-1 max-w-md text-pretty text-xs text-muted-foreground">
        {description}
      </p>
      {action ? <div className="mt-4">{action}</div> : null}
    </div>
  );
}

export function ProjectCodeScreen({
  baseRef: requestedBaseRef,
  onThreadIdChange,
  projectId,
  threadId,
}: {
  baseRef?: string;
  onThreadIdChange: (threadId: string | null, replace?: boolean) => void;
  projectId: string;
  threadId?: string;
}) {
  const { activeCommunity } = useCommunities();
  const { goProject, goProjects } = useAppNavigation();
  const projectQuery = useProjectQuery(projectId);
  const project = projectQuery.data;
  const baseRef =
    requestedBaseRef?.trim() || project?.defaultBranch?.trim() || "HEAD";
  const terminalBranch =
    baseRef === "HEAD" ? project?.defaultBranch?.trim() || null : baseRef;
  const localRepositoryQuery = useProjectLocalRepoSnapshotQuery(
    project,
    activeCommunity?.reposDir,
    baseRef,
  );
  const openProjectTerminal = useOpenProjectTerminal(activeCommunity?.reposDir);
  const [isOpeningTerminal, setIsOpeningTerminal] = React.useState(false);
  const handleOpenProjectTerminal = React.useCallback(
    async (hasLocalCheckout: boolean) => {
      if (!project || isOpeningTerminal) return;

      setIsOpeningTerminal(true);
      try {
        await openProjectTerminal(project, {
          branch: terminalBranch,
          hasLocalCheckout,
        });
      } finally {
        setIsOpeningTerminal(false);
      }
    },
    [isOpeningTerminal, openProjectTerminal, project, terminalBranch],
  );
  const repositoryRoot = localRepositoryQuery.data?.path ?? "";
  const isEmptyLocalRepository =
    repositoryRoot.length > 0 &&
    localRepositoryQuery.data?.snapshot.latestCommit === null;
  const repositoryInput = React.useMemo(
    () => ({ repositoryRoot, baseRef }),
    [baseRef, repositoryRoot],
  );
  const repositoryQuery = useQuery({
    ...codeRepositoryQueryOptions(repositoryInput),
    enabled: repositoryRoot.length > 0 && !isEmptyLocalRepository,
    staleTime: 30_000,
  });

  if (projectQuery.isPending) {
    return (
      <CodeBootstrapState
        description="Loading the selected project context."
        loading
        title="Opening SchoolX Code"
      />
    );
  }

  if (projectQuery.isError) {
    return (
      <CodeBootstrapState
        action={
          <div className="flex flex-wrap items-center justify-center gap-2">
            <Button
              disabled={projectQuery.isFetching}
              onClick={() => void projectQuery.refetch()}
              size="sm"
            >
              {projectQuery.isFetching ? (
                <LoaderCircle className="animate-spin motion-reduce:animate-none" />
              ) : null}
              {projectQuery.isFetching ? "Retrying…" : "Retry"}
            </Button>
            <Button
              onClick={() => void goProjects()}
              size="sm"
              variant="outline"
            >
              <ArrowLeft />
              Back to Projects
            </Button>
          </div>
        }
        announcementRole="alert"
        description="A relay request failed. This project may still be available; retry to check again."
        title="Project load failed"
      />
    );
  }

  if (!project) {
    return (
      <CodeBootstrapState
        action={
          <Button onClick={() => void goProjects()} size="sm" variant="outline">
            <ArrowLeft />
            Back to Projects
          </Button>
        }
        description="This project is no longer available in the active community."
        title="Project unavailable"
      />
    );
  }

  const backToProject = () => {
    void goProject(project.id);
  };

  let content: React.ReactNode;
  if (!activeCommunity) {
    content = (
      <CodeBootstrapState
        description="Choose a community before opening a project Code workspace."
        title="No active community"
      />
    );
  } else if (localRepositoryQuery.isPending) {
    content = (
      <CodeBootstrapState
        description={`Locating the local ${baseRef} checkout.`}
        loading
        title="Finding repository"
      />
    );
  } else if (localRepositoryQuery.isError) {
    content = (
      <CodeBootstrapState
        action={
          <Button
            onClick={() => void localRepositoryQuery.refetch()}
            size="sm"
            variant="outline"
          >
            Retry
          </Button>
        }
        description="SchoolX could not inspect this project's local checkout."
        title="Repository unavailable"
      />
    );
  } else if (!repositoryRoot) {
    content = (
      <CodeBootstrapState
        action={
          <Button
            disabled={isOpeningTerminal}
            onClick={() => void handleOpenProjectTerminal(false)}
            size="sm"
          >
            {isOpeningTerminal ? (
              <LoaderCircle className="animate-spin motion-reduce:animate-none" />
            ) : (
              <SquareTerminal />
            )}
            {isOpeningTerminal
              ? "Opening Terminal…"
              : projectTerminalLabel(false)}
          </Button>
        }
        description="Clone this project and open its checkout in Terminal before starting a Code task."
        title="Local checkout required"
      />
    );
  } else if (repositoryQuery.isPending && !isEmptyLocalRepository) {
    content = (
      <CodeBootstrapState
        description="Validating the selected Git repository and base branch."
        loading
        title="Preparing project scope"
      />
    );
  } else if (
    isEmptyLocalRepository ||
    (repositoryQuery.isError &&
      isFirstCommitRequiredError(repositoryQuery.error))
  ) {
    content = (
      <CodeBootstrapState
        action={
          <div className="flex flex-wrap items-center justify-center gap-2">
            <Button
              disabled={isOpeningTerminal}
              onClick={() => void handleOpenProjectTerminal(true)}
              size="sm"
            >
              {isOpeningTerminal ? (
                <LoaderCircle className="animate-spin motion-reduce:animate-none" />
              ) : (
                <SquareTerminal />
              )}
              {isOpeningTerminal
                ? "Opening Terminal…"
                : projectTerminalLabel(true)}
            </Button>
            <Button
              onClick={() =>
                void (isEmptyLocalRepository
                  ? localRepositoryQuery.refetch()
                  : repositoryQuery.refetch())
              }
              size="sm"
              variant="outline"
            >
              Retry
            </Button>
          </div>
        }
        description="Open this project in Terminal, create its first commit, then retry SchoolX Code."
        title="First commit required"
      />
    );
  } else if (
    repositoryQuery.isError &&
    isDeveloperIdAppRequiredError(repositoryQuery.error)
  ) {
    content = (
      <CodeBootstrapState
        action={
          <Button onClick={backToProject} size="sm" variant="outline">
            <ArrowLeft />
            Back to Project
          </Button>
        }
        announcementRole="alert"
        description="SchoolX Code on macOS requires a signed and notarized SchoolX app installed in Applications. Quit this copy, install the signed app in Applications, then open SchoolX again."
        title="Signed SchoolX app required"
      />
    );
  } else if (repositoryQuery.isError || !repositoryQuery.data) {
    content = (
      <CodeBootstrapState
        action={
          <Button
            onClick={() => void repositoryQuery.refetch()}
            size="sm"
            variant="outline"
          >
            Retry
          </Button>
        }
        description="The local repository or selected base branch could not be validated."
        title="Project scope unavailable"
      />
    );
  } else {
    const scope = {
      communityId: activeCommunity.id,
      projectDtag: project.dtag,
      repositoryIdentity: repositoryQuery.data.repositoryIdentity,
    };
    const scopeKey = JSON.stringify([
      scope.communityId,
      scope.projectDtag,
      scope.repositoryIdentity,
    ]);
    content = (
      <CodeWorkspaceScreen
        baseRef={baseRef}
        key={scopeKey}
        onSelectedThreadIdChange={onThreadIdChange}
        projectName={project.name}
        repository={repositoryQuery.data}
        scope={scope}
        selectedThreadId={threadId ?? null}
      />
    );
  }

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <TopChromeInsetHeader className="border-border/60 border-b" flush>
        <header
          className="flex h-11 min-w-0 items-center justify-between gap-3 px-4"
          data-tauri-drag-region
        >
          <nav
            aria-label="Code project breadcrumb"
            className="flex min-w-0 items-center gap-0.5 text-xs text-muted-foreground"
          >
            <button
              className="flex shrink-0 items-center gap-1.5 rounded-md px-1 py-1 font-medium transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              onClick={() => void goProjects()}
              type="button"
            >
              <FolderGit2 className="size-3.5" />
              Projects
            </button>
            <ChevronRight className="size-3 shrink-0 opacity-60" />
            <button
              className="min-w-0 truncate rounded-md px-1 py-1 font-medium transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              onClick={backToProject}
              type="button"
            >
              {project.name}
            </button>
            <ChevronRight className="size-3 shrink-0 opacity-60" />
            <span aria-current="page" className="shrink-0 px-1 font-medium">
              Code
            </span>
          </nav>
          <Button
            className="shrink-0"
            onClick={backToProject}
            size="sm"
            variant="ghost"
          >
            <ArrowLeft />
            Project
          </Button>
        </header>
      </TopChromeInsetHeader>
      {content}
    </div>
  );
}
