pub mod mft_scanner;
pub mod cache;
pub mod startup;
pub mod watcher;

use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeInfo {
    pub drive_letter: String,
    pub volume_name: String,
    pub total_size: u64,
    pub is_ntfs: bool,
}
