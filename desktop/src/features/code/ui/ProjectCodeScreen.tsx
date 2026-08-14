import { useQuery } from "@tanstack/react-query";
import {
  ArrowLeft,
  ChevronRight,
  Code2,
  FolderGit2,
  LoaderCircle,
} from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useCommunities } from "@/features/communities/useCommunities";
import {
  useProjectLocalRepoSnapshotQuery,
  useProjectQuery,
} from "@/features/projects/hooks";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { Button } from "@/shared/ui/button";
import { codeRepositoryQueryOptions } from "../state/codeSessionQueries";
import { CodeWorkspaceScreen } from "./CodeWorkspaceScreen";

function CodeBootstrapState({
  action,
  description,
  loading = false,
  title,
}: {
  action?: React.ReactNode;
  description: string;
  loading?: boolean;
  title: string;
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center px-6 py-12 text-center">
      {loading ? (
        <LoaderCircle className="h-8 w-8 animate-spin text-muted-foreground motion-reduce:animate-none" />
      ) : (
        <Code2 className="h-8 w-8 text-muted-foreground/40" />
      )}
      <h2 className="mt-3 text-sm font-semibold">{title}</h2>
      <p className="mt-1 max-w-md text-xs text-muted-foreground">
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
  const localRepositoryQuery = useProjectLocalRepoSnapshotQuery(
    project,
    activeCommunity?.reposDir,
    baseRef,
  );
  const repositoryRoot = localRepositoryQuery.data?.path ?? "";
  const repositoryInput = React.useMemo(
    () => ({ repositoryRoot, baseRef }),
    [baseRef, repositoryRoot],
  );
  const repositoryQuery = useQuery({
    ...codeRepositoryQueryOptions(repositoryInput),
    enabled: repositoryRoot.length > 0,
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

  if (!project || projectQuery.isError) {
    return (
      <CodeBootstrapState
        action={
          <Button onClick={() => void goProjects()} size="sm" variant="outline">
            <ArrowLeft />
            Back to Projects
          </Button>
        }
        description={
          projectQuery.isError
            ? "The project could not be loaded."
            : "This project is no longer available in the active community."
        }
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
          <Button onClick={backToProject} size="sm" variant="outline">
            Open project
          </Button>
        }
        description="Clone this project from its project page before starting a Code task."
        title="Local checkout required"
      />
    );
  } else if (repositoryQuery.isPending) {
    content = (
      <CodeBootstrapState
        description="Validating the selected Git repository and base branch."
        loading
        title="Preparing project scope"
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
              <FolderGit2 className="h-3.5 w-3.5" />
              Projects
            </button>
            <ChevronRight className="h-3 w-3 shrink-0 opacity-60" />
            <button
              className="min-w-0 truncate rounded-md px-1 py-1 font-medium transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              onClick={backToProject}
              type="button"
            >
              {project.name}
            </button>
            <ChevronRight className="h-3 w-3 shrink-0 opacity-60" />
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
