use core::{cmp::min, mem::size_of};

use serde::{Deserialize, Serialize};

pub enum ReaderState<'buf, T: Sized> {
    ReadingHeader(usize, [u8; size_of::<Header>()]),
    HeaderReceived(Header),
    ReadingData(usize, &'buf mut [u8], Header),
    Decoded(T),
    BufferOverflow,
    UnexpectedEOF,
    Unreachable,
}

#[derive(Serialize, Deserialize)]
pub struct Header {
    data_len: usize,
}

impl<'buf, T> Default for ReaderState<'buf, T> {
    fn default() -> Self {
        Self::ReadingHeader(0, Default::default())
    }
}

impl<'buf, T: Sized> ReaderState<'buf, T> {
    fn unreachable(&mut self) {
        *self = Self::Unreachable
    }
    pub fn read_header(&mut self, reading: &[u8], pos: &mut usize) {
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
            Self::HeaderReceived(_) => self.unreachable(),
            _ => self.unreachable(),
        };
    }
    pub fn read<'de>(self, reading: &[u8], pos: &mut usize) -> Self
    where
        T: Deserialize<'de>,
        'buf: 'de,
    {
        match self {
            Self::ReadingData(mut read, buf, hd) => {
                if *pos > reading.len() {
                    return Self::Unreachable;
                }
                let len = reading.len() - *pos;
                if len == 0 {
                    Self::UnexpectedEOF
                } else {
                    let start = read;
                    let end = read + buf.len();
                    if start >= buf.len() {
                        let buf = &*buf;
                        let de = postcard::from_bytes(&buf).unwrap();
                        ReaderState::Decoded(de)
                    } else {
                        if buf.len() < end {
                            Self::BufferOverflow
                        } else {
                            buf[start..end].copy_from_slice(&reading[*pos..(*pos + len)]);
                            read += len;
                            Self::ReadingData(read, buf, hd)
                        }
                    }
                }
            }
            _ => Self::Unreachable,
        }
    }
    pub fn use_buf(self, supplied: &'buf mut [u8]) -> Self {
        match self {
            Self::HeaderReceived(hd) => Self::ReadingData(0, supplied, hd),
            _ => Self::Unreachable,
        }
    }
}

pub fn build_frame<T: Serialize>(data: &T) {
    // let serded = postcard::serialize_with_flavor(data, storage)
    // let hd = Header {
    //     data_len: 
    // }
}

#[test]
fn test_reader() {}
