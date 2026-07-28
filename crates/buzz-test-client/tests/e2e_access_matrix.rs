//! End-to-end access matrix for the SchoolX managed-agent membership rule.
//!
//! Buzz's stock semantics: an `open` channel is readable and writable by any
//! authenticated member of the community, whether or not they joined it.
//! SchoolX narrows that for one principal class only — a NIP-OA attested
//! managed agent must hold active channel membership even on an `open`
//! channel. Humans keep the stock behaviour.
//!
//! The rule is only worth anything if it holds on *every* path a principal can
//! reach, because the ACP subscription allowlist is an automation scope, not a
//! security boundary: the same credential can always open a raw WebSocket or
//! call the HTTP bridge directly. These tests therefore drive the matrix
//! across WS REQ / COUNT / EVENT and HTTP `/query` / `/count` / `/events`.
//!
//! The relay-side gate is `PrincipalClass::requires_explicit_channel_membership`
//! (`crates/buzz-relay/src/handlers/ingest.rs`), consumed on the read paths by
//! the `get_member_channel_ids` lookup and on the write path by
//! `check_channel_membership`.
//!
//! # Running
//!
//! ```text
//! just relay                       # in another shell
//! RELAY_URL=ws://localhost:3000 cargo test -p buzz-test-client \
//!     --test e2e_access_matrix -- --ignored --test-threads=1
//! ```

use std::time::Duration;

use buzz_sdk::nip_oa;
use buzz_test_client::{BuzzTestClient, RelayMessage, TestClientError};
use nostr::{EventBuilder, Filter, Keys, Kind, Tag};

const STREAM_MESSAGE_KIND: u16 = 9;
const CREATE_GROUP_KIND: u16 = 9007;
const PUT_USER_KIND: u16 = 9000;
const JOIN_REQUEST_KIND: u16 = 9021;

/// Long enough for the relay to fan out, short enough that a suite of
/// "nothing must arrive" assertions stays quick.
const DELIVERY_WAIT: Duration = Duration::from_secs(3);
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
    format!("e2e-access-{name}-{}", uuid::Uuid::new_v4())
}

// ─── fixtures ──────────────────────────────────────────────────────────────

/// Create a channel with the given visibility, owned by `keys`.
///
/// Submitted over the HTTP bridge with the dev `X-Pubkey` header, matching the
/// pattern the other live e2e suites use.
async fn create_channel(keys: &Keys, visibility: &str) -> String {
    let channel_uuid = uuid::Uuid::new_v4();
    let event = EventBuilder::new(Kind::Custom(CREATE_GROUP_KIND), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
            Tag::parse(["name", &format!("access-matrix-{channel_uuid}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", visibility]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();

    let body = post_event_as(keys, &event, None).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "channel creation not accepted ({visibility}): {body}"
    );
    channel_uuid.to_string()
}

/// Add `target` to `channel_id` as a member, signed by the channel owner.
async fn add_member(owner: &Keys, channel_id: &str, target: &Keys) {
    let event = EventBuilder::new(Kind::Custom(PUT_USER_KIND), "")
        .tags(vec![
            Tag::parse(["h", channel_id]).unwrap(),
            Tag::parse(["p", &target.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(owner)
        .unwrap();

    let body = post_event_as(owner, &event, None).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "add_member not accepted: {body}"
    );
}

/// POST a signed event to the bridge, optionally presenting a NIP-OA auth tag.
///
/// Returns the parsed JSON body. The caller asserts on `accepted` — a rejected
/// event is still an HTTP 200 with `"accepted": false`, so status alone proves
/// nothing.
async fn post_event_as(
    keys: &Keys,
    event: &nostr::Event,
    auth_tag_json: Option<&str>,
) -> serde_json::Value {
    let client = reqwest::Client::new();
    let mut req = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json");
    if let Some(tag) = auth_tag_json {
        req = req.header("x-auth-tag", tag);
    }
    let resp = req
        .body(serde_json::to_string(event).unwrap())
        .send()
        .await
        .expect("POST /events");
    resp.json().await.expect("parse /events response")
}

/// POST a filter array to `/query` or `/count`, optionally as a managed agent.
async fn post_bridge(
    path: &str,
    keys: &Keys,
    filters: serde_json::Value,
    auth_tag_json: Option<&str>,
) -> (reqwest::StatusCode, serde_json::Value) {
    let client = reqwest::Client::new();
    let mut req = client
        .post(format!("{}{path}", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json");
    if let Some(tag) = auth_tag_json {
        req = req.header("x-auth-tag", tag);
    }
    let resp = req
        .body(filters.to_string())
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {path} failed: {e}"));
    let status = resp.status();
    let body = resp.json().await.unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// The NIP-OA tag JSON string proving `owner` authorized `agent`.
fn auth_tag_json(owner: &Keys, agent: &Keys) -> String {
    nip_oa::compute_auth_tag(owner, &agent.public_key(), "").expect("compute_auth_tag")
}

/// Connect `agent` as a NIP-OA managed agent owned by `owner`.
async fn connect_agent(agent: &Keys, owner: &Keys) -> BuzzTestClient {
    let tag = nip_oa::parse_auth_tag(&auth_tag_json(owner, agent)).expect("parse_auth_tag");
    let mut client = BuzzTestClient::connect_unauthenticated(&relay_url())
        .await
        .expect("agent connect");
    client
        .authenticate_with_nip_oa(agent, &tag)
        .await
        .expect("agent NIP-OA auth");
    client
}

/// A filter scoped to one channel's stream messages.
fn channel_filter(channel_id: &str) -> Filter {
    Filter::new()
        .kind(Kind::Custom(STREAM_MESSAGE_KIND))
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
            channel_id,
        )
}

fn channel_filter_json(channel_id: &str) -> serde_json::Value {
    serde_json::json!([{ "kinds": [STREAM_MESSAGE_KIND], "#h": [channel_id] }])
}

/// Seed one message into `channel_id`, authored by a member, so that "reads
/// return nothing" is a real assertion rather than an empty-channel artifact.
async fn seed_message(author: &Keys, channel_id: &str) -> String {
    let content = format!("seed-{}", uuid::Uuid::new_v4());
    let mut client = BuzzTestClient::connect(&relay_url(), author)
        .await
        .expect("seed author connect");
    let ok = client
        .send_text_message(author, channel_id, &content, STREAM_MESSAGE_KIND)
        .await
        .expect("seed publish");
    assert!(ok.accepted, "seed message rejected: {}", ok.message);
    client.disconnect().await.ok();
    content
}

/// Outcome of a WS REQ, distinguishing "allowed but empty" from "refused".
///
/// `BuzzTestClient::collect_until_eose` cannot express this: it drops every
/// message that is not an EVENT or EOSE for its subscription, so a relay
/// CLOSED arrives, is discarded, and the call sits until it times out. A
/// refusal and a hung relay look identical through it. This helper drives
/// `recv_event` directly so a refusal is a first-class result.
#[derive(Debug)]
struct ReadOutcome {
    /// Contents of events actually delivered.
    contents: Vec<String>,
    /// Set when the relay refused the subscription, carrying its reason.
    closed: Option<String>,
    /// Set when the relay neither completed nor refused within the deadline.
    timed_out: bool,
}

impl ReadOutcome {
    fn saw(&self, content: &str) -> bool {
        self.contents.iter().any(|c| c == content)
    }

    /// Assert the read was refused, not merely empty. An empty allowed read
    /// would also hide the content, but for the wrong reason — it would mean
    /// the filter missed, not that the gate held.
    fn assert_refused(&self, what: &str) {
        assert!(
            self.closed.is_some() || self.timed_out,
            "{what}: relay completed the read instead of refusing it \
             (delivered {} event(s): {:?})",
            self.contents.len(),
            self.contents
        );
    }
}

/// Read a channel over WS REQ.
async fn ws_read(client: &mut BuzzTestClient, channel_id: &str) -> ReadOutcome {
    let sid = sub_id("read");
    client
        .subscribe(&sid, vec![channel_filter(channel_id)])
        .await
        .expect("subscribe");

    let mut contents = Vec::new();
    let mut closed = None;
    let mut timed_out = false;

    loop {
        match client.recv_event(EOSE_WAIT).await {
            Ok(RelayMessage::Event {
                subscription_id,
                event,
            }) if subscription_id == sid => contents.push(event.content.clone()),
            Ok(RelayMessage::Eose { subscription_id }) if subscription_id == sid => break,
            Ok(RelayMessage::Closed {
                subscription_id,
                message,
            }) if subscription_id == sid => {
                closed = Some(message);
                break;
            }
            // Frames for other subscriptions, notices, auth — keep waiting.
            Ok(_) => {}
            Err(TestClientError::Timeout) => {
                timed_out = true;
                break;
            }
            Err(e) => panic!("unexpected transport error during read: {e}"),
        }
    }

    client.close_subscription(&sid).await.ok();
    ReadOutcome {
        contents,
        closed,
        timed_out,
    }
}

// ─── baseline: humans keep Buzz's stock open-channel semantics ─────────────

/// A human who never joined an open channel can still read it.
///
/// This is the control. If SchoolX's agent rule ever widens to humans, this
/// test fails and tells us the product changed, not just the agents.
#[tokio::test]
#[ignore]
async fn human_nonmember_reads_open_channel() {
    let owner = Keys::generate();
    let stranger = Keys::generate();
    let channel = create_channel(&owner, "open").await;
    let seeded = seed_message(&owner, &channel).await;

    let mut client = BuzzTestClient::connect(&relay_url(), &stranger)
        .await
        .expect("human connect");
    let outcome = ws_read(&mut client, &channel).await;

    assert!(
        outcome.saw(&seeded),
        "human non-member must read an open channel (stock Buzz semantics), got {outcome:?}"
    );
    client.disconnect().await.ok();
}

/// A human who never joined a private channel learns nothing from it.
#[tokio::test]
#[ignore]
async fn human_nonmember_cannot_read_private_channel() {
    let owner = Keys::generate();
    let stranger = Keys::generate();
    let channel = create_channel(&owner, "private").await;
    let seeded = seed_message(&owner, &channel).await;

    let mut client = BuzzTestClient::connect(&relay_url(), &stranger)
        .await
        .expect("human connect");
    let outcome = ws_read(&mut client, &channel).await;

    assert!(
        !outcome.saw(&seeded),
        "private channel content leaked to a human non-member: {outcome:?}"
    );
    outcome.assert_refused("human non-member reading a private channel");
    client.disconnect().await.ok();
}

// ─── the SchoolX rule: agents need membership even on open channels ────────

/// WS REQ: a managed agent that is not a member reads nothing from an open
/// channel, while the same channel is readable by a human stranger.
#[tokio::test]
#[ignore]
async fn agent_nonmember_cannot_read_open_channel_over_ws() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let channel_owner = Keys::generate();
    let channel = create_channel(&channel_owner, "open").await;
    let seeded = seed_message(&channel_owner, &channel).await;

    let mut agent_client = connect_agent(&agent, &owner).await;
    let outcome = ws_read(&mut agent_client, &channel).await;

    assert!(
        !outcome.saw(&seeded),
        "managed agent read an open channel it never joined: {outcome:?}"
    );
    outcome.assert_refused("non-member agent reading an open channel");
    agent_client.disconnect().await.ok();
}

/// HTTP bridge `/query`: same rule, different transport.
#[tokio::test]
#[ignore]
async fn agent_nonmember_cannot_read_open_channel_over_http_query() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let channel_owner = Keys::generate();
    let channel = create_channel(&channel_owner, "open").await;
    let seeded = seed_message(&channel_owner, &channel).await;

    let tag = auth_tag_json(&owner, &agent);
    let (status, body) =
        post_bridge("/query", &agent, channel_filter_json(&channel), Some(&tag)).await;

    let leaked = body
        .as_array()
        .map(|events| {
            events
                .iter()
                .any(|e| e["content"].as_str() == Some(seeded.as_str()))
        })
        .unwrap_or(false);
    assert!(
        !leaked,
        "managed agent read an open channel over /query (status {status}): {body}"
    );
}

/// HTTP bridge `/count`: a count is a read. Cardinality leaks too.
#[tokio::test]
#[ignore]
async fn agent_nonmember_cannot_count_open_channel_over_http() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let channel_owner = Keys::generate();
    let channel = create_channel(&channel_owner, "open").await;
    seed_message(&channel_owner, &channel).await;

    let tag = auth_tag_json(&owner, &agent);
    let (status, body) =
        post_bridge("/count", &agent, channel_filter_json(&channel), Some(&tag)).await;

    let count = body["count"].as_u64().unwrap_or(0);
    assert_eq!(
        count, 0,
        "managed agent counted messages in an open channel it never joined \
         (status {status}): {body}"
    );
}

/// WS EVENT: a managed agent that is not a member cannot write to an open
/// channel. Read-side gating alone would leave the room writable by anyone's
/// agent.
#[tokio::test]
#[ignore]
async fn agent_nonmember_cannot_write_open_channel_over_ws() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let channel_owner = Keys::generate();
    let channel = create_channel(&channel_owner, "open").await;

    let mut agent_client = connect_agent(&agent, &owner).await;
    let result = agent_client
        .send_text_message(
            &agent,
            &channel,
            "agent write into a channel it never joined",
            STREAM_MESSAGE_KIND,
        )
        .await;

    match result {
        Ok(ok) => assert!(
            !ok.accepted,
            "managed agent wrote to an open channel it never joined: {}",
            ok.message
        ),
        Err(TestClientError::EventRejected(_)) => {}
        Err(e) => panic!("unexpected transport error: {e}"),
    }
    agent_client.disconnect().await.ok();
}

/// HTTP bridge `/events`: the write rule holds over HTTP too.
#[tokio::test]
#[ignore]
async fn agent_nonmember_cannot_write_open_channel_over_http() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let channel_owner = Keys::generate();
    let channel = create_channel(&channel_owner, "open").await;

    let event = EventBuilder::new(Kind::Custom(STREAM_MESSAGE_KIND), "http agent write")
        .tags(vec![Tag::parse(["h", &channel]).unwrap()])
        .sign_with_keys(&agent)
        .unwrap();

    let tag = auth_tag_json(&owner, &agent);
    let body = post_event_as(&agent, &event, Some(&tag)).await;

    assert!(
        !body["accepted"].as_bool().unwrap_or(false),
        "managed agent wrote to an open channel over /events: {body}"
    );
}

/// A managed agent cannot let itself in. kind:9021 self-join normally bypasses
/// the generic membership gate; for a restricted principal it must not.
#[tokio::test]
#[ignore]
async fn agent_cannot_self_join_open_channel() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let channel_owner = Keys::generate();
    let channel = create_channel(&channel_owner, "open").await;

    let event = EventBuilder::new(Kind::Custom(JOIN_REQUEST_KIND), "")
        .tags(vec![Tag::parse(["h", &channel]).unwrap()])
        .sign_with_keys(&agent)
        .unwrap();

    let tag = auth_tag_json(&owner, &agent);
    let body = post_event_as(&agent, &event, Some(&tag)).await;

    assert!(
        !body["accepted"].as_bool().unwrap_or(false),
        "managed agent self-joined an open channel via kind:9021: {body}"
    );
}

// ─── the rule is not over-broad ────────────────────────────────────────────

/// A managed agent that IS a member reads and writes normally. Without this,
/// a relay that simply denied every agent everything would pass the suite.
#[tokio::test]
#[ignore]
async fn agent_member_can_read_and_write() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let channel_owner = Keys::generate();
    let channel = create_channel(&channel_owner, "open").await;
    let seeded = seed_message(&channel_owner, &channel).await;

    add_member(&channel_owner, &channel, &agent).await;

    let mut agent_client = connect_agent(&agent, &owner).await;
    let outcome = ws_read(&mut agent_client, &channel).await;
    assert!(
        outcome.saw(&seeded),
        "member agent must read its own channel, got {outcome:?}"
    );

    let ok = agent_client
        .send_text_message(&agent, &channel, "member agent write", STREAM_MESSAGE_KIND)
        .await
        .expect("member agent publish");
    assert!(
        ok.accepted,
        "member agent must be able to write: {}",
        ok.message
    );
    agent_client.disconnect().await.ok();
}

// ─── the classification asymmetry ──────────────────────────────────────────

/// Dropping the NIP-OA tag must not restore human semantics.
///
/// The per-connection `AuthContext.agent_owner_pubkey` is set only from a tag
/// presented on that connection, but the community-scoped
/// `users.agent_owner_pubkey` column is first-write-wins and permanent. An
/// agent that authenticates once with a tag and then reconnects with plain
/// NIP-42 must still be treated as an agent — otherwise the restriction is
/// opt-in by the very principal it restricts.
#[tokio::test]
#[ignore]
async fn agent_cannot_shed_its_class_by_dropping_the_tag() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let channel_owner = Keys::generate();
    let channel = create_channel(&channel_owner, "open").await;
    let seeded = seed_message(&channel_owner, &channel).await;

    // First connection establishes the persisted owner mapping.
    let first = connect_agent(&agent, &owner).await;
    first.disconnect().await.ok();

    // Second connection presents no auth tag at all.
    let mut plain = BuzzTestClient::connect(&relay_url(), &agent)
        .await
        .expect("agent reconnect without tag");
    let outcome = ws_read(&mut plain, &channel).await;

    assert!(
        !outcome.saw(&seeded),
        "agent regained open-channel access by omitting its NIP-OA tag: {outcome:?}"
    );
    outcome.assert_refused("agent reconnecting without its NIP-OA tag");
    plain.disconnect().await.ok();
}

// ─── live fan-out ──────────────────────────────────────────────────────────

/// A non-member agent holding an open subscription receives nothing when a
/// member posts. Stored-event gating is not enough; the push path is separate.
#[tokio::test]
#[ignore]
async fn agent_nonmember_receives_no_live_fanout() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let channel_owner = Keys::generate();
    let channel = create_channel(&channel_owner, "open").await;

    let mut agent_client = connect_agent(&agent, &owner).await;
    let sid = sub_id("live");
    agent_client
        .subscribe(&sid, vec![channel_filter(&channel)])
        .await
        .expect("agent subscribe");
    // Drain history so anything seen later is genuinely live.
    let _ = agent_client.collect_until_eose(&sid, EOSE_WAIT).await;

    let content = seed_message(&channel_owner, &channel).await;

    // Timeout is the pass condition here.
    match agent_client.recv_event(DELIVERY_WAIT).await {
        Err(TestClientError::Timeout) => {}
        Ok(RelayMessage::Event { event, .. }) => {
            assert_ne!(
                event.content, content,
                "live fan-out delivered a message to a non-member agent"
            );
        }
        Ok(_) => {}
        Err(e) => panic!("unexpected transport error: {e}"),
    }
    agent_client.disconnect().await.ok();
}
