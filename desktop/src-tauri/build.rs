// Shared schema, included from the same source the runtime command parses with,
// so the build-time validation below and the runtime parse cannot drift.
include!("src/commands/reconnect_hook_config.rs");
// Same source of truth the runtime filters with, so a baked build env cannot
// carry a reserved key the runtime believes it already rejected.
include!("src/managed_agents/reserved_env_keys.rs");

use base64::Engine as _;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=BUZZ_RELAY_URL");
    println!("cargo:rerun-if-env-changed=BUZZ_RELAY_HTTP");
    println!("cargo:rerun-if-env-changed=BUZZ_UPDATER_PUBLIC_KEY");
    println!("cargo:rerun-if-env-changed=BUZZ_UPDATER_ENDPOINT");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_BUZZ_AGENT_PROVIDER");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_BUZZ_AGENT_MODEL");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_AGENT_ENV");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_RELAY_RECONNECT_CMD");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_AGENT_ACCESS_OWNER_ONLY");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_AUTO_CONNECT_DEFAULT_RELAY");
    println!("cargo:rustc-check-cfg=cfg(buzz_updater_enabled)");

    // Explicit owner-only agent-access capability. Release packaging sets this
    // presence-only marker; OSS/custom builds leave agent access configurable.
    if std::env::var("BUZZ_BUILD_AGENT_ACCESS_OWNER_ONLY").is_ok() {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_AGENT_ACCESS_OWNER_ONLY=1");
    }

    if let Ok(relay_url) = std::env::var("BUZZ_RELAY_URL") {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_RELAY_URL={relay_url}");
    }

    if let Ok(relay_http) = std::env::var("BUZZ_RELAY_HTTP") {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_RELAY_HTTP={relay_http}");
    }

    if let Ok(provider) = std::env::var("BUZZ_BUILD_BUZZ_AGENT_PROVIDER") {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_BUZZ_AGENT_PROVIDER={provider}");
    }

    if let Ok(model) = std::env::var("BUZZ_BUILD_BUZZ_AGENT_MODEL") {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_BUZZ_AGENT_MODEL={model}");
    }

    // Generic KEY=VALUE pairs to inject into every spawned agent process.
    // Newline-delimited; each line must be non-empty and contain exactly one
    // `=` separator with a non-empty key.  OSS builds leave this unset.
    // The validated value is base64-encoded before emitting so the single-line
    // Cargo build-script output carries all pairs (Cargo output is line-oriented;
    // a raw multiline value would be silently truncated to the first line).
    if let Ok(raw) = std::env::var("BUZZ_BUILD_AGENT_ENV") {
        for (line_no, line) in raw.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let eq = line.find('=').unwrap_or_else(|| {
                panic!(
                    "BUZZ_BUILD_AGENT_ENV line {}: missing '=' separator in {:?}",
                    line_no + 1,
                    line
                )
            });
            let key = &line[..eq];
            if key.is_empty() {
                panic!(
                    "BUZZ_BUILD_AGENT_ENV line {}: key must not be empty in {:?}",
                    line_no + 1,
                    line
                );
            }
            // The baked env is written into every spawned agent's environment
            // LAST (see `managed_agents/runtime.rs`), after Buzz sets the
            // access gates and identity vars. A baked reserved key would
            // therefore silently override the gate the UI promises, so reject
            // it at build time instead of shipping a binary that bypasses its
            // own enforcement.
            if is_reserved_env_key(key) {
                panic!(
                    "BUZZ_BUILD_AGENT_ENV line {}: `{}` is reserved by Buzz and cannot be baked \
                     into a build (it would override Buzz's own identity/access env)",
                    line_no + 1,
                    key
                );
            }
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_AGENT_ENV={encoded}");
    }

    if let Ok(val) = std::env::var("BUZZ_BUILD_RELAY_RECONNECT_CMD") {
        let parsed: serde_json::Value = serde_json::from_str(&val)
            .unwrap_or_else(|e| panic!("BUZZ_BUILD_RELAY_RECONNECT_CMD is not valid JSON: {e}"));
        serde_json::from_value::<ReconnectHookConfig>(parsed).unwrap_or_else(|e| {
            panic!("BUZZ_BUILD_RELAY_RECONNECT_CMD doesn't match ReconnectHookConfig: {e}")
        });
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_RELAY_RECONNECT_CMD={val}");
    }

    // Presence-only release capability: internal desktop builds opt into
    // auto-connecting their configured default relay on first run. OSS builds
    // leave this unset and retain explicit community selection.
    if std::env::var("BUZZ_BUILD_AUTO_CONNECT_DEFAULT_RELAY").is_ok() {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_AUTO_CONNECT_DEFAULT_RELAY=1");
    }

    let updater_public_key = std::env::var("BUZZ_UPDATER_PUBLIC_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let updater_endpoint = std::env::var("BUZZ_UPDATER_ENDPOINT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if updater_public_key.is_some() && updater_endpoint.is_some() {
        println!("cargo:rustc-cfg=buzz_updater_enabled");
    }

    // Cargo test executables get no embedded Windows manifest (tauri_build
    // attaches one to bin targets only), so the loader binds comctl32 v5, which
    // lacks TaskDialogIndirect (statically imported via tauri-plugin-dialog/rfd)
    // and debug test exes die at load with STATUS_ENTRYPOINT_NOT_FOUND. Declaring
    // the Common Controls v6 dependency makes link.exe emit a side-by-side
    // <exe>.manifest that the loader honors for manifest-less executables;
    // binaries with an embedded manifest (the real app) ignore it.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        build_macos_git_xpc_bridge();
    }

    tauri_build::try_build(
        tauri_build::Attributes::new().plugin(
            "websocket",
            tauri_build::InlinedPlugin::new()
                .commands(&["connect", "send", "disconnect", "disconnect_all"])
                .default_permission(tauri_build::DefaultPermissionRule::AllowAllCommands),
        ),
    )
    .expect("failed to build Tauri application");
}

fn build_macos_git_xpc_bridge() {
    const BRIDGE_SOURCE: &str = "src/code_workspace/macos_git_xpc.rs";
    const SWIFT_SOURCES: &[&str] = &[
        "macos/SchoolXGitXpc.swift",
        "macos/SchoolXGitXpcSession.swift",
        "macos/SchoolXGitXpcMessages.swift",
        "macos/SchoolXGitXpcService.swift",
        "macos/SchoolXGitXpcLifecycle.swift",
        "macos/SchoolXGitXpcSupport.swift",
    ];
    const LIBRARY_NAME: &str = "schoolx_git_xpc";

    println!("cargo:rerun-if-changed={BRIDGE_SOURCE}");
    for source in SWIFT_SOURCES {
        println!("cargo:rerun-if-changed={source}");
    }
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");

    let out_dir = required_path_env("OUT_DIR");
    let target = required_env("TARGET");
    let swift_target = match target.as_str() {
        "aarch64-apple-darwin" => "arm64-apple-macosx11.0",
        "x86_64-apple-darwin" => "x86_64-apple-macosx10.15",
        other => panic!("unsupported macOS XPC bridge target {other}"),
    };
    let generated = out_dir.join("schoolx-git-xpc-generated");
    swift_bridge_build::parse_bridges([BRIDGE_SOURCE])
        .write_all_concatenated(&generated, LIBRARY_NAME);

    let bridge_header = out_dir.join("schoolx-git-xpc-bridge.h");
    let header = format!(
        "#include \"{}\"\n#include \"{}\"\n",
        generated.join("SwiftBridgeCore.h").display(),
        generated
            .join(LIBRARY_NAME)
            .join(format!("{LIBRARY_NAME}.h"))
            .display()
    );
    std::fs::write(&bridge_header, header)
        .unwrap_or_else(|error| panic!("failed to write macOS XPC bridge header: {error}"));

    let sdk = command_stdout(
        Command::new("xcrun").args(["--sdk", "macosx", "--show-sdk-path"]),
        "resolve the macOS SDK",
    );
    let sdk = sdk.trim();
    if sdk.is_empty() {
        panic!("xcrun returned an empty macOS SDK path");
    }

    let target_info = command_stdout(
        Command::new("xcrun").args([
            "--sdk",
            "macosx",
            "swiftc",
            "-print-target-info",
            "-target",
            swift_target,
            "-sdk",
            sdk,
        ]),
        "read Swift target information",
    );
    let target_info: serde_json::Value = serde_json::from_str(&target_info)
        .unwrap_or_else(|error| panic!("invalid swiftc target information: {error}"));
    let runtime_paths = target_info
        .pointer("/paths/runtimeLibraryPaths")
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| panic!("swiftc target information omitted runtimeLibraryPaths"));

    let library = out_dir.join(format!("lib{LIBRARY_NAME}.a"));
    let mut swiftc = Command::new("xcrun");
    swiftc
        .args([
            "--sdk",
            "macosx",
            "swiftc",
            "-emit-library",
            "-static",
            "-parse-as-library",
            "-target",
            swift_target,
            "-sdk",
            sdk,
            "-module-name",
            "SchoolXGitXpc",
            "-import-objc-header",
        ])
        .arg(&bridge_header)
        .args(SWIFT_SOURCES)
        .arg(
            generated
                .join(LIBRARY_NAME)
                .join(format!("{LIBRARY_NAME}.swift")),
        )
        .arg(generated.join("SwiftBridgeCore.swift"))
        .arg("-o")
        .arg(&library);
    if std::env::var("PROFILE").as_deref() == Ok("release") {
        swiftc.arg("-O");
    }
    let status = swiftc
        .status()
        .unwrap_or_else(|error| panic!("failed to start swiftc for macOS XPC bridge: {error}"));
    if !status.success() {
        panic!("swiftc failed while building the macOS XPC bridge");
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    for path in runtime_paths {
        let path = path
            .as_str()
            .unwrap_or_else(|| panic!("swiftc returned a non-string runtime library path"));
        println!("cargo:rustc-link-search=native={path}");
    }
    println!("cargo:rustc-link-lib=static={LIBRARY_NAME}");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=Security");
}

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|error| panic!("missing build environment {name}: {error}"))
}

fn required_path_env(name: &str) -> PathBuf {
    Path::new(&required_env(name)).to_path_buf()
}

fn command_stdout(command: &mut Command, label: &str) -> String {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("failed to {label}: {error}"));
    if !output.status.success() {
        panic!(
            "failed to {label}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("{label} returned non-UTF-8 output: {error}"))
}
