use std::{
    future::{self, pending},
    io,
    str::{self, from_utf8},
    time::Duration,
};

use anyhow::Result;
use common::*;
use futures::{stream::FuturesUnordered, Future, SinkExt};
use serde::{Deserialize, Serialize};
use serialport::SerialPortType;
use spinners::Spinner;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    task::JoinSet,
    time::sleep,
};
use tokio_serial::SerialPortBuilderExt;

#[tokio::main]
async fn main() -> Result<()> {
    let mut sp = Spinner::new(
        spinners::Spinners::Arc,
        "waiting for conneciton".to_owned(),
    );
    loop {
        let rx = try_conn(&mut sp).await;
        if let Ok(rx) = rx {
            if rx {
                break;
            }
        } else {
            println!("{:?}", rx);
        }
        sleep(Duration::from_millis(500)).await;
    }

    // pending::<()>();

    Ok(())
}

async fn try_conn(spin: &mut Spinner) -> Result<bool> {
    let ports = serialport::available_ports()?;
    let mut fv = JoinSet::new();
    fv.spawn(async { Ok(()) });
    let mut found = false;
    for p in ports {
        if let SerialPortType::UsbPort(u) = p.port_type {
            if u.manufacturer.as_ref().unwrap() == "Plein" {
                spin.stop_with_message(format!("found device {}", u.product.unwrap()));
                fv.spawn(handle_g4(p.port_name));
                found = true;
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

    Ok(found)
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
                    println!("read_len={}, {:?}", n, &buf[..10]);
                    let mut copy = buf.clone();
                    match postcard::take_from_bytes_cobs::<Message>(&mut copy[..(n + skip)]) {
                        Ok((decoded, remainder)) => {
                            buf[..remainder.len()].copy_from_slice(&remainder);
                            skip = remainder.len();
                            dbg!(&decoded);
                        }
                        Err(er) => {
                            println!("read, {:?}", er);
                            let sentinel = buf.iter().position(|k| *k == 0);
                            if let Some(pos) = sentinel {
                                buf[..(copy.len() - pos)].copy_from_slice(&copy[pos..]);
                                skip = 0;
                            } else {
                                buf.fill(0);
                                skip = 0;
                            }
                        }
                    }
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
        hall_speed: Default::default(),
    };
    let mut vec = [0; 1024];
    let mut coded = postcard::to_slice_cobs(&msg, &mut vec).unwrap();
    dbg!(&coded, coded.len());

    let (decoded, buf): (Message, _) = postcard::take_from_bytes_cobs(&mut coded).unwrap();
    dbg!(decoded);
}
