use std::{
    future::{self, pending},
    str::{self, from_utf8},
};

use anyhow::Result;
use futures::{stream::FuturesUnordered, Future};
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
    dev.write_all("test".as_bytes()).await?;

    let mut buf = [0; 4];
    loop {
        let x = dev.read_exact(&mut buf).await?;
        if x > 0 {
            println!("{}", from_utf8(&buf).unwrap());
        }
    }
    Ok(())
}
