import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ZodType } from "zod";

import { invokeTauri, TauriInvokeError } from "@/shared/api/tauri";

import {
  CodeApprovalResponseInputSchema,
  CodeBoundThreadOpenResultSchema,
  CodeEventBacklogSchema,
  CodePreparedWorktreeSchema,
  CodeRepositoryDescriptorSchema,
  CodeRepositoryInspectInputSchema,
  CodeRuntimeEventsInputSchema,
  CodeRuntimeProbeSchema,
  CodeRuntimeStatusSchema,
  CodeThreadBindingRecoverInputSchema,
  CodeThreadListInputSchema,
  CodeThreadPreparationListInputSchema,
  CodeThreadPreparationSchema,
  CodeThreadResumeInputSchema,
  CodeThreadStartErrorSchema,
  CodeThreadStartInputSchema,
  CodeThreadsPageSchema,
  CodeTurnInterruptInputSchema,
  CodeTurnStartInputSchema,
  CodeTurnSteerInputSchema,
  CodeTurnSummarySchema,
  CodeUnitResponseSchema,
  CodeWorkspaceEventSchema,
  CodeWorkspaceEventScopeSchema,
  CodeWorktreeDescriptorSchema,
  CodeWorktreePrepareInputSchema,
  CodeWorktreeStatusSchema,
} from "./schemas";
import type {
  CodeApprovalResponseInput,
  CodeBoundThreadOpenResult,
  CodeEventBacklog,
  CodePreparedWorktree,
  CodeRepositoryDescriptor,
  CodeRepositoryInspectInput,
  CodeRuntimeEventsInput,
  CodeRuntimeProbe,
  CodeRuntimeStatus,
  CodeThreadBindingRecoverInput,
  CodeThreadBindingScope,
  CodeThreadListInput,
  CodeThreadPreparation,
  CodeThreadPreparationListInput,
  CodeThreadResumeInput,
  CodeThreadStartError,
  CodeThreadStartInput,
  CodeThreadsPage,
  CodeTurnInterruptInput,
  CodeTurnStartInput,
  CodeTurnSteerInput,
  CodeTurnSummary,
  CodeWorkspaceEvent,
  CodeWorktreeDescriptor,
  CodeWorktreePrepareInput,
  CodeWorktreeStatus,
} from "./types";
import { codeScopesEqual } from "./types";

export const CODE_WORKSPACE_EVENT_NAME =
  "schoolx-code-workspace-event" as const;

const MAX_BUFFERED_EVENTS = 512;
const MAX_GENERATION_REPLAY_ATTEMPTS = 4;

/** Frozen command names and top-level Tauri argument names. */
export const CODE_WORKSPACE_COMMAND_CONTRACT = [
  { name: "code_runtime_probe", topLevelArgs: [] },
  { name: "code_runtime_start", topLevelArgs: [] },
  { name: "code_runtime_stop", topLevelArgs: [] },
  { name: "code_runtime_status", topLevelArgs: [] },
  {
    name: "code_runtime_events",
    topLevelArgs: ["afterSequence", "runtimeGeneration", "scope"],
  },
  { name: "code_repository_inspect", topLevelArgs: ["input"] },
  { name: "code_worktree_prepare", topLevelArgs: ["input"] },
  { name: "code_worktree_status", topLevelArgs: ["descriptor"] },
  { name: "code_thread_preparations_list", topLevelArgs: ["input"] },
  { name: "code_threads_list", topLevelArgs: ["input"] },
  { name: "code_thread_start", topLevelArgs: ["input"] },
  { name: "code_thread_binding_recover", topLevelArgs: ["input"] },
  { name: "code_thread_resume", topLevelArgs: ["input"] },
  { name: "code_turn_start", topLevelArgs: ["input"] },
  { name: "code_turn_steer", topLevelArgs: ["input"] },
  { name: "code_turn_interrupt", topLevelArgs: ["input"] },
  { name: "code_approval_respond", topLevelArgs: ["input"] },
] as const;

type CodeWorkspaceTransport = {
  invoke(command: string, args?: Record<string, unknown>): Promise<unknown>;
  listen(
    eventName: string,
    handler: (event: { payload: unknown }) => void,
  ): Promise<UnlistenFn>;
};

/** One replay snapshot plus live events buffered while it was requested. */
export type CodeWorkspaceReplayBatch = {
  subscriptionEpoch: number;
  request: CodeRuntimeEventsInput;
  backlog: CodeEventBacklog;
  bufferedEvents: CodeWorkspaceEvent[];
  bufferTruncated: boolean;
};

/** Callbacks for race-free listener registration followed by native replay. */
export type CodeWorkspaceReplayHandlers = {
  onReplay(batch: CodeWorkspaceReplayBatch): void;
  onEvent(event: CodeWorkspaceEvent, subscriptionEpoch: number): void;
  onError(error: unknown): void;
};

/** Identity and cancellation boundary for one scope-owned event subscription. */
export type CodeWorkspaceReplayOptions = {
  subscriptionEpoch: number;
  signal?: AbortSignal;
  scope?: CodeThreadBindingScope;
};

/** Typed SchoolX Code command and event boundary. */
export type CodeWorkspaceApi = {
  probeCodeRuntime(): Promise<CodeRuntimeProbe>;
  startCodeRuntime(): Promise<CodeRuntimeStatus>;
  stopCodeRuntime(): Promise<CodeRuntimeStatus>;
  getCodeRuntimeStatus(): Promise<CodeRuntimeStatus>;
  getCodeRuntimeEvents(
    input: CodeRuntimeEventsInput,
  ): Promise<CodeEventBacklog>;
  inspectCodeRepository(
    input: CodeRepositoryInspectInput,
  ): Promise<CodeRepositoryDescriptor>;
  prepareCodeWorktree(
    input: CodeWorktreePrepareInput,
  ): Promise<CodePreparedWorktree>;
  getCodeWorktreeStatus(
    descriptor: CodeWorktreeDescriptor,
  ): Promise<CodeWorktreeStatus>;
  listCodeThreadPreparations(
    input: CodeThreadPreparationListInput,
  ): Promise<CodeThreadPreparation[]>;
  listCodeThreads(input: CodeThreadListInput): Promise<CodeThreadsPage>;
  startCodeThread(
    input: CodeThreadStartInput,
  ): Promise<CodeBoundThreadOpenResult>;
  recoverCodeThreadBinding(
    input: CodeThreadBindingRecoverInput,
  ): Promise<CodeBoundThreadOpenResult>;
  resumeCodeThread(
    input: CodeThreadResumeInput,
  ): Promise<CodeBoundThreadOpenResult>;
  startCodeTurn(input: CodeTurnStartInput): Promise<CodeTurnSummary>;
  steerCodeTurn(input: CodeTurnSteerInput): Promise<CodeTurnSummary>;
  interruptCodeTurn(input: CodeTurnInterruptInput): Promise<void>;
  respondToCodeApproval(input: CodeApprovalResponseInput): Promise<void>;
  listenForCodeWorkspaceEvents(
    onEvent: (event: CodeWorkspaceEvent) => void,
    onError: (error: unknown) => void,
    scope?: CodeThreadBindingScope,
  ): Promise<UnlistenFn>;
  listenAndReplayCodeWorkspaceEvents(
    input: CodeRuntimeEventsInput,
    handlers: CodeWorkspaceReplayHandlers,
    options: CodeWorkspaceReplayOptions,
  ): Promise<UnlistenFn>;
};

function replayAbortError(): Error {
  const error = new Error("SchoolX Code event replay was aborted");
  error.name = "AbortError";
  return error;
}

function replayUnstableError(): Error {
  return new Error(
    "SchoolX Code runtime generation changed too often to establish an event replay",
  );
}

async function waitWithAbort<T>(
  promise: Promise<T>,
  signal: AbortSignal | undefined,
): Promise<T> {
  if (signal === undefined) return promise;
  if (signal.aborted) throw replayAbortError();

  return new Promise<T>((resolve, reject) => {
    const onAbort = () => reject(replayAbortError());
    signal.addEventListener("abort", onAbort, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener("abort", onAbort);
        resolve(value);
      },
      (error: unknown) => {
        signal.removeEventListener("abort", onAbort);
        reject(error);
      },
    );
  });
}

function promiseWithFatalDecode<T>(
  promise: Promise<T>,
  fatalDecode: Promise<never>,
): Promise<T> {
  return Promise.race([promise, fatalDecode]);
}

function validateSubscriptionEpoch(subscriptionEpoch: number): number {
  if (!Number.isSafeInteger(subscriptionEpoch) || subscriptionEpoch < 0) {
    throw new TypeError(
      "subscriptionEpoch must be a non-negative safe integer",
    );
  }
  return subscriptionEpoch;
}

async function parseInvocation<T>(
  transport: CodeWorkspaceTransport,
  command: string,
  schema: ZodType<T>,
  args?: Record<string, unknown>,
): Promise<T> {
  return schema.parse(await transport.invoke(command, args));
}

/** Build an adapter around an injectable transport for isolated contract tests. */
export function createCodeWorkspaceApi(
  transport: CodeWorkspaceTransport,
): CodeWorkspaceApi {
  const getCodeRuntimeEvents = (
    input: CodeRuntimeEventsInput,
  ): Promise<CodeEventBacklog> => {
    const parsed = CodeRuntimeEventsInputSchema.parse(input);
    return parseInvocation(
      transport,
      "code_runtime_events",
      CodeEventBacklogSchema,
      parsed,
    );
  };

  const listenForCodeWorkspaceEvents = (
    onEvent: (event: CodeWorkspaceEvent) => void,
    onError: (error: unknown) => void,
    scope?: CodeThreadBindingScope,
  ): Promise<UnlistenFn> =>
    transport.listen(CODE_WORKSPACE_EVENT_NAME, (event) => {
      if (scope !== undefined) {
        const envelope = CodeWorkspaceEventScopeSchema.safeParse(event.payload);
        if (envelope.success && !codeScopesEqual(envelope.data.scope, scope)) {
          return;
        }
      }
      const parsed = CodeWorkspaceEventSchema.safeParse(event.payload);
      if (parsed.success) {
        onEvent(parsed.data);
      } else {
        onError(parsed.error);
      }
    });

  return {
    probeCodeRuntime: () =>
      parseInvocation(transport, "code_runtime_probe", CodeRuntimeProbeSchema),
    startCodeRuntime: () =>
      parseInvocation(transport, "code_runtime_start", CodeRuntimeStatusSchema),
    stopCodeRuntime: () =>
      parseInvocation(transport, "code_runtime_stop", CodeRuntimeStatusSchema),
    getCodeRuntimeStatus: () =>
      parseInvocation(
        transport,
        "code_runtime_status",
        CodeRuntimeStatusSchema,
      ),
    getCodeRuntimeEvents,
    inspectCodeRepository: (input) =>
      parseInvocation(
        transport,
        "code_repository_inspect",
        CodeRepositoryDescriptorSchema,
        { input: CodeRepositoryInspectInputSchema.parse(input) },
      ),
    prepareCodeWorktree: (input) =>
      parseInvocation(
        transport,
        "code_worktree_prepare",
        CodePreparedWorktreeSchema,
        { input: CodeWorktreePrepareInputSchema.parse(input) },
      ),
    getCodeWorktreeStatus: (descriptor) =>
      parseInvocation(
        transport,
        "code_worktree_status",
        CodeWorktreeStatusSchema,
        { descriptor: CodeWorktreeDescriptorSchema.parse(descriptor) },
      ),
    listCodeThreadPreparations: (input) =>
      parseInvocation(
        transport,
        "code_thread_preparations_list",
        CodeThreadPreparationSchema.array(),
        { input: CodeThreadPreparationListInputSchema.parse(input) },
      ),
    listCodeThreads: (input) =>
      parseInvocation(transport, "code_threads_list", CodeThreadsPageSchema, {
        input: CodeThreadListInputSchema.parse(input),
      }),
    startCodeThread: (input) =>
      parseInvocation(
        transport,
        "code_thread_start",
        CodeBoundThreadOpenResultSchema,
        { input: CodeThreadStartInputSchema.parse(input) },
      ),
    recoverCodeThreadBinding: (input) =>
      parseInvocation(
        transport,
        "code_thread_binding_recover",
        CodeBoundThreadOpenResultSchema,
        { input: CodeThreadBindingRecoverInputSchema.parse(input) },
      ),
    resumeCodeThread: (input) =>
      parseInvocation(
        transport,
        "code_thread_resume",
        CodeBoundThreadOpenResultSchema,
        { input: CodeThreadResumeInputSchema.parse(input) },
      ),
    startCodeTurn: (input) =>
      parseInvocation(transport, "code_turn_start", CodeTurnSummarySchema, {
        input: CodeTurnStartInputSchema.parse(input),
      }),
    steerCodeTurn: (input) =>
      parseInvocation(transport, "code_turn_steer", CodeTurnSummarySchema, {
        input: CodeTurnSteerInputSchema.parse(input),
      }),
    interruptCodeTurn: async (input) => {
      await parseInvocation(
        transport,
        "code_turn_interrupt",
        CodeUnitResponseSchema,
        { input: CodeTurnInterruptInputSchema.parse(input) },
      );
    },
    respondToCodeApproval: async (input) => {
      await parseInvocation(
        transport,
        "code_approval_respond",
        CodeUnitResponseSchema,
        { input: CodeApprovalResponseInputSchema.parse(input) },
      );
    },
    listenForCodeWorkspaceEvents,
    listenAndReplayCodeWorkspaceEvents: async (input, handlers, options) => {
      const parsedInput = CodeRuntimeEventsInputSchema.parse(input);
      const subscriptionEpoch = validateSubscriptionEpoch(
        options.subscriptionEpoch,
      );
      if (
        options.scope !== undefined &&
        !codeScopesEqual(options.scope, parsedInput.scope)
      ) {
        throw new TypeError(
          "Replay option scope must match the replay input scope",
        );
      }
      const { signal } = options;
      if (signal?.aborted) throw replayAbortError();

      let active = true;
      let buffering = true;
      const bufferedEvents: CodeWorkspaceEvent[] = [];
      const droppedMaxSequence = new Map<number, number>();
      let highestSeenGeneration = -1;
      let decodeError: unknown = null;
      let unlisten: UnlistenFn | null = null;
      let listenerCleaned = false;
      let rejectFatalDecode: (error: unknown) => void = () => {};
      const fatalDecode = new Promise<never>((_resolve, reject) => {
        rejectFatalDecode = reject;
      });

      const cleanListener = (registeredUnlisten?: UnlistenFn) => {
        const cleanup = registeredUnlisten ?? unlisten;
        if (cleanup === null || cleanup === undefined || listenerCleaned) {
          return;
        }
        listenerCleaned = true;
        cleanup();
      };

      const dispose = () => {
        if (!active) return;
        active = false;
        signal?.removeEventListener("abort", dispose);
        cleanListener();
      };
      signal?.addEventListener("abort", dispose, { once: true });

      const listenerPromise = listenForCodeWorkspaceEvents(
        (event) => {
          if (!active || !codeScopesEqual(event.scope, parsedInput.scope)) {
            return;
          }
          highestSeenGeneration = Math.max(
            highestSeenGeneration,
            event.runtimeGeneration,
          );
          if (buffering) {
            if (bufferedEvents.length === MAX_BUFFERED_EVENTS) {
              const dropped = bufferedEvents.shift();
              if (dropped !== undefined) {
                droppedMaxSequence.set(
                  dropped.runtimeGeneration,
                  Math.max(
                    droppedMaxSequence.get(dropped.runtimeGeneration) ?? -1,
                    dropped.sequence,
                  ),
                );
              }
            }
            bufferedEvents.push(event);
          } else {
            handlers.onEvent(event, subscriptionEpoch);
          }
        },
        (error) => {
          if (!active) return;
          decodeError = error;
          dispose();
          rejectFatalDecode(error);
          try {
            handlers.onError(error);
          } catch {
            // The decode error remains the subscription's terminal reason.
          }
        },
        parsedInput.scope,
      ).then((registeredUnlisten) => {
        if (!active) {
          cleanListener(registeredUnlisten);
          return () => {};
        }
        return registeredUnlisten;
      });

      try {
        unlisten = await waitWithAbort(
          promiseWithFatalDecode(listenerPromise, fatalDecode),
          signal,
        );
        if (!active) {
          cleanListener();
          if (decodeError !== null) throw decodeError;
          throw replayAbortError();
        }

        let request = parsedInput;
        for (
          let attempt = 0;
          attempt < MAX_GENERATION_REPLAY_ATTEMPTS;
          attempt += 1
        ) {
          const backlog = await waitWithAbort(
            promiseWithFatalDecode(getCodeRuntimeEvents(request), fatalDecode),
            signal,
          );
          if (decodeError !== null) throw decodeError;
          if (!active) throw replayAbortError();

          const generationChanged =
            request.runtimeGeneration !== null &&
            request.runtimeGeneration !== backlog.runtimeGeneration;
          if (generationChanged) {
            request = {
              scope: parsedInput.scope,
              runtimeGeneration: backlog.runtimeGeneration,
              afterSequence: 0,
            };
            continue;
          }

          if (highestSeenGeneration > backlog.runtimeGeneration) {
            request = {
              scope: parsedInput.scope,
              runtimeGeneration: highestSeenGeneration,
              afterSequence: 0,
            };
            continue;
          }

          const finalBufferedEvents = bufferedEvents.filter(
            (event) => event.runtimeGeneration === backlog.runtimeGeneration,
          );
          const bufferTruncated =
            (droppedMaxSequence.get(backlog.runtimeGeneration) ?? -1) >
            backlog.latestSequence;
          buffering = false;
          handlers.onReplay({
            subscriptionEpoch,
            request,
            backlog,
            bufferedEvents: finalBufferedEvents,
            bufferTruncated,
          });
          return dispose;
        }
        throw replayUnstableError();
      } catch (error) {
        dispose();
        throw error;
      }
    },
  };
}

const defaultCodeWorkspaceTransport: CodeWorkspaceTransport = {
  invoke: (command, args) => invokeTauri<unknown>(command, args),
  listen: (eventName, handler) =>
    tauriListen<unknown>(eventName, (event) => handler(event)),
};

/** Default production adapter backed by the Tauri invoke and event APIs. */
export const codeWorkspaceApi = createCodeWorkspaceApi(
  defaultCodeWorkspaceTransport,
);

export const {
  probeCodeRuntime,
  startCodeRuntime,
  stopCodeRuntime,
  getCodeRuntimeStatus,
  getCodeRuntimeEvents,
  inspectCodeRepository,
  prepareCodeWorktree,
  getCodeWorktreeStatus,
  listCodeThreadPreparations,
  listCodeThreads,
  startCodeThread,
  recoverCodeThreadBinding,
  resumeCodeThread,
  startCodeTurn,
  steerCodeTurn,
  interruptCodeTurn,
  respondToCodeApproval,
  listenForCodeWorkspaceEvents,
  listenAndReplayCodeWorkspaceEvents,
} = codeWorkspaceApi;

/** Recover the structured native thread-start rejection without parsing copy. */
export function getCodeThreadStartError(
  error: unknown,
): CodeThreadStartError | null {
  const payload =
    error instanceof TauriInvokeError
      ? error.payload
      : typeof error === "object" && error !== null && "payload" in error
        ? error.payload
        : error;
  const parsed = CodeThreadStartErrorSchema.safeParse(payload);
  return parsed.success ? parsed.data : null;
}
