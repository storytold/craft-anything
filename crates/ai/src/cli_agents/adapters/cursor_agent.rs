//! Cursor Agent (`cursor-agent acp`).
//!
//! Grok Build also installs a program named `agent`, so the id and default program are
//! `cursor-agent` (the install provides both names). `command` may point at either, and
//! [`AgentIdentity`] checks `--version` before start.

use super::*;

/// First Cursor Agent date known to start Client-supplied MCP servers on `session/new`.
/// 2026.06.04-5fd875e accepted the list and then ran without those tools.
const ACP_MCP_SINCE: (u16, u8, u8) = (2026, 6, 5);

/// Cursor Agent speaks only ACP; the native prompt runner is not offered.
pub struct CursorAgent;

impl AgentAdapter for CursorAgent {
    fn ids(&self) -> &'static [&'static str] {
        &["cursor-agent"]
    }

    fn args(&self, _cx: &ArgsCx) -> Vec<String> {
        Vec::new()
    }

    fn requires_acp(&self) -> bool {
        true
    }

    fn plain_fallback(&self) -> bool {
        false
    }

    fn acp_options(&self, _options: &BackendOptions, _access: Access) -> Vec<String> {
        Vec::new()
    }

    /// Cursor Agent reads files natively and its ACP permission prompts follow its own approval
    /// settings, so a Clean room run cannot keep the original out of reach (like Aider and Grok).
    fn validate_access(&self, access: Access) -> Result<(), ToolError> {
        match access {
            Access::CleanRoomAnalyse | Access::CleanRoomBuild | Access::CleanRoomQuestion => {
                Err(ToolError::invalid("Cursor Agent reads files on its own, so a Clean room run can't hold it back.")
                    .with_remediation("Use a deep build for Cursor Agent. Its reads aren't checked, so it can't be kept out of the original's files."))
            }
            _ => Ok(()),
        }
    }

    fn acp_mcp_since(&self) -> Option<(u16, u8, u8)> {
        Some(ACP_MCP_SINCE)
    }

    fn identity(&self) -> Option<AgentIdentity> {
        Some(AgentIdentity::Cursor)
    }

    fn validate_options(&self, options: &BackendOptions) -> Result<(), ToolError> {
        if options.args.is_empty() {
            Ok(())
        } else {
            Err(ToolError::invalid("Extra Cursor Agent arguments could skip Craft Anything's approval step.").with_remediation(
                "Remove this backend's extra arguments. Flags such as --force, -f and --yolo let Cursor run shell commands and file writes without asking.",
            ))
        }
    }

    fn sign_in_fix(&self) -> Option<String> {
        Some("Sign in with `cursor-agent login` (or `agent login` for Cursor's CLI), then try again.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_not_plain() {
        assert!(AgentAdapters::built_in().for_agent("cursor-agent").ids().contains(&"cursor-agent"));
        assert!(CursorAgent.requires_acp() && !CursorAgent.plain_fallback());
        assert_eq!(CursorAgent.acp_mcp_since(), Some((2026, 6, 5)));
        assert_eq!(CursorAgent.identity(), Some(AgentIdentity::Cursor));
    }

    #[test]
    fn the_bare_name_agent_is_not_taken_by_cursor() {
        // `agent` alone could be Grok Build; it gets the plain adapter, not Cursor's.
        assert!(!AgentAdapters::built_in().for_agent("agent").ids().contains(&"cursor-agent"));
    }

    #[test]
    fn extra_arguments_that_can_disable_approval_are_refused() {
        assert!(CursorAgent.validate_options(&BackendOptions::default()).is_ok());
        assert!(CursorAgent.acp_options(&BackendOptions::default(), Access::Build).is_empty());
        for argument in ["--force", "-f", "--yolo", "--print", "--sandbox=disabled"] {
            let options = BackendOptions { args: vec![argument.into()], ..Default::default() };
            assert!(CursorAgent.validate_options(&options).is_err(), "{argument}");
            assert!(CursorAgent.acp_options(&options, Access::Build).is_empty(), "{argument} must not reach argv");
        }
    }

    #[test]
    fn clean_room_modes_are_refused_and_deep_modes_allowed() {
        for access in [Access::CleanRoomAnalyse, Access::CleanRoomBuild, Access::CleanRoomQuestion] {
            let err = CursorAgent.validate_access(access).expect_err("a clean room can't hold Cursor Agent back");
            assert_eq!(err.code, ErrorCode::BadInput, "{access:?}");
        }
        for access in [Access::Analyse, Access::Build, Access::Question] {
            assert!(CursorAgent.validate_access(access).is_ok(), "{access:?}");
        }
    }
}
