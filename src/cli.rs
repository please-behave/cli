use clap::{Parser, Subcommand};

#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum ProgressMode {
    #[default]
    Auto,
    Plain,
    Tty,
}

#[derive(Debug, Parser)]
#[command(
    name = "please",
    version,
    before_help = include_str!("banner.txt").trim_end(),
    about = "Enforce development rules across coding agents and workflows.",
    disable_help_subcommand = true,
    help_template = include_str!("help.txt")
)]
pub struct Cli {
    #[arg(long, global = true, help = "Disable colors")]
    pub no_color: bool,
    #[command(subcommand)]
    pub command: Words,
}
#[derive(Debug, Subcommand)]
pub enum Words {
    /// Initialize a new project configuration once.
    Init,
    /// Validate configuration, refresh imports and synchronize hooks.
    Sync {
        #[arg(long, help = "Approve planned integration changes without prompting")]
        yes: bool,
    },
    /// Execute selected rules and save results.
    Run {
        #[arg(
            long,
            value_enum,
            default_value = "auto",
            help = "Progress display: auto, plain or tty"
        )]
        progress: ProgressMode,
        #[arg(long, help = "Run only deterministic checks, without LLM calls")]
        checks_only: bool,
        #[arg(long, help = "Run rules at this execution point (default: pre-push)")]
        at: Option<String>,
        #[arg(
            long,
            help = "Show the execution plan without running rules or providers"
        )]
        plan: bool,
        #[arg(
            long,
            requires = "plan",
            help = "Print the plan or plan errors as JSON"
        )]
        json: bool,
        #[arg(long, hide = true, requires = "git_hook_path", conflicts_with = "plan")]
        git_hook: Option<String>,
        #[arg(long, hide = true, requires = "git_hook")]
        git_hook_path: Option<std::path::PathBuf>,
        #[arg(last = true, hide = true, requires = "git_hook")]
        hook_arguments: Vec<std::ffi::OsString>,
    },
    /// Display the last saved report without rerunning rules.
    Status {
        #[arg(long, help = "Show full findings and saved output")]
        details: bool,
    },
    /// Show help.
    Help,
}
