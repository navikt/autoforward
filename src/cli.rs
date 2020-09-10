use structopt::{
    clap::AppSettings::{ColorAuto, ColoredHelp},
    StructOpt,
};

#[derive(Debug, StructOpt)]
#[structopt(setting(ColorAuto), setting(ColoredHelp), about)]
pub struct CliInputs {
    /// Verbosity level (-v, -vv, -vvv, etc.)
    /// Default (no `-v` flag(s)) => zeroth verbosity level.
    #[structopt(short, parse(from_occurrences))]
    pub verbosity_level: usize,

    /// Update frequency (in seconds) of forwarding config
    #[structopt(short, long, default_value = "120")]
    pub update_frequency: u64,
}

pub fn verbosity_level(args: &CliInputs) -> usize {
    args.verbosity_level
}
