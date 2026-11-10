pub mod backfill;
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

    #[command(
        about = "Write SQL that fills the relayer's missing transfer amounts, parties and applications from the api"
    )]
    Backfill {
        #[arg(
            long,
            help = "Api to read the transfers from (default: STELLAR_API_URL)"
        )]
        api_url: Option<String>,
        #[arg(
            long,
            default_value = "relayer-backfill.sql",
            help = "Where to write the SQL script"
        )]
        out: std::path::PathBuf,
    },
}
