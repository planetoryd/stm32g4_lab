#![no_std]

use core::mem::size_of;

use heapless::Vec;
use serde::{self, Deserialize, Serialize};

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct G4Message {
    /// Data from hall effect sensor
    pub hall: Vec<u8, HALL_BYTES>,
}

pub const HALL_BYTES: usize = 256;
pub const MAX_PACKET_SIZE: usize = 4096;
pub const BUF_SIZE: usize = MAX_PACKET_SIZE * 2;

pub fn constraints() {
    assert!(size_of::<G4Message>() < MAX_PACKET_SIZE);
}
