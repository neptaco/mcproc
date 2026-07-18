use std::fmt;

/// Supported version management toolchain
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Toolchain {
    /// The name identifier (e.g., "mise", "asdf")
    pub name: &'static str,
    /// The command prefix template
    pub command_template: &'static str,
    /// The display template for logging
    pub display_template: &'static str,
}

impl Toolchain {
    /// mise toolchain
    pub const MISE: Self = Self {
        name: "mise",
        command_template: "mise exec -- sh -c '{cmd}'",
        display_template: "mise exec -- sh -c '{cmd}'",
    };

    /// asdf toolchain
    pub const ASDF: Self = Self {
        name: "asdf",
        command_template: "asdf exec sh -c '{cmd}'",
        display_template: "asdf exec sh -c '{cmd}'",
    };

    /// nvm toolchain
    pub const NVM: Self = Self {
        name: "nvm",
        command_template: "bash -c 'source \"$NVM_DIR/nvm.sh\" && {cmd}'",
        display_template: "nvm (bash) -c '{cmd}'",
    };

    /// rbenv toolchain
    pub const RBENV: Self = Self {
        name: "rbenv",
        command_template: "rbenv exec sh -c '{cmd}'",
        display_template: "rbenv exec sh -c '{cmd}'",
    };

    /// pyenv toolchain
    pub const PYENV: Self = Self {
        name: "pyenv",
        command_template: "pyenv exec sh -c '{cmd}'",
        display_template: "pyenv exec sh -c '{cmd}'",
    };

    /// nodenv toolchain
    pub const NODENV: Self = Self {
        name: "nodenv",
        command_template: "nodenv exec sh -c '{cmd}'",
        display_template: "nodenv exec sh -c '{cmd}'",
    };

    /// jenv toolchain
    pub const JENV: Self = Self {
        name: "jenv",
        command_template: "jenv exec sh -c '{cmd}'",
        display_template: "jenv exec sh -c '{cmd}'",
    };

    /// tfenv toolchain
    pub const TFENV: Self = Self {
        name: "tfenv",
        command_template: "tfenv exec sh -c '{cmd}'",
        display_template: "tfenv exec sh -c '{cmd}'",
    };

    /// goenv toolchain
    pub const GOENV: Self = Self {
        name: "goenv",
        command_template: "goenv exec sh -c '{cmd}'",
        display_template: "goenv exec sh -c '{cmd}'",
    };

    /// rustup toolchain
    pub const RUSTUP: Self = Self {
        name: "rustup",
        command_template: "rustup run stable sh -c '{cmd}'",
        display_template: "rustup run stable sh -c '{cmd}'",
    };

    /// All supported toolchains
    pub const ALL: &'static [Self] = &[
        Self::MISE,
        Self::ASDF,
        Self::NVM,
        Self::RBENV,
        Self::PYENV,
        Self::NODENV,
        Self::JENV,
        Self::TFENV,
        Self::GOENV,
        Self::RUSTUP,
    ];

    /// Get all supported toolchains as a comma-separated string
    pub fn all_supported() -> String {
        Self::ALL
            .iter()
            .map(|t| t.name)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Parse toolchain from string
    pub fn parse(s: &str) -> Option<&'static Self> {
        Self::ALL.iter().find(|t| t.name.eq_ignore_ascii_case(s))
    }

    /// Wrap command with toolchain-specific execution
    pub fn wrap_command(&self, shell_command: &str) -> (String, String) {
        let escaped_cmd = shell_command.replace("'", "'\\''");

        let final_command = self.command_template.replace("{cmd}", &escaped_cmd);
        let display_command = self.display_template.replace("{cmd}", shell_command);

        (final_command, display_command)
    }
}

impl fmt::Display for Toolchain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_command_preserves_shell_expansion_characters_for_inner_shell() {
        let (wrapped, _) = Toolchain::MISE.wrap_command("echo $HOME `whoami` \\path");
        assert_eq!(wrapped, "mise exec -- sh -c 'echo $HOME `whoami` \\path'");
    }

    #[test]
    fn wrap_command_escapes_single_quotes() {
        let (wrapped, _) = Toolchain::MISE.wrap_command("echo don't");
        assert_eq!(wrapped, "mise exec -- sh -c 'echo don'\\''t'");
    }
}
