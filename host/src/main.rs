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
    let msg = Message {
        val: 3
    };
    let coded = serde_json::to_vec(&msg)?;
    dev.write_all(&coded).await?;

    let mut buf = [0; 12];
    loop {
        match dev.try_read(&mut buf) {
            Result::Ok(n) => {
                if n > 0 {
                    let decoded: Message = serde_json::from_slice(&buf[..n])?;
                    dbg!(&decoded);
                    buf.fill(0);
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
