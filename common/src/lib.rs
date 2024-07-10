#![cfg_attr(not(test), no_std)]

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
    ConfigDelta(Setting),
    ConfigState(G4Settings),
}

/// in us
pub const FREQ_PRESETS: [u64; 5] = [12, 128, 512, 8192, 32768];

#[derive(Serialize, Deserialize, Default, Debug, Clone, Format)]
pub struct G4Settings {
    /// in us
    pub sampling_interval: u64,
    /// in us
    pub min_report_interval: u64,
    /// used for ui, num of sample bytes
    pub sampling_window: usize,
}

impl G4Settings {
    pub fn duration_to_samples(&self, milli: u64) -> u64 {
        milli * 1000 / self.sampling_interval
    }
    pub fn duration_to_sample_bytes(&self, milli: u64) -> usize {
        (self.duration_to_samples(milli) / 8) as usize
    }
}

pub trait SettingState {
    type Delta: ApplyOp;
    fn push(&mut self, delta: Self::Delta);
    fn to_delta(&self, new: Self) -> Vec<Self::Delta, 16>;
}

pub trait ApplyOp {
    // deltas are finally used for applying settings, like RPC calls
}

impl ApplyOp for Setting {}

impl SettingState for G4Settings {
    type Delta = Setting;
    fn push(&mut self, delta: Self::Delta) {
        match delta {
            Setting::SetRefreshIntv(t) => {
                self.min_report_interval = t;
            }
            Setting::SetSamplingIntv(t) => {
                self.sampling_interval = t;
            }
            _ => unreachable!(),
        }
    }
    fn to_delta(&self, new: Self) -> Vec<Self::Delta, 16> {
        todo!()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Format)]
pub enum Setting {
    /// Time in milli secs
    SetViewport(usize),
    SetRefreshIntv(u64),
    SetSamplingIntv(u64),
}

pub const HALL_BYTES: usize = 256;
pub const MAX_PACKET_SIZE: usize = 4096;
pub const BUF_SIZE: usize = MAX_PACKET_SIZE * 2;

#[test]
pub fn constraints() {
    assert!(size_of::<G4Message>() < MAX_PACKET_SIZE);
    assert!(size_of::<G4Command>() < MAX_PACKET_SIZE);
}

pub mod cob;