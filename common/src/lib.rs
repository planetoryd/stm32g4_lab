#![no_std]
#![feature(saturating_int_impl)]

use framed::{bytes::Config, typed::max_encoded_len};
use serde::{self, Deserialize, Serialize};

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

pub const MAX_PAYLOAD: usize = 2048;

pub fn frame_config() -> Config {
    Config::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
        use framed::bytes::Config;
    }
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Message {
    /// Speed from hall effect sensor
    pub hall_speed: Option<u32>,
    pub hall_volt: Option<u16>
}


pub mod codec;
