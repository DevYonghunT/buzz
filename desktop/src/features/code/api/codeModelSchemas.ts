import { z } from "zod";

const CodeSafeUnsignedIntegerSchema = z
  .number()
  .int()
  .nonnegative()
  .max(Number.MAX_SAFE_INTEGER);

export const CodeModelIdentifierSchema = z
  .string()
  .min(1)
  .refine((value) => value.trim().length > 0, "Must not be blank");

export const CodeReasoningEffortOptionSchema = z.strictObject({
  reasoningEffort: CodeModelIdentifierSchema,
  description: z.string(),
});

export const CodeModelOptionSchema = z
  .strictObject({
    id: CodeModelIdentifierSchema,
    model: CodeModelIdentifierSchema,
    displayName: CodeModelIdentifierSchema,
    description: z.string(),
    isDefault: z.boolean(),
    defaultReasoningEffort: CodeModelIdentifierSchema,
    supportedReasoningEfforts: z.array(CodeReasoningEffortOptionSchema).min(1),
  })
  .superRefine((model, context) => {
    const efforts = new Set<string>();
    for (const [index, option] of model.supportedReasoningEfforts.entries()) {
      if (efforts.has(option.reasoningEffort)) {
        context.addIssue({
          code: "custom",
          message: "Reasoning efforts must be unique within a model",
          path: ["supportedReasoningEfforts", index, "reasoningEffort"],
        });
      }
      efforts.add(option.reasoningEffort);
    }
    if (!efforts.has(model.defaultReasoningEffort)) {
      context.addIssue({
        code: "custom",
        message: "The default reasoning effort must be supported",
        path: ["defaultReasoningEffort"],
      });
    }
  });

export const CodeModelSelectionSchema = z.strictObject({
  model: CodeModelIdentifierSchema,
  reasoningEffort: CodeModelIdentifierSchema,
});

export const CodeModelSelectionInputSchema = CodeModelSelectionSchema;

export const CodeModelsCatalogSchema = z
  .strictObject({
    runtimeGeneration: CodeSafeUnsignedIntegerSchema,
    models: z.array(CodeModelOptionSchema).min(1),
    recentSelection: CodeModelSelectionSchema.nullable(),
  })
  .superRefine((catalog, context) => {
    const ids = new Set<string>();
    const models = new Set<string>();
    let defaults = 0;
    for (const [index, option] of catalog.models.entries()) {
      if (ids.has(option.id)) {
        context.addIssue({
          code: "custom",
          message: "Model preset ids must be unique",
          path: ["models", index, "id"],
        });
      }
      if (models.has(option.model)) {
        context.addIssue({
          code: "custom",
          message: "Model slugs must be unique",
          path: ["models", index, "model"],
        });
      }
      ids.add(option.id);
      models.add(option.model);
      if (option.isDefault) defaults += 1;
    }
    if (defaults > 1) {
      context.addIssue({
        code: "custom",
        message: "At most one model can be the catalog default",
        path: ["models"],
      });
    }
    if (catalog.recentSelection !== null) {
      const recentModel = catalog.models.find(
        (option) => option.model === catalog.recentSelection?.model,
      );
      if (
        recentModel === undefined ||
        !recentModel.supportedReasoningEfforts.some(
          (option) =>
            option.reasoningEffort === catalog.recentSelection?.reasoningEffort,
        )
      ) {
        context.addIssue({
          code: "custom",
          message: "The recent selection must exist in the model catalog",
          path: ["recentSelection"],
        });
      }
    }
  });
