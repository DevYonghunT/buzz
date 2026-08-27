// SchoolX Code freezes the Codex app-server wire contract to audited minor
// versions. Keep the standalone auto-installer on an exact compatible patch:
// the vendor installers otherwise resolve `latest`, which can cross that
// contract boundary before SchoolX has audited the new schema.
macro_rules! define_codex_cli_install_contract {
    ($version:literal) => {
        #[cfg(test)]
        pub(super) const SCHOOLX_CODE_CODEX_INSTALL_VERSION: &str = $version;
        pub(super) const CODEX_CLI_INSTALL_COMMAND_UNIX: &str = concat!(
            "curl -fsSL https://chatgpt.com/codex/install.sh | sh -s -- --release ",
            $version
        );
        pub(super) const CODEX_CLI_INSTALL_COMMAND_WINDOWS: &str = concat!(
            r#"powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$env:CODEX_RELEASE='"#,
            $version,
            r#"'; irm https://chatgpt.com/codex/install.ps1 | iex""#
        );
    };
}

define_codex_cli_install_contract!("0.149.1");

#[cfg(test)]
mod tests {
    use super::{
        CODEX_CLI_INSTALL_COMMAND_UNIX, CODEX_CLI_INSTALL_COMMAND_WINDOWS,
        SCHOOLX_CODE_CODEX_INSTALL_VERSION,
    };

    #[test]
    fn codex_cli_install_commands_pin_a_schoolx_code_compatible_release() {
        assert_eq!(SCHOOLX_CODE_CODEX_INSTALL_VERSION, "0.149.1");
        let codex = super::super::known_acp_runtime_exact("codex").unwrap();
        assert_eq!(
            codex.cli_install_commands,
            &[CODEX_CLI_INSTALL_COMMAND_UNIX]
        );
        assert_eq!(
            codex.cli_install_commands_windows,
            &[CODEX_CLI_INSTALL_COMMAND_WINDOWS]
        );

        for command in codex
            .cli_install_commands
            .iter()
            .chain(codex.cli_install_commands_windows.iter())
        {
            assert!(
                command.contains(SCHOOLX_CODE_CODEX_INSTALL_VERSION),
                "Codex install command must request the pinned release; got: {command}"
            );
        }

        let probe = crate::code_workspace::CodeRuntimeProbe {
            available: true,
            executable: Some("codex".to_string()),
            version: Some(format!("codex-cli {SCHOOLX_CODE_CODEX_INSTALL_VERSION}")),
            error: None,
        };
        crate::code_workspace::ensure_supported_codex_version(&probe)
            .expect("the auto-install pin must satisfy the SchoolX Code compatibility gate");
    }
}
