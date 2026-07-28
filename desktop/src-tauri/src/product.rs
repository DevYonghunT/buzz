//! Product identity for the SchoolX desktop app.
//!
//! SchoolX is a rebranded distribution of Buzz. Two kinds of strings look
//! alike in this codebase and must not be confused:
//!
//! **Product strings** name *this build*. They decide where the app stores
//! data, which URL scheme the OS routes to it, and which keychain entries it
//! owns. Rebranding changes them, and every one of them is a coexistence
//! boundary: if SchoolX and Buzz share one, the two products read and write
//! each other's data. They live here.
//!
//! **Protocol identifiers** name things *on the wire* — Nostr event kinds,
//! NIP names, the `buzz:nostr-identity` audience and `buzz-nostr-identity`
//! protocol string in the identity-binding handshake, relay NIP-11 fields.
//! They are shared vocabulary with relays and other clients. Renaming one
//! breaks interop with every peer that did not rename in lockstep, so they
//! are **not** product strings and do not belong in this module. See
//! [`deep_link::parse_nostr_bind_deep_link`](crate::deep_link) — its
//! `audience`/`protocol` checks stay `buzz`-prefixed on purpose.
//!
//! The rule of thumb: if a *relay* or another client would notice the
//! change, it is protocol. If only this machine would notice, it is product.

/// URL scheme this build registers with the OS and generates links with.
///
/// Registered in `tauri.conf.json` under `plugins.deep-link.desktop.schemes`;
/// [`tauri_config_matches_product_identity`] keeps the two in sync.
///
/// SchoolX deliberately does **not** register `buzz`. Both products claim a
/// scheme at install time, and the OS picks one winner non-deterministically
/// when two apps register the same one — on macOS the last registered bundle
/// usually wins, but nothing guarantees it survives the next `lsregister`
/// rebuild. Registering only `schoolx` keeps deep-link routing decidable when
/// Buzz is installed alongside. The cost is that legacy `buzz://` links do not
/// open SchoolX; [`crate::deep_link::handle_deep_link_url`] rejects them
/// explicitly rather than half-handling them.
pub const DEEP_LINK_SCHEME: &str = "schoolx";

/// `schoolx://` — the scheme with its separator, for prefix checks.
pub const DEEP_LINK_URL_PREFIX: &str = "schoolx://";

/// Release bundle identifier. Decides the platform app-data directory
/// (`~/Library/Application Support/<identifier>` on macOS).
pub const BUNDLE_IDENTIFIER: &str = "io.github.schoolx520.app";

/// Dev bundle identifier, from `tauri.dev.conf.json`.
pub const DEV_BUNDLE_IDENTIFIER: &str = "io.github.schoolx520.app.dev";

/// Agent nest directory name under `$HOME`.
///
/// The nest is **not** derived from [`BUNDLE_IDENTIFIER`] — it is a plain
/// home-directory path, so changing the bundle identifier alone would leave
/// SchoolX and Buzz sharing one nest and one set of agent knowledge files.
pub const NEST_DIR_PROD: &str = ".schoolx";

/// Dev-build agent nest directory name under `$HOME`.
pub const NEST_DIR_DEV: &str = ".schoolx-dev";

/// OS keychain service name holding this build's secrets.
///
/// Like the nest, this is a constant rather than a value derived from the
/// bundle identifier, so it needs its own product-scoped name. It guards the
/// user's Nostr identity and every managed agent key: sharing it with Buzz
/// would mean either product could read and overwrite the other's keys.
pub const KEYRING_SERVICE: &str = "schoolx-desktop";

/// Dev-build keychain service name.
pub const KEYRING_SERVICE_DEV: &str = "schoolx-desktop-dev";

/// Product name shown to the OS: window titles, the macOS bundle, the DMG.
///
/// This is the *filesystem and OS* name and stays ASCII so app paths, DMG
/// volumes, and Windows/Linux packaging stay predictable. The name shown
/// **inside** the UI is a translated string (`app.productName` in the i18n
/// catalog) so Korean users see 스쿨엑스; the two are deliberately separate.
pub const PRODUCT_NAME: &str = "SchoolX";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn load_config(relative: &str) -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
    }

    /// `tauri.conf.json` is a second source of truth for values this module
    /// also declares: Tauri reads the JSON to build the bundle, while Rust
    /// code reads these constants. Nothing in the type system ties them
    /// together, so a rename applied to one and not the other ships an app
    /// whose registered scheme does not match the scheme it generates links
    /// with — and the failure only shows up when a user clicks a link.
    #[test]
    fn tauri_config_matches_product_identity() {
        let config = load_config("tauri.conf.json");

        assert_eq!(
            config["identifier"].as_str(),
            Some(BUNDLE_IDENTIFIER),
            "tauri.conf.json identifier must match product::BUNDLE_IDENTIFIER",
        );
        assert_eq!(
            config["productName"].as_str(),
            Some(PRODUCT_NAME),
            "tauri.conf.json productName must match product::PRODUCT_NAME",
        );

        let schemes = config["plugins"]["deep-link"]["desktop"]["schemes"]
            .as_array()
            .expect("deep-link desktop schemes must be an array");
        let schemes: Vec<&str> = schemes.iter().filter_map(Value::as_str).collect();
        assert_eq!(
            schemes,
            vec![DEEP_LINK_SCHEME],
            "SchoolX registers exactly one scheme; registering `buzz` too would \
             make OS routing non-deterministic when Buzz is installed alongside",
        );
    }

    #[test]
    fn dev_config_matches_dev_identity() {
        let config = load_config("tauri.dev.conf.json");

        assert_eq!(
            config["identifier"].as_str(),
            Some(DEV_BUNDLE_IDENTIFIER),
            "tauri.dev.conf.json identifier must match product::DEV_BUNDLE_IDENTIFIER",
        );
        assert_eq!(
            config["productName"].as_str(),
            Some("SchoolX Dev"),
            "dev builds must be visibly distinct from release builds",
        );
    }

    /// Every coexistence boundary must actually differ from the Buzz value it
    /// replaces. A rename that misses one of these leaves the two products
    /// sharing a directory, a keychain service, or a URL scheme.
    #[test]
    fn product_strings_do_not_collide_with_buzz() {
        let buzz_values = [
            "buzz",
            "xyz.block.buzz.app",     // schoolx:buzz-name-ok
            "xyz.block.buzz.app.dev", // schoolx:buzz-name-ok
            ".buzz",                  // schoolx:buzz-name-ok
            ".buzz-dev",              // schoolx:buzz-name-ok
            "buzz-desktop",           // schoolx:buzz-name-ok
            "buzz-desktop-dev",       // schoolx:buzz-name-ok
        ];
        let schoolx_values = [
            DEEP_LINK_SCHEME,
            BUNDLE_IDENTIFIER,
            DEV_BUNDLE_IDENTIFIER,
            NEST_DIR_PROD,
            NEST_DIR_DEV,
            KEYRING_SERVICE,
            KEYRING_SERVICE_DEV,
        ];

        for value in schoolx_values {
            assert!(
                !buzz_values.contains(&value),
                "product string {value:?} still matches a Buzz value; SchoolX would \
                 share that resource with a co-installed Buzz",
            );
        }
    }

    #[test]
    fn deep_link_prefix_agrees_with_scheme() {
        assert_eq!(DEEP_LINK_URL_PREFIX, format!("{DEEP_LINK_SCHEME}://"));
    }

    /// Dev and release must not share a nest or a keychain service either —
    /// running a dev build must not disturb an installed release build.
    #[test]
    fn dev_and_release_resources_are_distinct() {
        assert_ne!(NEST_DIR_PROD, NEST_DIR_DEV);
        assert_ne!(KEYRING_SERVICE, KEYRING_SERVICE_DEV);
        assert_ne!(BUNDLE_IDENTIFIER, DEV_BUNDLE_IDENTIFIER);
    }
}
