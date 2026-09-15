pub mod config;
pub mod db;
pub mod keys;
pub mod wiring;

#[derive(clap::Subcommand)]
pub enum RelayerCmd {
    #[command(about = "Write relayer-keys.json (chain id -> private key) from the keys in .env")]
    KeysImport,

    #[command(about = "List the packets the relayer is tracking")]
    Transfers {
        #[arg(long, help = "How many rows to show, newest first")]
        limit: Option<u32>,
        #[arg(long, help = "Only rows in this state, e.g. AWAITING_TIMEOUT_FINALITY")]
        status: Option<String>,
    },

    #[command(about = "Count the relayer's transfers, relay requests and submissions by status")]
    Summary,
}
