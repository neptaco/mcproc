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
    /// Quote style for shell command: true for double quotes, false for single quotes
    pub use_double_quotes: bool,
}

impl Toolchain {
    /// mise toolchain
    pub const MISE: Self = Self {
        name: "mise",
        command_template: "mise exec -- sh -c \"{cmd}\"",
        display_template: "mise exec -- sh -c '{cmd}'",
        use_double_quotes: true,
    };

    /// asdf toolchain
    pub const ASDF: Self = Self {
        name: "asdf",
        command_template: "asdf exec sh -c \"{cmd}\"",
        display_template: "asdf exec sh -c '{cmd}'",
        use_double_quotes: true,
    };

    /// nvm toolchain
    pub const NVM: Self = Self {
        name: "nvm",
        command_template: "bash -c 'source \"$NVM_DIR/nvm.sh\" && {cmd}'",
        display_template: "nvm (bash) -c '{cmd}'",
        use_double_quotes: false,
    };

    /// rbenv toolchain
    pub const RBENV: Self = Self {
        name: "rbenv",
        command_template: "rbenv exec sh -c \"{cmd}\"",
        display_template: "rbenv exec sh -c '{cmd}'",
        use_double_quotes: true,
    };

    /// pyenv toolchain
    pub const PYENV: Self = Self {
        name: "pyenv",
        command_template: "pyenv exec sh -c \"{cmd}\"",
        display_template: "pyenv exec sh -c '{cmd}'",
        use_double_quotes: true,
    };

    /// nodenv toolchain
    pub const NODENV: Self = Self {
        name: "nodenv",
        command_template: "nodenv exec sh -c \"{cmd}\"",
        display_template: "nodenv exec sh -c '{cmd}'",
        use_double_quotes: true,
    };

    /// jenv toolchain
    pub const JENV: Self = Self {
        name: "jenv",
        command_template: "jenv exec sh -c \"{cmd}\"",
        display_template: "jenv exec sh -c '{cmd}'",
        use_double_quotes: true,
    };

    /// tfenv toolchain
    pub const TFENV: Self = Self {
        name: "tfenv",
        command_template: "tfenv exec sh -c \"{cmd}\"",
        display_template: "tfenv exec sh -c '{cmd}'",
        use_double_quotes: true,
    };

    /// goenv toolchain
    pub const GOENV: Self = Self {
        name: "goenv",
        command_template: "goenv exec sh -c \"{cmd}\"",
        display_template: "goenv exec sh -c '{cmd}'",
        use_double_quotes: true,
    };

    /// rustup toolchain
    pub const RUSTUP: Self = Self {
        name: "rustup",
        command_template: "rustup run stable sh -c \"{cmd}\"",
        display_template: "rustup run stable sh -c '{cmd}'",
        use_double_quotes: true,
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
        let escaped_cmd = if self.use_double_quotes {
            shell_command.replace("\"", "\\\"")
        } else {
            shell_command.replace("'", "'\\''")
        };

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
