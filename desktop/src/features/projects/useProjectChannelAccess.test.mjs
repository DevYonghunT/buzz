import assert from "node:assert/strict";
import { test } from "node:test";

import {
  ensureProjectChannelAccess,
  ProjectChannelAccessError,
  projectChannelAccessDecision,
  projectChannelJoinErrorCode,
} from "./useProjectChannelAccess.ts";

const CHANNEL_ID = "11111111-1111-4111-8111-111111111111";

function channel(overrides = {}) {
  return {
    id: CHANNEL_ID,
    name: "project",
    channelType: "stream",
    visibility: "open",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 1,
    memberPubkeys: [],
    lastMessageAt: null,
    archivedAt: null,
    participants: [],
    participantPubkeys: [],
    isMember: false,
    ttlSeconds: null,
    ttlDeadline: null,
    ...overrides,
  };
}

function dependencies({ reads, joinError = null }) {
  const calls = [];
  let readIndex = 0;
  return {
    calls,
    value: {
      getChannels: async () => {
        calls.push("getChannels");
        const value = reads[Math.min(readIndex, reads.length - 1)];
        readIndex += 1;
        if (value instanceof Error) throw value;
        return value;
      },
      joinChannel: async (channelId) => {
        calls.push(`joinChannel:${channelId}`);
        if (joinError) throw joinError;
      },
    },
  };
}

test("access decision keeps members, joins open channels, and blocks private channels", () => {
  assert.equal(
    projectChannelAccessDecision([channel({ isMember: true })], CHANNEL_ID)
      .kind,
    "already-member",
  );
  assert.equal(
    projectChannelAccessDecision([channel()], CHANNEL_ID).kind,
    "join",
  );
  assert.equal(
    projectChannelAccessDecision(
      [channel({ visibility: "private" })],
      CHANNEL_ID,
    ).kind,
    "invite-required",
  );
  assert.equal(projectChannelAccessDecision([], CHANNEL_ID).kind, "join");
});

test("existing membership performs one read and no join", async () => {
  const fixture = dependencies({
    reads: [[channel({ isMember: true, visibility: "private" })]],
  });

  const result = await ensureProjectChannelAccess(CHANNEL_ID, fixture.value);

  assert.equal(result.status, "already-member");
  assert.deepEqual(fixture.calls, ["getChannels"]);
});

test("open non-members join before a second membership read", async () => {
  const fixture = dependencies({
    reads: [[channel()], [channel({ isMember: true })]],
  });

  const result = await ensureProjectChannelAccess(CHANNEL_ID, fixture.value);

  assert.equal(result.status, "joined");
  assert.deepEqual(fixture.calls, [
    "getChannels",
    `joinChannel:${CHANNEL_ID}`,
    "getChannels",
  ]);
});

test("a missing discovery row safely recovers by joining and refreshing", async () => {
  const fixture = dependencies({
    reads: [[], [channel({ isMember: true })]],
  });

  const result = await ensureProjectChannelAccess(CHANNEL_ID, fixture.value);

  assert.equal(result.status, "joined");
  assert.deepEqual(fixture.calls, [
    "getChannels",
    `joinChannel:${CHANNEL_ID}`,
    "getChannels",
  ]);
});

test("private non-members receive an invite-required error without joining", async () => {
  const fixture = dependencies({
    reads: [[channel({ visibility: "private" })]],
  });

  await assert.rejects(
    ensureProjectChannelAccess(CHANNEL_ID, fixture.value),
    (error) =>
      error instanceof ProjectChannelAccessError &&
      error.code === "invite-required" &&
      error.message.includes("invite this account"),
  );
  assert.deepEqual(fixture.calls, ["getChannels"]);
});

test("relay join refusals distinguish private and unavailable channels", async () => {
  assert.equal(
    projectChannelJoinErrorCode(
      new Error(
        "relay returned 400 Bad Request: restricted: channel is private",
      ),
    ),
    "invite-required",
  );
  assert.equal(
    projectChannelJoinErrorCode(
      "relay returned 400 Bad Request: invalid: channel not found",
    ),
    "channel-unavailable",
  );

  for (const [message, code] of [
    ["restricted: channel is private", "invite-required"],
    ["invalid: channel not found", "channel-unavailable"],
  ]) {
    const fixture = dependencies({
      reads: [[]],
      joinError: new Error(message),
    });
    await assert.rejects(
      ensureProjectChannelAccess(CHANNEL_ID, fixture.value),
      (error) =>
        error instanceof ProjectChannelAccessError && error.code === code,
    );
  }
});

test("an accepted join proceeds when the membership projection stays stale", async () => {
  const fixture = dependencies({ reads: [[channel()], [channel()]] });

  const result = await ensureProjectChannelAccess(CHANNEL_ID, fixture.value);

  assert.equal(result.status, "joined");
  assert.equal(result.channels?.[0]?.isMember, false);
  assert.deepEqual(fixture.calls, [
    "getChannels",
    `joinChannel:${CHANNEL_ID}`,
    "getChannels",
  ]);
});

test("a retry observes membership without issuing a duplicate join", async () => {
  const fixture = dependencies({
    reads: [[channel()], [channel()], [channel({ isMember: true })]],
  });

  const first = await ensureProjectChannelAccess(CHANNEL_ID, fixture.value);
  const retry = await ensureProjectChannelAccess(CHANNEL_ID, fixture.value);

  assert.equal(first.status, "joined");
  assert.equal(retry.status, "already-member");
  assert.deepEqual(fixture.calls, [
    "getChannels",
    `joinChannel:${CHANNEL_ID}`,
    "getChannels",
    "getChannels",
  ]);
});

test("a failed post-join cache refresh does not block authoritative Git", async () => {
  const fixture = dependencies({
    reads: [[channel()], new Error("membership projection unavailable")],
  });

  const result = await ensureProjectChannelAccess(CHANNEL_ID, fixture.value);

  assert.equal(result.status, "joined");
  assert.equal(result.channels, null);
  assert.deepEqual(fixture.calls, [
    "getChannels",
    `joinChannel:${CHANNEL_ID}`,
    "getChannels",
  ]);
});

test("unknown join failures propagate unchanged and skip the cache refresh", async () => {
  const networkError = new Error("connection reset");
  const fixture = dependencies({ reads: [[]], joinError: networkError });

  await assert.rejects(
    ensureProjectChannelAccess(CHANNEL_ID, fixture.value),
    (error) => error === networkError,
  );
  assert.deepEqual(fixture.calls, ["getChannels", `joinChannel:${CHANNEL_ID}`]);
});

test("unbound legacy announcements preserve the existing terminal path", async () => {
  const fixture = dependencies({ reads: [[]] });

  const result = await ensureProjectChannelAccess(null, fixture.value);

  assert.equal(result.status, "unbound");
  assert.deepEqual(fixture.calls, []);
});
