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
    pub state: Option<G4Settings>,
    pub ack: u64,
    pub balance_val: u16,
    pub balance: Vec<BalanceReport, BALANCE_BYTES>,
    pub dac: Vec<u16, BALANCE_BYTES>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Format)]
pub enum G4Command {
    ConfigDelta(Setting),
    ConfigState(G4Settings),
    CheckState,
    SetDAC(u32),
    Weigh,
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
    /// initial value for balance DAC
    pub sampling_window: u64,
    pub id: u64,
    pub idle: u32,
    pub balance_verify: u16,
    pub balance_interval: u8,
    pub probe_expand: u8,
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
            id: 0,
            idle: 200,
            balance_interval: 5,
            balance_verify: 20,
            probe_expand: 0,
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
            Setting::DACIdle(v) => {
                self.idle = v as u32;
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
    DACIdle(u16),
}

pub const HALL_BYTES: usize = 64;
pub const BALANCE_BYTES: usize = 4;
pub const MAX_PACKET_SIZE: usize = 4096;
pub const BUF_SIZE: usize = MAX_PACKET_SIZE * 2;

// const_assert!(size_of::<G4Message>() < MAX_PACKET_SIZE);
const_assert!(size_of::<G4Command>() < MAX_PACKET_SIZE);

pub mod cob;
pub mod num;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Format)]
pub struct BalanceReport {
    pub feedback: u16,
    pub photoc: u16,
    pub vmap: Option<VMap>,
    pub speed: Option<i32>,
    pub hall: u16,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Format)]
pub struct VMap {
    pub dac: u16,
    pub fbavg: u16,
}
