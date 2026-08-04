//! Live-relay end-to-end tests for kind:39500 (SchoolX workspace-catalog
//! provenance manifest — `docs/schoolx-2/WORKSPACE_CATALOG.md` §4).
//!
//! Everything upstream of this file — the read-only catalog, deterministic
//! channel ids, the `Provenance` wire type, preflight decisions, the
//! idempotent apply saga, Tauri commands, the settings screen, and the CLI —
//! is built and unit-tested against an in-memory fake
//! (`cargo test -p schoolx-catalog`). A fake cannot prove properties of the
//! *real* relay. Four such properties are exactly what everything upstream
//! assumes, and nothing else confirms them:
//!
//! 1. **The relay accepts kind 39500 at all.** Nothing upstream has ever sent
//!    one to a running relay. `provenance_round_trips_through_the_relay`
//!    checks acceptance and readback by the coordinate the design doc
//!    specifies; `second_publish_replaces_the_first` checks that the NIP-33
//!    LWW replacement the saga's idempotent retries depend on actually
//!    applies to this kind.
//! 2. **A non-member cannot read a private channel's provenance.** Provenance
//!    is stored channel-scoped (an `h` tag, same as any other channel
//!    content) precisely so a private channel's ACL covers it too — see
//!    WORKSPACE_CATALOG.md §4 "의도적 트레이드오프": a global-scope
//!    provenance store would let a non-member learn a private channel's
//!    existence from provenance alone, breaking `SECURITY_CONTRACT.md`
//!    (session A) at a path that document's own test suite
//!    (`e2e_access_matrix.rs`) never touches, because it predates kind 39500.
//!    `non_member_cannot_read_provenance` is the check.
//! 3. **A deleted channel's UUID stays burned.** Per WORKSPACE_CATALOG.md §6,
//!    a soft-deleted channel row keeps occupying its `(community_id, id)`
//!    primary key, so `create_channel_with_id`'s
//!    `ON CONFLICT (community_id, id) DO NOTHING` makes a same-id recreate a
//!    no-op — the saga's *only* signal for "this was deleted" versus "this
//!    was never created". The desktop adapter's `is_duplicate_channel_rejection`
//!    (`desktop/src-tauri/src/commands/workspace_catalog.rs`) pattern-matches
//!    the relay's exact rejection string, `"duplicate: channel already
//!    exists"`. `deleted_channel_id_is_burned` proves the relay still says
//!    exactly that after a soft delete.
//! 4. **A squatted channel does not read back as the victim's own.** Per
//!    `docs/schoolx-2/CATALOG_SECURITY.md` §5·§6, adoption asks two questions
//!    of the relay, and both answers must name the squatter rather than the
//!    admin who arrives second: *who signed this provenance record*
//!    (`ProvenanceRecord::signer`, the event's own `pubkey`) and *who created
//!    this channel* (`channel_owner`, the `created_by` tag on kind:39000).
//!    Anchoring on the creator is what makes the strong form of the squat
//!    unreachable — the squatter can grant the victim any role, including
//!    `owner`, but cannot rewrite who created the room.
//!    `squatted_channel_provenance_is_signed_by_the_squatter` fixes both
//!    answers at the relay level.
//!
//! # Harness reuse
//!
//! Nothing here is a new harness. It is the split `e2e_access_matrix.rs`
//! already uses: HTTP bridge (`POST /events` with the dev `X-Pubkey` header)
//! for NIP-29 channel lifecycle (kind:9007 create, kind:9008 delete), and
//! `BuzzTestClient` over WebSocket (`connect`, `send_event`, `subscribe`,
//! `collect_until_eose`) for publishing and reading channel content — the
//! same pairing `e2e_team.rs` and `e2e_persona.rs` use for parameterized-
//! replaceable content specifically.
//!
//! `buzz-test-client` does not gain a dependency on `schoolx-catalog` here.
//! The provenance content below is a JSON literal matching the *current*
//! wire format in `crates/schoolx-catalog/src/provenance.rs` (field names,
//! and `StepStatus` spellings `pending`/`done`/`failed`/`skipped`/
//! `unrecognized`) — so what gets verified is what the relay accepts on the
//! wire, independent of whether our own Rust types have drifted from it.
//!
//! # Running
//!
//! ```text
//! just test-e2e e2e_workspace_catalog
//! ```

use std::time::Duration;

use buzz_test_client::BuzzTestClient;
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag, Timestamp};

/// Must match `schoolx_catalog::provenance::KIND_WORKSPACE_PROVENANCE` /
/// `buzz_core::kind::KIND_WORKSPACE_PROVENANCE` (both 39500, cross-checked
/// against each other by that crate's own tests). Hardcoded locally rather
/// than imported, matching every sibling E2E file's convention of a local
/// `_KIND` constant (`CREATE_GROUP_KIND` etc. below, `STREAM_MESSAGE_KIND` in
/// `e2e_access_matrix.rs`, `TEAM_KIND` in `e2e_team.rs`).
const WORKSPACE_PROVENANCE_KIND: u16 = 39500;
const CREATE_GROUP_KIND: u16 = 9007;
const DELETE_GROUP_KIND: u16 = 9008;
const PUT_USER_KIND: u16 = 9000;
/// NIP-29 group metadata, relay-signed. Carries the `created_by` tag that
/// `channel_owner` (`desktop/src-tauri/src/commands/workspace_catalog.rs`)
/// reads to decide whether a channel is ours. Relay-only authorship is what
/// makes that tag trustworthy; `e2e_relay.rs`'s
/// `test_client_submitted_nip29_group_metadata_and_admins_are_rejected` is
/// the regression test for the invariant itself, so this file only reads.
const GROUP_METADATA_KIND: u16 = 39000;

/// `<catalog_id>:<item_key>` from the brief, reused verbatim across every
/// test. Safe to repeat: NIP-33 replacement keys on `(kind, pubkey, d_tag)`,
/// not on channel, and every test signs with its own fresh `Keys::generate()`
/// — so identical `d` tags across tests never collide.
const D_TAG: &str = "schoolx.default:meeting";

const EOSE_WAIT: Duration = Duration::from_secs(10);

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn relay_http_url() -> String {
    relay_url()
        .replacen("ws://", "http://", 1)
        .replacen("wss://", "https://", 1)
}

fn sub_id(name: &str) -> String {
    format!("e2e-workspace-catalog-{name}-{}", uuid::Uuid::new_v4())
}

// ─── channel lifecycle (HTTP bridge, mirrors e2e_access_matrix.rs) ─────────

/// POST a signed event to the bridge with the dev `X-Pubkey` header. Mirrors
/// `e2e_access_matrix.rs::post_event_as` minus the NIP-OA tag parameter —
/// these tests only ever exercise plain human principals.
async fn post_event_as(keys: &Keys, event: &nostr::Event) -> serde_json::Value {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(event).unwrap())
        .send()
        .await
        .expect("POST /events");
    resp.json().await.expect("parse /events response")
}

fn create_group_event(keys: &Keys, channel_id: uuid::Uuid, visibility: &str) -> nostr::Event {
    EventBuilder::new(Kind::Custom(CREATE_GROUP_KIND), "")
        .tags(vec![
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["name", &format!("workspace-catalog-{channel_id}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", visibility]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap()
}

/// Create a fresh channel owned by `keys`, asserting the create is accepted.
/// Mirrors `e2e_access_matrix.rs::create_channel`. The creator becomes the
/// channel's `owner`-role member (NIP-29 CREATE_GROUP), which both the
/// provenance write (`h`-scoped membership) and `delete_channel` below
/// (owner-only) rely on.
async fn create_channel(keys: &Keys, visibility: &str) -> uuid::Uuid {
    let channel_id = uuid::Uuid::new_v4();
    let body = post_event_as(keys, &create_group_event(keys, channel_id, visibility)).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "channel creation not accepted ({visibility}): {body}"
    );
    channel_id
}

/// Attempt a kind:9007 create at an already-decided `channel_id`, returning
/// the raw `/events` response so the caller can assert on the rejection
/// itself rather than just pass/fail.
async fn try_create_channel_at(
    keys: &Keys,
    channel_id: uuid::Uuid,
    visibility: &str,
) -> serde_json::Value {
    post_event_as(keys, &create_group_event(keys, channel_id, visibility)).await
}

/// Soft-delete a channel via kind:9008, signed by its owner.
async fn delete_channel(owner: &Keys, channel_id: uuid::Uuid) {
    let event = EventBuilder::new(Kind::Custom(DELETE_GROUP_KIND), "")
        .tags(vec![Tag::parse(["h", &channel_id.to_string()]).unwrap()])
        .sign_with_keys(owner)
        .unwrap();
    let body = post_event_as(owner, &event).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "channel deletion not accepted: {body}"
    );
}

/// Add `target` to `channel_id` as a member, signed by the channel owner.
/// Mirrors `e2e_access_matrix.rs::add_member` (kind:9000 NIP-29 PUT_USER).
async fn add_member(owner: &Keys, channel_id: uuid::Uuid, target: &Keys) {
    let event = EventBuilder::new(Kind::Custom(PUT_USER_KIND), "")
        .tags(vec![
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["p", &target.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(owner)
        .unwrap();
    let body = post_event_as(owner, &event).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "add_member not accepted: {body}"
    );
}

/// Add `target` at an explicit `role`, signed by an elevated member.
///
/// Same kind:9000 as `add_member`, with the `role` tag the relay's PUT_USER
/// handler reads (`side_effects.rs`). Split out rather than folded into
/// `add_member` because the two say different things: `add_member` grants
/// access, this grants *standing*. Only `squatted_channel_provenance_is_signed_by_the_squatter`
/// needs the latter, and it needs it to hand out the one role that used to
/// defeat adoption.
async fn grant_role(granter: &Keys, channel_id: uuid::Uuid, target: &Keys, role: &str) {
    let event = EventBuilder::new(Kind::Custom(PUT_USER_KIND), "")
        .tags(vec![
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["p", &target.public_key().to_hex()]).unwrap(),
            Tag::parse(["role", role]).unwrap(),
        ])
        .sign_with_keys(granter)
        .unwrap();
    let body = post_event_as(granter, &event).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "grant_role({role}) not accepted: {body}"
    );
}

/// First value of the named tag, or `None`. Mirrors the desktop adapter's
/// `first_tag_value` (`nostr_convert.rs`) so this test reads the event the
/// same way production does.
fn first_tag_value(event: &nostr::Event, name: &str) -> Option<String> {
    event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        (s.len() >= 2 && s[0] == name).then(|| s[1].clone())
    })
}

/// Read a channel's relay-signed kind:39000 by its `d` tag (the channel id).
///
/// Filters by `#d` rather than `#h` for the reason spelled out on
/// `read_provenance`: an `#h` filter takes the relay's per-channel REQ branch,
/// which can answer `CLOSED` where `collect_until_eose` cannot tell that apart
/// from a hang. kind:39000 keys its own `d` tag to the channel id
/// (`emit_group_discovery_events`), so `#d` addresses one channel exactly.
async fn read_group_metadata(
    client: &mut BuzzTestClient,
    channel_id: uuid::Uuid,
    name: &str,
) -> Vec<nostr::Event> {
    let sid = sub_id(name);
    let filter = Filter::new()
        .kind(Kind::Custom(GROUP_METADATA_KIND))
        .custom_tags(
            SingleLetterTag::lowercase(Alphabet::D),
            [channel_id.to_string()],
        );
    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe group metadata");
    let events = client
        .collect_until_eose(&sid, EOSE_WAIT)
        .await
        .expect("collect group metadata");
    client.close_subscription(&sid).await.ok();
    events
}

// ─── provenance publish + read (WS, mirrors e2e_team.rs / e2e_persona.rs) ──

/// The `Provenance` content shape from `crates/schoolx-catalog/src/provenance.rs`,
/// current as of this task (field names and `StepStatus` spellings verified
/// there — the brief's own three-value example predates `Skipped`/
/// `Unrecognized` but happens to still be valid, since it only uses the
/// `done`/`pending` spellings that were never renamed).
fn provenance_content(steps: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "catalog_id": "schoolx.default",
        "catalog_version": 1,
        "item_key": "meeting",
        "generation": 1,
        "steps": steps,
        "applied_at": "2026-07-28T09:00:00Z"
    })
}

fn provenance_event_at(
    keys: &Keys,
    channel_id: uuid::Uuid,
    content: &serde_json::Value,
    created_at: Timestamp,
) -> nostr::Event {
    EventBuilder::new(Kind::Custom(WORKSPACE_PROVENANCE_KIND), content.to_string())
        .tags(vec![
            Tag::parse(["d", D_TAG]).unwrap(),
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
        ])
        .custom_created_at(created_at)
        .sign_with_keys(keys)
        .unwrap()
}

fn provenance_event(
    keys: &Keys,
    channel_id: uuid::Uuid,
    content: &serde_json::Value,
) -> nostr::Event {
    EventBuilder::new(Kind::Custom(WORKSPACE_PROVENANCE_KIND), content.to_string())
        .tags(vec![
            Tag::parse(["d", D_TAG]).unwrap(),
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap()
}

/// The exact filter the brief specifies: `kinds` + `#d`, deliberately no `#h`
/// and no `authors`. The point of `non_member_cannot_read_provenance` is that
/// the channel ACL gates this on its own, from the event's stored
/// `channel_id` column — a filter that scoped by channel or author would have
/// the test do the relay's job for it instead of checking that the relay does
/// it.
fn provenance_filter() -> Filter {
    Filter::new()
        .kind(Kind::Custom(WORKSPACE_PROVENANCE_KIND))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [D_TAG])
}

/// Reads via `collect_until_eose`, which silently drops a relay `CLOSED` and
/// then just waits out the deadline — safe here only because
/// `provenance_filter()` carries no `#h` tag. With an `#h` tag the relay
/// takes a per-channel branch (`req.rs`) that can answer with `CLOSED`
/// instead of an empty `EOSE`, which `collect_until_eose` cannot tell apart
/// from a hang; `e2e_access_matrix.rs`'s `ws_read`/`ReadOutcome` exists for
/// exactly that case.
async fn read_provenance(client: &mut BuzzTestClient, name: &str) -> Vec<nostr::Event> {
    let sid = sub_id(name);
    client
        .subscribe(&sid, vec![provenance_filter()])
        .await
        .expect("subscribe");
    let events = client
        .collect_until_eose(&sid, EOSE_WAIT)
        .await
        .expect("collect");
    client.close_subscription(&sid).await.ok();
    events
}

fn parsed_content(event: &nostr::Event) -> serde_json::Value {
    serde_json::from_str(&event.content).expect("provenance content must be valid JSON")
}

// ─── fact 1: the relay accepts kind 39500 and returns it by (kind, #d) ─────

/// Publishes a provenance event to a private channel and reads it back by
/// the exact `{"kinds":[39500],"#d":[...]}` coordinate the brief specifies.
/// This is the base confirmation that the relay accepts kind 39500 at all —
/// nothing upstream of this test (saga, Tauri commands, settings screen,
/// CLI) has ever gotten further than the in-memory fake.
#[tokio::test]
#[ignore]
async fn provenance_round_trips_through_the_relay() {
    let owner = Keys::generate();
    let channel_id = create_channel(&owner, "private").await;

    let content = provenance_content(serde_json::json!({
        "channel": "done",
        "canvas": "pending",
        "membership": "pending"
    }));

    let mut client = BuzzTestClient::connect(&relay_url(), &owner)
        .await
        .expect("owner connect");
    let ok = client
        .send_event(provenance_event(&owner, channel_id, &content))
        .await
        .expect("publish provenance");
    assert!(ok.accepted, "relay rejected kind 39500: {}", ok.message);

    let events = read_provenance(&mut client, "round-trip").await;

    assert_eq!(
        events.len(),
        1,
        "expected exactly one provenance event, got {events:?}"
    );
    assert_eq!(
        parsed_content(&events[0]),
        content,
        "stored content must match what was published"
    );

    client.disconnect().await.ok();
}

/// Republishes at the same `d` with different `steps` and confirms the relay
/// leaves exactly one event behind, holding the newer content — NIP-33
/// last-write-wins, which the saga's idempotent step-by-step retries depend
/// on so a retried apply never accumulates a history of provenance records
/// for one workspace item.
#[tokio::test]
#[ignore]
async fn second_publish_replaces_the_first() {
    let owner = Keys::generate();
    let channel_id = create_channel(&owner, "private").await;

    let first_content = provenance_content(serde_json::json!({
        "channel": "done",
        "canvas": "pending",
        "membership": "pending"
    }));
    // `skipped` postdates the brief's three-value example — `StepStatus`
    // gained it (and `Unrecognized`) since. Using it here doubles as
    // confirmation that the currently-valid wire shape includes it.
    let second_content = provenance_content(serde_json::json!({
        "channel": "done",
        "canvas": "skipped",
        "membership": "done"
    }));

    let mut client = BuzzTestClient::connect(&relay_url(), &owner)
        .await
        .expect("owner connect");

    // Distinct, explicit timestamps: NIP-33 replacement orders by created_at
    // and then by lowest event id, so two same-second publishes would decide
    // "newer" by coincidence of id rather than by which one was actually sent
    // second. Mirrors e2e_team.rs's test_team_nip33_replacement_newer_wins.
    let now = Timestamp::now();
    let first_at = Timestamp::from(now.as_secs() - 100);

    let ok = client
        .send_event(provenance_event_at(
            &owner,
            channel_id,
            &first_content,
            first_at,
        ))
        .await
        .expect("publish first");
    assert!(
        ok.accepted,
        "relay rejected first provenance: {}",
        ok.message
    );

    let ok = client
        .send_event(provenance_event_at(
            &owner,
            channel_id,
            &second_content,
            now,
        ))
        .await
        .expect("publish second");
    assert!(
        ok.accepted,
        "relay rejected second provenance: {}",
        ok.message
    );

    let events = read_provenance(&mut client, "replace").await;

    assert_eq!(
        events.len(),
        1,
        "NIP-33 LWW must leave exactly one event, got {events:?}"
    );
    assert_eq!(
        parsed_content(&events[0]),
        second_content,
        "the second publish must win"
    );

    client.disconnect().await.ok();
}

// ─── fact 2: a non-member cannot read a private channel's provenance ───────

/// A different authenticated user, running the exact same REQ, must see
/// nothing. Per WORKSPACE_CATALOG.md §4 "의도적 트레이드오프", provenance is
/// deliberately channel-scoped (not global) precisely so a private channel's
/// ACL covers it: a leak here would let any authenticated stranger learn a
/// private channel's existence just by probing catalog coordinates, which is
/// exactly what `SECURITY_CONTRACT.md` (session A) forbids for private
/// channels. This kind never carries the channel name itself, but a nonempty
/// result alone already confirms the channel exists — which is the leak.
///
/// Positive control: an empty result by itself cannot distinguish a genuine
/// ACL block from a typo'd filter, a wrong channel id, or a query that simply
/// does not work — proof the query works for an authorized reader otherwise
/// lives only in `provenance_round_trips_through_the_relay`, a different test
/// function. So after the stranger reads nothing, this test adds them to the
/// same channel and reruns the identical query on the identical connection:
/// same identity, same filter, same channel, only membership changed. The
/// empty-then-nonempty pair makes the block self-evidently the ACL's doing,
/// from inside this test alone.
#[tokio::test]
#[ignore]
async fn non_member_cannot_read_provenance() {
    let owner = Keys::generate();
    let stranger = Keys::generate();
    let channel_id = create_channel(&owner, "private").await;

    let content = provenance_content(serde_json::json!({
        "channel": "done",
        "canvas": "pending",
        "membership": "pending"
    }));

    let mut owner_client = BuzzTestClient::connect(&relay_url(), &owner)
        .await
        .expect("owner connect");
    let ok = owner_client
        .send_event(provenance_event(&owner, channel_id, &content))
        .await
        .expect("publish provenance");
    assert!(ok.accepted, "relay rejected kind 39500: {}", ok.message);
    owner_client.disconnect().await.ok();

    let mut stranger_client = BuzzTestClient::connect(&relay_url(), &stranger)
        .await
        .expect("stranger connect");
    let events = read_provenance(&mut stranger_client, "non-member").await;

    assert!(
        events.is_empty(),
        "non-member read a private channel's provenance: {events:?}"
    );

    // Positive control: grant the same stranger membership and rerun the
    // exact same query on the exact same connection. The PUT_USER handler
    // calls `state.invalidate_membership`, which synchronously drops both
    // `accessible_channels_cache` and `member_channels_cache` for the target
    // pubkey (`buzz-relay/src/state.rs`), and `req.rs` resolves accessible
    // channels fresh on every REQ — so, as in
    // `e2e_access_matrix.rs::agent_gains_access_immediately_on_add`, no
    // reconnect and no sleep are needed for the grant to be visible.
    add_member(&owner, channel_id, &stranger).await;

    let events_after_add = read_provenance(&mut stranger_client, "member-after-add").await;

    assert_eq!(
        events_after_add.len(),
        1,
        "newly added member still could not read the channel's provenance: {events_after_add:?}"
    );
    assert_eq!(
        parsed_content(&events_after_add[0]),
        content,
        "member-read provenance content must match what was published"
    );

    stranger_client.disconnect().await.ok();
}

// ─── fact 3: a deleted channel's UUID stays burned ─────────────────────────

/// Deletes a channel, then attempts to recreate a channel at the exact same
/// UUID. The relay must refuse it with the exact string the desktop saga
/// pattern-matches on (`is_duplicate_channel_rejection` in
/// `desktop/src-tauri/src/commands/workspace_catalog.rs`) to conclude "this
/// id was used and deleted", as opposed to any other creation failure. Per
/// WORKSPACE_CATALOG.md §6, this is the *only* signal the saga has for that
/// distinction — there is no tombstone or "was deleted" flag it can read
/// instead. If the relay ever allowed the UUID to be reused, or worded the
/// rejection differently, deletion detection silently stops working.
#[tokio::test]
#[ignore]
async fn deleted_channel_id_is_burned() {
    let owner = Keys::generate();
    let channel_id = create_channel(&owner, "private").await;

    delete_channel(&owner, channel_id).await;

    let body = try_create_channel_at(&owner, channel_id, "private").await;

    assert!(
        !body["accepted"].as_bool().unwrap_or(false),
        "relay allowed recreating a channel at a soft-deleted id: {body}"
    );
    assert_eq!(
        body["message"].as_str().unwrap_or_default(),
        "duplicate: channel already exists",
        "unexpected rejection reason (saga deletion-detection matches this exact string): {body}"
    );
}

// ─── fact 4: a squatted channel reads back as the squatter's, not ours ─────

/// Plays out the squat from `docs/schoolx-2/CATALOG_SECURITY.md` §1 against a
/// live relay and fixes the two values adoption decides on.
///
/// The squatter gets to a derived channel id first — every input to the
/// derivation is public (§1), so this needs no privilege at all. They publish
/// a complete-looking provenance record inside their own channel, which means
/// the channel-binding check session D added (`record_sits_in_its_derived_channel`)
/// passes: the record really is in the channel the derivation predicts. Then
/// they pull the admin in and hand them `owner`, which kind:9000 PUT_USER
/// permits without the target's consent.
///
/// That last move is the whole point of the design correction in §6. Adoption
/// used to ask the role table "am I an owner here?", and the answer the
/// squatter just manufactured was *yes*. So this test asserts what adoption
/// asks now instead:
///
/// - **The record's signer is the squatter.** `ProvenanceRecord::signer` is
///   `ev.pubkey.to_hex()`, and `preflight` keeps a record only when the signer
///   equals the channel's creator (§5). Fixing the signer here is what lets
///   that comparison mean anything.
/// - **`created_by` on kind:39000 is the squatter, even after the `owner`
///   grant.** This is the value `channel_owner` reads, and the whole reason
///   adoption moved off roles: `channels.created_by` is written once at
///   creation and no relay path rewrites it, so no role the squatter hands out
///   can change the answer.
///
/// Both are read by the *admin's* connection, from inside the channel they
/// were just given `owner` in — the exact vantage point the attack creates.
/// Their `to_hex()`/`hex::encode` encodings must match byte for byte, since
/// `preflight` compares them with `==`; asserting on the hex strings is what
/// catches an encoding drift that would silently make every comparison false.
///
/// What this does *not* do is call preflight or the saga. That the crate then
/// discards the record is already covered by
/// `provenance_signed_by_a_non_owner_is_ignored` against the fake. What only a
/// live relay can show is that the relay hands back these two answers in the
/// first place, and names the squatter in both.
#[tokio::test]
#[ignore]
async fn squatted_channel_provenance_is_signed_by_the_squatter() {
    let squatter = Keys::generate();
    let admin = Keys::generate();

    // The squatter creates the channel, so the relay writes *their* pubkey
    // into `channels.created_by`. In the real attack this is a derived catalog
    // id; the derivation is irrelevant to what the relay reports, so a fresh
    // uuid stands in and keeps this file free of a `schoolx-catalog` dep.
    let channel_id = create_channel(&squatter, "private").await;

    // A record that looks finished, so an unguarded preflight would read it as
    // "already applied" and adopt the room rather than create one.
    let content = provenance_content(serde_json::json!({
        "channel": "done",
        "canvas": "done",
        "membership": "done"
    }));

    let mut squatter_client = BuzzTestClient::connect(&relay_url(), &squatter)
        .await
        .expect("squatter connect");
    let ok = squatter_client
        .send_event(provenance_event(&squatter, channel_id, &content))
        .await
        .expect("publish provenance");
    assert!(ok.accepted, "relay rejected kind 39500: {}", ok.message);
    squatter_client.disconnect().await.ok();

    // The strong form: hand the victim `owner`. No consent is asked of them,
    // and the relay caps neither the number of owners nor who may be granted
    // one — only the last owner is protected from removal.
    grant_role(&squatter, channel_id, &admin, "owner").await;

    let mut admin_client = BuzzTestClient::connect(&relay_url(), &admin)
        .await
        .expect("admin connect");

    let events = read_provenance(&mut admin_client, "squatted").await;
    assert_eq!(
        events.len(),
        1,
        "admin should see exactly the squatter's record: {events:?}"
    );
    assert_eq!(
        events[0].pubkey.to_hex(),
        squatter.public_key().to_hex(),
        "provenance signer must be the squatter — the admin's preflight has to \
         be able to tell this record is not its own"
    );
    assert_ne!(
        events[0].pubkey.to_hex(),
        admin.public_key().to_hex(),
        "provenance signer must not be attributable to the admin"
    );

    let metadata = read_group_metadata(&mut admin_client, channel_id, "squatted-metadata").await;
    assert_eq!(
        metadata.len(),
        1,
        "expected exactly one relay-signed kind:39000 for {channel_id} \
         (NIP-33 replacement keys on kind+pubkey+d): {metadata:?}"
    );
    assert_eq!(
        first_tag_value(&metadata[0], "created_by").as_deref(),
        Some(squatter.public_key().to_hex().as_str()),
        "created_by must still name the squatter after they granted the admin \
         `owner` — this is the value adoption anchors on, and a role grant must \
         not be able to move it. tags: {:?}",
        metadata[0].tags
    );

    admin_client.disconnect().await.ok();
}
