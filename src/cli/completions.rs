//! 自动补全脚本生成 (Bash, Zsh, Fish, PowerShell)

use super::args::Cli;
use clap::CommandFactory;
use clap_complete::{generate, Shell};

pub struct CompletionGenerator;

impl CompletionGenerator {
    pub fn generate(shell: Shell) {
        let mut cmd = Cli::command();
        let bin_name = cmd.get_name().to_string();
        generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
    }
}
