#![allow(unreachable_code)]
#![feature(decl_macro)]

use std::any::{self, TypeId};
use std::collections::VecDeque;
use std::future::pending;
use std::mem::size_of;
use std::time::{Duration, Instant};
use std::{default, iter};

use common::G4Message;
use futures::channel::mpsc::Sender;
use futures::SinkExt;
use iced::alignment::{Horizontal, Vertical};
use iced::widget::{self, column, container, row, text, Column, Container, Row};
use iced::{executor, Length};
use iced::{Application, Command, Element, Settings, Theme};
use iced_aw::{spinner, Spinner};
use plotters::style;
use plotters_iced::{Chart, ChartWidget};
use ringbuf::traits::{Consumer, Observer, Producer, RingBuffer};
use ringbuf::{HeapRb, LocalRb};
use serialport::{SerialPort, SerialPortType};
use spectrum_analyzer::samples_fft_to_spectrum;
use spectrum_analyzer::windows::hann_window;
use tokio::io::AsyncReadExt;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_serial::SerialPortBuilderExt;

// hall sensor line plot
// heater temp plot
// dac output voltage plot

macro forever() {
    pending::<()>()
}

pub fn main() -> iced::Result {
    Page::run(Settings {
        antialiasing: true,
        ..Settings::default()
    })
}

#[derive(Default)]
struct Page {
    pub hall: HallChart,
    pub g4_conn: ConnState,
}

#[derive(Debug)]
enum Msg {
    G4Conn(ConnState),
    G4Data(G4Message),
}

#[derive(Debug, Default)]
enum ConnState {
    #[default]
    Waiting,
    Connected,
    /// so that we dont clear the dashboard when device disconnects
    Disconnected,
}

impl Application for Page {
    type Executor = executor::Default;
    type Flags = ();
    type Message = Msg;
    type Theme = Theme;

    fn new(_flags: ()) -> (Page, Command<Self::Message>) {
        let mut k = Page::default();
        (k, Command::none())
    }

    fn title(&self) -> String {
        String::from("device")
    }

    fn update(&mut self, msg: Self::Message) -> Command<Self::Message> {
        match msg {
            Msg::G4Conn(conn) => self.g4_conn = conn,
            Msg::G4Data(data) => {
                println!("data len {}", data.hall.len());
                self.hall.data_points.push_slice_overwrite(&data.hall);
            }
        };
        Command::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        container(match self.g4_conn {
            ConnState::Waiting => Element::new(
                container(
                    row!(
                        text("waiting for connection"),
                        Spinner::new().circle_radius(3.),
                    )
                    .spacing(20),
                )
                .center_x()
                .center_y()
                .width(Length::Fill)
                .height(Length::Fill),
            ),
            ConnState::Connected | ConnState::Disconnected => {
                widget::row([column!(self.hall.view()).into()])
                    .align_items(iced::Alignment::Center)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .spacing(20)
                    .into()
            }
        })
        .into()
    }

    fn subscription(&self) -> iced::Subscription<Self::Message> {
        struct Host;
        iced::subscription::channel(
            TypeId::of::<Host>(),
            128,
            |mut sx: Sender<Msg>| async move {
                if let Err(er) = (async move {
                    loop {
                        sx.send(Msg::G4Conn(ConnState::Waiting)).await?;

                        let try_conn = |mut sx: Sender<Msg>| async move {
                            let ports = serialport::available_ports()?;
                            let mut fv = JoinSet::new();
                            // fv.spawn(async { Ok(()) });
                            for p in ports {
                                if let SerialPortType::UsbPort(u) = p.port_type {
                                    if u.manufacturer.as_ref().unwrap() == "Plein" {
                                        sx.send(Msg::G4Conn(ConnState::Connected)).await?;
                                        fv.spawn(handle_g4(p.port_name, sx.clone()));
                                    }
                                }
                            }

                            loop {
                                if let Some(rx) = fv.join_next().await {
                                    let k: Result<(), anyhow::Error> = rx?;
                                    match k {
                                        Err(e) => {
                                            println!("G4 handler error {:?}", e);
                                        }
                                        _ => {}
                                    }
                                } else {
                                    break;
                                }
                            }

                            anyhow::Ok(())
                        };

                        loop {
                            println!("try conn");
                            (try_conn)(sx.clone()).await?;
                            println!("disconnected");
                            sleep(Duration::from_millis(1000)).await;
                        }
                    }
                    #[allow(unreachable_code)]
                    anyhow::Ok(())
                })
                .await
                {
                    println!("{:?}", er);
                    panic!()
                } else {
                    unreachable!()
                }
            },
        )
    }
}

async fn handle_g4(portname: String, mut sx: Sender<Msg>) -> anyhow::Result<()> {
    println!("handle g4 {}", portname);
    let mut dev = tokio_serial::new(portname, 9600).open_native_async()?;

    dev.set_exclusive(true)?;
    use ringbuf::*;
    let mut rbuf: LocalRb<storage::Heap<u8>> = LocalRb::new(4096);
    dev.clear(serialport::ClearBuffer::All)?;

    loop {
        let mut readbuf = Vec::with_capacity(4096);
        let mut packet = Vec::with_capacity(2048);
        let mut terminated = false;
        while !terminated {
            readbuf.clear();
            let n = dev.read_buf(&mut readbuf).await?;
            if n == 0 {
                anyhow::bail!("device eof")
            }
            let k = rbuf.push_slice(&readbuf[..n]);
            for b in rbuf.pop_iter() {
                match packet.len() {
                    0 => {
                        if b == 0 {
                            continue;
                        } else {
                            packet.push(b)
                        }
                    }
                    _ => {
                        if b == 0 {
                            packet.push(0);
                            terminated = true;
                            break;
                        } else {
                            packet.push(b);
                        }
                    }
                }
            }
        }

        // println!("packet fin {}", packet.len());

        let des: Result<G4Message, postcard::Error> = postcard::from_bytes_cobs(&mut packet);
        match des {
            Ok(decoded) => {
                println!("send {}", decoded.hall.len());
                let s = Msg::G4Data(decoded);
                sx.send(s).await?;
                println!("sent");
            }
            Err(er) => {
                println!("read err, {:?}", er);
            }
        }
    }

    Ok(())
}

struct HallChart {
    data_points: HeapRb<u8>,
}

impl Default for HallChart {
    fn default() -> Self {
        let mut h = HeapRb::new(100);
        h.push_iter(iter::repeat(0));
        HallChart { data_points: h }
    }
}

impl HallChart {
    fn view(&self) -> Element<Msg> {
        column!(ChartWidget::new(self), text("hall"))
            .align_items(iced::Alignment::Center)
            .padding(20)
            .into()
    }
}

impl Chart<Msg> for HallChart {
    type State = ();
    fn build_chart<DB: plotters::prelude::DrawingBackend>(
        &self,
        state: &Self::State,
        mut c: plotters::prelude::ChartBuilder<DB>,
    ) {
        use plotters::prelude::*;
        let mut c = c
            .x_label_area_size(20)
            .y_label_area_size(20)
            .margin(20)
            .build_cartesian_2d(0..self.data_points.occupied_len() * 8, -1..2)
            .unwrap();
        c.configure_mesh()
            .bold_line_style(style::colors::BLUE.mix(0.2))
            .light_line_style(plotters::style::colors::BLUE.mix(0.1))
            .draw()
            .unwrap();
        c.draw_series(
            AreaSeries::new(
                self.data_points
                    .iter()
                    .enumerate()
                    .map(|(x, y)| {
                        let mut v = [0; 8];
                        for i in 0..8 {
                            let k = ((*y) & (1 << i)) != 0;
                            v[i] = k.into();
                        }
                        v.into_iter().enumerate().map(move |(i, y)| (x * 8 + i, y))
                    })
                    .flatten(),
                0,
                style::colors::BLUE.mix(0.4),
            )
            .border_style(ShapeStyle::from(style::colors::BLUE).stroke_width(2)),
        )
        .unwrap();
    }
}

#[test]
fn spect() -> anyhow::Result<()> {
    let samples = [0f32, 1., 0., 0., 1., 0., 0., 1.];
    let w = hann_window(&samples[..]);
    dbg!(&w);
    let spec =
        samples_fft_to_spectrum(&w, 4, spectrum_analyzer::FrequencyLimit::All, None).unwrap();
    for (fr, v) in spec.data().iter() {
        println!("{}hz -> {}", fr, v);
    }

    Ok(())
}
