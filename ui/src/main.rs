use std::any::{self, TypeId};
use std::collections::VecDeque;
use std::default;
use std::future::pending;
use std::time::{Duration, Instant};

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
use serialport::SerialPortType;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_serial::SerialPortBuilderExt;

// hall sensor line plot
// heater temp plot
// dac output voltage plot

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
        k.hall.data_points.extend([0, 0, 1, 0, 1]);
        (k, Command::none())
    }

    fn title(&self) -> String {
        String::from("device")
    }

    fn update(&mut self, msg: Self::Message) -> Command<Self::Message> {
        match msg {
            Msg::G4Conn(conn) => self.g4_conn = conn,
            Msg::G4Data(data) => {
                todo!()
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
                            fv.spawn(async { Ok(()) });
                            for p in ports {
                                if let SerialPortType::UsbPort(u) = p.port_type {
                                    if u.manufacturer.as_ref().unwrap() == "Plein" {
                                        sx.send(Msg::G4Conn(ConnState::Connected)).await?;
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

                            anyhow::Ok(())
                        };
                        loop {
                            (try_conn)(sx.clone()).await?;
                            sleep(Duration::from_millis(500)).await;
                        }

                        pending::<()>().await;
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

async fn handle_g4(portname: String) -> anyhow::Result<()> {
    println!("handle g4 {}", portname);
    let mut dev = tokio_serial::new(portname, 9600).open_native_async()?;

    dev.set_exclusive(true)?;

    let mut buf = [0; 2048];
    let mut skip = 0;
    loop {
        match dev.try_read(&mut buf[skip..]) {
            Result::Ok(n) => {
                if n > 0 {
                    println!("read_len={}, {:?}", n, &buf[..10]);
                    let mut copy = buf.clone();
                    match postcard::take_from_bytes_cobs::<G4Message>(&mut copy[..(n + skip)]) {
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
                if er.kind() == std::io::ErrorKind::WouldBlock {
                    dev.readable().await?;
                }
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct HallChart {
    data_points: VecDeque<i32>,
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
            .build_cartesian_2d(0..5, -2..2)
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
                    .map(|(x, y)| (x.try_into().unwrap(), *y)),
                0,
                style::colors::BLUE.mix(0.4),
            )
            .border_style(ShapeStyle::from(style::colors::BLUE).stroke_width(2)),
        )
        .unwrap();
    }
}
