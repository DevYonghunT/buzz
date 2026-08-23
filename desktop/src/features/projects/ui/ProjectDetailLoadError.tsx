import { ArrowLeft, FolderGit2 } from "lucide-react";

import { Button } from "@/shared/ui/button";

export function ProjectDetailLoadError({
  isRetrying,
  onBack,
  onRetry,
}: {
  isRetrying: boolean;
  onBack: () => unknown;
  onRetry: () => unknown;
}) {
  return (
    <div
      className="flex flex-1 flex-col items-center justify-center gap-3 px-4 py-16 text-center"
      role="alert"
    >
      <FolderGit2 className="h-10 w-10 text-muted-foreground/40" />
      <p className="text-sm text-destructive">Failed to load project</p>
      <div className="flex items-center gap-2">
        <Button
          disabled={isRetrying}
          onClick={() => void onRetry()}
          size="sm"
          variant="outline"
        >
          {isRetrying ? "Retrying…" : "Retry"}
        </Button>
        <Button onClick={() => void onBack()} size="sm" variant="ghost">
          <ArrowLeft className="mr-1.5 h-4 w-4" />
          Back to Projects
        </Button>
      </div>
    </div>
  );
}
