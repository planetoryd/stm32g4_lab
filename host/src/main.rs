use std::{
    future::{self, pending},
    io,
    str::{self, from_utf8},
};

use anyhow::Result;
use common::*;
use futures::{stream::FuturesUnordered, Future, SinkExt};
use serde::{Deserialize, Serialize};
use serialport::SerialPortType;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    task::JoinSet,
};
use tokio_serial::SerialPortBuilderExt;

#[tokio::main]
async fn main() -> Result<()> {
    let ports = serialport::available_ports().expect("No ports found!");
    let mut fv = JoinSet::new();
    fv.spawn(async { Ok(()) });
    for p in ports {
        if let SerialPortType::UsbPort(u) = p.port_type {
            if u.manufacturer.as_ref().unwrap() == "Plein" {
                println!("found device {}", u.product.unwrap());
                fv.spawn(handle_g4(p.port_name));
            }
        }
    }

    loop {
        if let Some(rx) = fv.join_next().await {
            rx??;
        } else {
            break;
        }
    }

    // pending::<()>();

    Ok(())
}

async fn handle_g4(portname: String) -> Result<()> {
    println!("handle g4 {}", portname);
    let mut dev = tokio_serial::new(portname, 9600)
        .open_native_async()
        .unwrap();

    dev.set_exclusive(true)?;
    // let msg = Message::default();
    // let coded = serde_json::to_vec(&msg)?;
    // dev.write_all(&coded).await?;

    let mut buf = [0; 2048];
    let mut skip = 0;
    loop {
        match dev.try_read(&mut buf[skip..]) {
            Result::Ok(n) => {
                if n > 0 {
                    // buf.iter().position();
                    println!("read_len={}, {:?}", n, &buf[..10]);
                    let mut copy = buf.clone();
                    let (decoded, remainder): (Message, _) =
                        postcard::take_from_bytes_cobs(&mut copy[..n]).unwrap();
                    buf[..remainder.len()].copy_from_slice(&remainder);
                    skip = remainder.len();
                    dbg!(&decoded);
                }
            }
            Result::Err(er) => {
                if er.kind() == io::ErrorKind::WouldBlock {
                    dev.readable().await?;
                }
            }
        }
    }

    Ok(())
}

#[test]
fn test_cobs() {
    let msg = Message {
        hall_speed: Some(1),
        hall_volt: Some(3),
    };
    let mut vec = [0; 1024];
    let mut coded = postcard::to_slice_cobs(&msg, &mut vec).unwrap();
    dbg!(&coded, coded.len());

    let (decoded, buf): (Message, _) = postcard::take_from_bytes_cobs(&mut coded).unwrap();
    dbg!(decoded);
}
