#![cfg_attr(not(test), no_std)]

use core::mem::size_of;

use bytemuck::NoUninit;
use defmt::Format;
use heapless::Vec;
use serde::{self, Deserialize, Serialize};
use static_assertions::const_assert;

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct G4Message {
    /// Data from hall effect sensor
    pub hall: Vec<u8, HALL_BYTES>,
    pub state: Option<G4Settings>
}

#[derive(Serialize, Deserialize, Debug, Clone, Format)]
pub enum G4Command {
    ConfigDelta(Setting),
    ConfigState(G4Settings),
    CheckState
}

/// in us
pub const FREQ_PRESETS: [u64; 5] = [128, 1024, 2048, 8192, 32768];

#[derive(Serialize, Deserialize, Debug, Clone, Format, Copy, NoUninit)]
#[repr(C)]
pub struct G4Settings {
    /// in us
    pub sampling_interval: u64,
    /// in us
    pub min_report_interval: u64,
    /// used for ui, num of sample bytes
    pub sampling_window: u64,
}

impl Default for G4Settings {
    fn default() -> Self {
        Self::new()
    }
}

impl G4Settings {
    pub const fn new() -> Self {
        Self {
            sampling_interval: 1024,
            min_report_interval: FREQ_PRESETS[4],
            sampling_window: 200,
        }
    }
}

impl G4Settings {
    pub fn duration_to_samples(&self, milli: u64) -> u64 {
        milli * 1000 / self.sampling_interval
    }
    pub fn duration_to_sample_bytes(&self, milli: u64) -> u64 {
        self.duration_to_samples(milli) / 8
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
    SetViewport(u32, usize),
    SetRefreshIntv(u64),
    SetSamplingIntv(u64),
}

pub const HALL_BYTES: usize = 256;
pub const MAX_PACKET_SIZE: usize = 4096;
pub const BUF_SIZE: usize = MAX_PACKET_SIZE * 2;

const_assert!(size_of::<G4Message>() < MAX_PACKET_SIZE);
const_assert!(size_of::<G4Command>() < MAX_PACKET_SIZE);

pub mod cob;
pub mod num;

