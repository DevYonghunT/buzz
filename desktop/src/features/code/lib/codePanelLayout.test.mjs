import assert from "node:assert/strict";
import test from "node:test";

import {
  CODE_CHANGES_PANEL_DEFAULT_WIDTH_PX,
  CODE_CHANGES_PANEL_MIN_WIDTH_PX,
  CODE_CONVERSATION_MIN_WIDTH_PX,
  CODE_PANEL_KEYBOARD_STEP_PX,
  CODE_TASKS_PANEL_DEFAULT_WIDTH_PX,
  CODE_TASKS_PANEL_MIN_WIDTH_PX,
  codePanelWidthFromKey,
  codePanelWidthFromPointer,
  readCodePanelWidths,
  resolveCodePanelLayout,
  writeCodePanelWidths,
} from "./codePanelLayout.ts";

function memoryStorage(initial = null) {
  let value = initial;
  return {
    getItem() {
      return value;
    },
    setItem(_key, next) {
      value = next;
    },
    value() {
      return value;
    },
  };
}

test("Code panel widths persist together and reject malformed snapshots", () => {
  const storage = memoryStorage();
  const widths = { tasks: 304, changes: 612 };
  writeCodePanelWidths(storage, widths);
  assert.deepEqual(readCodePanelWidths(storage), widths);

  assert.deepEqual(readCodePanelWidths(memoryStorage("not-json")), {
    tasks: CODE_TASKS_PANEL_DEFAULT_WIDTH_PX,
    changes: CODE_CHANGES_PANEL_DEFAULT_WIDTH_PX,
  });
  assert.deepEqual(
    readCodePanelWidths(memoryStorage('{"tasks":-1,"changes":9999}')),
    { tasks: CODE_TASKS_PANEL_MIN_WIDTH_PX, changes: 720 },
  );
});

test("wide layouts preserve a useful conversation between both side panes", () => {
  const containerWidth = 1_260;
  const layout = resolveCodePanelLayout({
    changesOpen: true,
    containerWidth,
    preferred: { tasks: 420, changes: 720 },
    tasksOpen: true,
  });

  assert.equal(layout.inspectorDocked, true);
  assert.ok(layout.tasks.width >= CODE_TASKS_PANEL_MIN_WIDTH_PX);
  assert.ok(layout.changes.width >= CODE_CHANGES_PANEL_MIN_WIDTH_PX);
  assert.ok(
    containerWidth - layout.tasks.width - layout.changes.width >=
      CODE_CONVERSATION_MIN_WIDTH_PX,
  );
});

test("narrow layouts overlay Changes instead of crushing the conversation", () => {
  const layout = resolveCodePanelLayout({
    changesOpen: true,
    containerWidth: 760,
    preferred: { tasks: 260, changes: 600 },
    tasksOpen: true,
  });

  assert.equal(layout.inspectorDocked, false);
  assert.ok(760 - layout.tasks.width >= CODE_CONVERSATION_MIN_WIDTH_PX);
  assert.ok(layout.changes.width <= 760 - layout.tasks.width - 48);
  assert.ok(760 - layout.tasks.width - layout.changes.width >= 48);
});

test("very narrow layouts keep an open Tasks pane usable", () => {
  const layout = resolveCodePanelLayout({
    changesOpen: false,
    containerWidth: 500,
    preferred: { tasks: 240, changes: 544 },
    tasksOpen: true,
  });

  assert.equal(layout.tasks.min, CODE_TASKS_PANEL_MIN_WIDTH_PX);
  assert.equal(layout.tasks.width, CODE_TASKS_PANEL_MIN_WIDTH_PX);
});

test("pointer and keyboard resizing follow the separator one-to-one", () => {
  assert.equal(
    codePanelWidthFromPointer({
      currentX: 346,
      growDirection: 1,
      max: 420,
      min: 200,
      startWidth: 240,
      startX: 300,
    }),
    286,
  );
  assert.equal(
    codePanelWidthFromPointer({
      currentX: 260,
      growDirection: -1,
      max: 720,
      min: 320,
      startWidth: 544,
      startX: 300,
    }),
    584,
  );
  assert.equal(
    codePanelWidthFromKey({
      growDirection: 1,
      key: "ArrowRight",
      max: 420,
      min: 200,
      width: 240,
    }),
    240 + CODE_PANEL_KEYBOARD_STEP_PX,
  );
  assert.equal(
    codePanelWidthFromKey({
      growDirection: -1,
      key: "Home",
      max: 720,
      min: 320,
      width: 544,
    }),
    720,
  );
  assert.equal(
    codePanelWidthFromKey({
      growDirection: 1,
      key: "Enter",
      max: 420,
      min: 200,
      width: 240,
    }),
    null,
  );
});
