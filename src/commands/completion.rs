use anyhow::Result;
use clap::{Args, CommandFactory};
use clap_complete::{generate, Shell};

use crate::cli::Cli;

#[derive(Debug, Args)]
#[command(long_about = "\
Print a shell tab-completion script to stdout. Set it up once and your shell \
will auto-complete `jetemail` commands and flags when you press Tab.

Examples:
  zsh:   jetemail completion zsh  > ~/.zsh/completions/_jetemail
         # then in ~/.zshrc:  fpath=(~/.zsh/completions $fpath); autoload -U compinit && compinit
  bash:  jetemail completion bash > /usr/local/etc/bash_completion.d/jetemail
  fish:  jetemail completion fish > ~/.config/fish/completions/jetemail.fish")]
pub struct Cmd {
    /// Target shell (`bash`, `zsh`, `fish`, `powershell`, `elvish`).
    pub shell: Shell,
}

pub fn run(cmd: &Cmd) -> Result<()> {
    let mut clap_cmd = Cli::command();
    let bin = clap_cmd.get_name().to_string();
    generate(cmd.shell, &mut clap_cmd, bin, &mut std::io::stdout());
    Ok(())
}
