//! Live-relay end-to-end tests for kind:39500 (SchoolX workspace-catalog
//! provenance manifest — `docs/schoolx-2/WORKSPACE_CATALOG.md` §4).
//!
//! Everything upstream of this file — the read-only catalog, deterministic
//! channel ids, the `Provenance` wire type, preflight decisions, the
//! idempotent apply saga, Tauri commands, the settings screen, and the CLI —
//! is built and unit-tested against an in-memory fake
//! (`cargo test -p schoolx-catalog`). A fake cannot prove properties of the
//! *real* relay. Three such properties are exactly what everything upstream
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
