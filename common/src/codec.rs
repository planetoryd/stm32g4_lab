use core::{cmp::min, mem::size_of};

use serde::{Deserialize, Serialize};

pub enum ReaderState<'de, 'buf, T: Deserialize<'de>> {
    ReadingHeader(usize, [u8; size_of::<Header>()]),
    HeaderReceived(Header),
    ReadingData(usize, &'buf mut [u8], Header),
    Decoded(&'de mut T),
    BufferOverflow,
    UnexpectedEOF,
}

#[derive(Serialize, Deserialize)]
pub struct Header {
    data_len: usize,
}

impl<'de, 'buf, T: Deserialize<'de>> ReaderState<'de, 'buf, T> {
    pub fn advance(&mut self, reading: &[u8], pos: &mut usize) {
        match self {
            ReaderState::ReadingHeader(read, bytes) => {
                if *read == bytes.len() {
                    let v = postcard::from_bytes(bytes).unwrap();
                    *self = ReaderState::HeaderReceived(v);
                } else {
                    let len = min(bytes.len(), reading.len() - *pos);
                    if len > 0 {
                        bytes[*read..(*read + len)].copy_from_slice(&reading[*pos..(*pos + len)]);
                        *pos += len;
                    }
                }
            }
            Self::HeaderReceived(_) => unreachable!(),
            Self::ReadingData(read, buf, hd) => {
                let len = reading.len() - *pos;
                if len == 0 {
                    *self = Self::UnexpectedEOF
                } else {
                    let start = *read;
                    let end = *read + buf.len();
                    if buf.len() < end {
                        *self = Self::BufferOverflow;
                    } else {
                        buf[start..end].copy_from_slice(&reading[*pos..(*pos + len)]);
                        *read += len;
                    }
                }
            }
            _ => unreachable!(),
        };
    }
    pub fn use_buf(self, supplied: &'buf mut [u8]) -> Self {
        match self {
            Self::HeaderReceived(hd) => Self::ReadingData(0, supplied, hd),
            _ => unreachable!(),
        }
    }
}
