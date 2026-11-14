use thiserror::Error;

pub type Result<T> = core::result::Result<T, ExporterError>;

#[derive(Debug, Error)]
pub enum ExporterError {
    #[error("GET {url} returned HTTP {status}")]
    ArchiveStatus { url: String, status: u32 },
    #[error("GET {url}: {detail}")]
    ArchiveRequest { url: String, detail: String },
    #[error("reading the body of {url}: {detail}")]
    ArchiveBody { url: String, detail: String },
    #[error("{url} is larger than the {limit}-byte download limit")]
    ArchiveTooLarge { url: String, limit: usize },
    #[error("gunzipping {url}: {detail}")]
    ArchiveGunzip { url: String, detail: String },
    #[error("parsing {url}: {detail}")]
    ArchiveJson { url: String, detail: String },
    #[error("{url} carries no currentLedger")]
    ArchiveNoCurrentLedger { url: String },
    #[error("truncated frame header at byte {offset}")]
    FrameTruncated { offset: usize },
    #[error("the frame at byte {offset} claims {len} bytes, past the end of the file")]
    FrameOverruns { offset: usize, len: usize },
    #[error("decoding {field}: {detail}")]
    Decode { field: String, detail: String },
    #[error("byte span {start}..{end} is outside the record")]
    ByteSpanOutsideRecord { start: usize, end: usize },
    #[error("unsupported SCPHistoryEntry version {version}")]
    UnsupportedHistoryVersion { version: i32 },
    #[error("unsupported TransactionHistoryEntry ext {ext}")]
    UnsupportedTransactionHistoryExt { ext: i32 },
    #[error("unknown network {name:?}, expected mainnet or testnet")]
    UnknownNetwork { name: String },
}

impl ExporterError {
    pub fn decoding(field: impl Into<String>, detail: impl ToString) -> Self {
        Self::Decode {
            field: field.into(),
            detail: detail.to_string(),
        }
    }
}
