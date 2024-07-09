#![no_std]

use core::mem::size_of;

use defmt::Format;
use heapless::Vec;
use serde::{self, Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct G4Message {
    /// Data from hall effect sensor
    pub hall: Vec<u8, HALL_BYTES>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Format)]
pub enum G4Command {
    Config(G4Settings),
}

/// in us
pub const FREQ_PRESETS: [u32; 5] = [12, 128, 512, 8192, 32768];

#[derive(Serialize, Deserialize, Default, Debug, Clone, Format)]
pub struct G4Settings {
    /// in us
    sampling_interval: u64,
    /// in us
    min_report_interval: u64,
}

pub const HALL_BYTES: usize = 256;
pub const MAX_PACKET_SIZE: usize = 4096;
pub const BUF_SIZE: usize = MAX_PACKET_SIZE * 2;

#[test]
pub fn constraints() {
    assert!(size_of::<G4Message>() < MAX_PACKET_SIZE);
    assert!(size_of::<G4Command>() < MAX_PACKET_SIZE);
}
