//! The command line: every command and its options, as one `enum` (a new command that isn't handled doesn't compile).

use crate::VERSION;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "sandbox",
    version = VERSION,
    about = "AI Sandbox \u{2014} a desktop for this project, reachable only through an allowlist proxy.",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// Print the docker commands that are run
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    // --- Lifecycle ---
    /// Build (if needed), start and open the desktop in the browser
    #[command(visible_alias = "start")]
    Up {
        /// Don't open the browser
        #[arg(long)]
        no_open: bool,
    },
    /// Stop (everything is kept)
    Stop,
    /// Remove containers (home volume and project are kept)
    Down,
    /// Restart
    Restart,
    /// Container status and URL
    #[command(visible_alias = "ps")]
    Status,
    /// Open the desktop in the browser
    Open,
    /// Bash inside the sandbox (in the project folder)
    Shell,
    /// Claude Code CLI inside the sandbox (needs claude_code under [tools]); all arguments are passed on
    #[command(disable_help_flag = true)]
    Claude {
        /// Arguments for claude
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Follow logs (proxy, gateway, desktop)
    Logs {
        /// proxy | gateway | desktop (default: all)
        service: Vec<String>,
        /// Lines to show from the end
        #[arg(short = 'n', long, default_value_t = 100)]
        tail: i64,
        /// Print and exit instead of following
        #[arg(long)]
        no_follow: bool,
    },

    // --- Network ---
    /// Allow host(s) and reload the proxy
    Allow {
        /// Host name, wildcards like *.example.com allowed
        #[arg(required = true)]
        host: Vec<String>,
    },
    /// Remove host(s) and reload the proxy
    Deny {
        /// Host name, wildcards like *.example.com allowed
        #[arg(required = true)]
        host: Vec<String>,
    },
    /// Show active allowlist entries
    Allowlist {
        /// Also show the commented-out (available) entries
        #[arg(short, long)]
        all: bool,
    },
    /// Show hosts the sandbox tried to reach but were blocked
    Blocked,
    /// Reload the proxy after editing allowlist.txt
    Reload,

    // --- Maintenance ---
    /// Rebuild after changing [tools] in sandbox.toml
    Rebuild,
    /// Rebuild with the newest versions of all tools
    Update,
    /// Delete the persistent home (logins, IDE settings)
    ResetHome {
        /// Don't ask for confirmation
        #[arg(short, long)]
        yes: bool,
    },
    /// Create .sandbox/ (sandbox.toml, allowlist.txt) in a project folder
    Init {
        /// Project folder (default: the current directory)
        folder: Option<String>,
    },
    /// Show the project identity, settings and configured tools
    Info,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("sandbox").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn aliases_and_options() {
        assert!(matches!(
            parse(&["start", "--no-open"]).command,
            Command::Up { no_open: true }
        ));
        assert!(matches!(parse(&["ps"]).command, Command::Status));
        assert!(parse(&["status", "-v"]).verbose);
        assert!(parse(&["-v", "status"]).verbose);
        match parse(&["logs", "proxy", "desktop", "-n", "5", "--no-follow"]).command {
            Command::Logs {
                service,
                tail,
                no_follow,
            } => {
                assert_eq!(
                    (service, tail, no_follow),
                    (vec!["proxy".to_string(), "desktop".to_string()], 5, true)
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            parse(&["reset-home", "-y"]).command,
            Command::ResetHome { yes: true }
        ));
        assert!(matches!(
            parse(&["allowlist", "--all"]).command,
            Command::Allowlist { all: true }
        ));
    }

    #[test]
    fn claude_takes_everything_after_it() {
        match parse(&["-v", "claude", "--resume", "-p", "hi there"]).command {
            Command::Claude { args } => assert_eq!(args, vec!["--resume", "-p", "hi there"]),
            other => panic!("{other:?}"),
        }
        match parse(&["claude", "--help"]).command {
            Command::Claude { args } => assert_eq!(args, vec!["--help"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn hosts_are_required() {
        assert!(Cli::try_parse_from(["sandbox", "allow"]).is_err());
        assert!(Cli::try_parse_from(["sandbox", "deny"]).is_err());
    }
}
