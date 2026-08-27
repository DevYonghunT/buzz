import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const ProjectCodeScreen = React.lazy(async () => {
  const module = await import("@/features/code/ui/ProjectCodeScreen");
  return { default: module.ProjectCodeScreen };
});

export const Route = createFileRoute("/projects/$projectId/code")({
  component: ProjectCodeRouteComponent,
  validateSearch: (search: Record<string, unknown>) => ({
    baseRef: typeof search.baseRef === "string" ? search.baseRef : undefined,
    threadId: typeof search.threadId === "string" ? search.threadId : undefined,
  }),
});

function ProjectCodeRouteComponent() {
  const { projectId } = Route.useParams();
  const { baseRef, threadId } = Route.useSearch();
  const navigate = Route.useNavigate();

  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="projects" />}>
      <ProjectCodeScreen
        baseRef={baseRef}
        onThreadIdChange={(nextThreadId, replace = false) => {
          void navigate({
            replace,
            search: (current) => ({
              ...current,
              threadId: nextThreadId ?? undefined,
            }),
          });
        }}
        projectId={projectId}
        threadId={threadId}
      />
    </React.Suspense>
  );
}
