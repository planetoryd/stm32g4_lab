#![allow(unreachable_code)]
#![feature(decl_macro)]

use std::any::{self, TypeId};
use std::cmp::{self, min};
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, VecDeque};
use std::future::pending;
use std::iter::repeat;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use std::{default, iter};

use balance::BalanceChart;
use common::num::log2;
use common::{
    G4Command, G4Message, G4Settings, Setting, SettingState, BUF_SIZE, FREQ_PRESETS,
    MAX_PACKET_SIZE,
};
use freq::{Col, FreqChart};
use futures::channel::mpsc::{self, Receiver, Sender};
use futures::lock::Mutex;
use futures::{join, SinkExt, StreamExt};
use iced::alignment::{Horizontal, Vertical};
use iced::widget::canvas::path::lyon_path::geom::euclid::approxeq::ApproxEq;
use iced::widget::canvas::path::lyon_path::geom::euclid::num::Round;
use iced::widget::{
    self, button, column, container, row, slider, text, toggler, vertical_slider, Column,
    Container, Row,
};
use iced::{executor, Alignment, Length, Padding, Point};
use iced::{Application, Command, Element, Settings, Theme};
use iced_aw::{spinner, Spinner};
use meta::{MetaChart, ReportStat};
use misc::fill_vec_p2;
use plotters::style;
use plotters_iced::{Chart, ChartWidget};
use ringbuf::traits::{Consumer, Observer, Producer, RingBuffer};
use ringbuf::{HeapRb, LocalRb};
use serialport::{SerialPort, SerialPortType};
use spectrum_analyzer::windows::hann_window;
use spectrum_analyzer::{samples_fft_to_spectrum, Frequency};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_serial::SerialPortBuilderExt;

// hall sensor line plot
// heater temp plot
// dac output voltage plot

mod balance;
mod freq;
mod meta;
mod misc;

macro forever() {
    pending::<()>()
}

pub fn main() -> iced::Result {
    console_subscriber::init();

    Page::run(Settings {
        antialiasing: true,
        ..Settings::default()
    })
}

#[derive(Default)]
struct Page {
    pub hall: HallChart,
    pub freq: FreqChart,
    pub meta: MetaChart,
    pub ba: BalanceChart,
    pub g4_conn: ConnState,
    pub g4_sx: Option<Sender<PushToG4>>,
    pub stat: Stats,
    pub dac_val: u32,
}

#[derive(Debug, Clone)]
enum Msg {
    G4Conn(ConnState),
    G4Data(G4Message),
    G4Setting(Setting),
    G4Cmd(G4Command),
    Stats(Stats),
    Tick,
    ContinualWeighing(bool),
    Null,
    Omit(u32),
    BaSelect(BaSelect),
}

#[derive(Debug, Clone)]
enum BaSelect {
    Begin(Point),
    Move(Point),
    End(Point),
    Clear,
}

#[derive(Debug, Clone, Default)]
struct Stats {
    reports_per_sec: u32,
    frame_per_sec: u32,
}

enum PushToG4 {
    Notify,
    CMD(G4Command),
}

static WEIGHING: AtomicBool = AtomicBool::new(false);
static mut G4_RX: Option<Receiver<PushToG4>> = None;
static mut G4_CONF: Option<G4Settings> = None;

#[derive(Debug, Default, Clone)]
enum ConnState {
    #[default]
    Waiting,
    Connected,
    /// so that we dont clear the dashboard when device disconnects
    Disconnected,
}

#[test]
fn test_vec() {
    let mut a = [0; 8];
    a[3..4].fill(1);
    a.copy_within(3..4, 0);
    dbg!(&a);
}

impl Application for Page {
    type Executor = executor::Default;
    type Flags = ();
    type Message = Msg;
    type Theme = Theme;

    fn new(_flags: ()) -> (Page, Command<Self::Message>) {
        let mut k = Page::default();
        let cols = [
            Col {
                kind: freq::Colkind::Freq,
                width: 60.,
            },
            Col {
                kind: freq::Colkind::Val,
                width: 60.,
            },
        ];

        k.freq.cols = cols.to_vec();
        let (s, r) = mpsc::channel(1);
        k.g4_sx = Some(s);
        unsafe {
            G4_RX = Some(r);
            G4_CONF = Some(Default::default());
        }

        (k, Command::none())
    }

    fn title(&self) -> String {
        String::from("device")
    }

    fn update(&mut self, msg: Self::Message) -> Command<Self::Message> {
        match msg {
            Msg::ContinualWeighing(val) => WEIGHING.store(val, Ordering::SeqCst),
            Msg::G4Conn(conn) => self.g4_conn = conn,
            Msg::G4Data(data) => {
                if let Some(sett) = data.state {
                    println!("{:?}", sett);
                }
                let new = data
                    .hall
                    .iter()
                    .enumerate()
                    .map(|(x, y)| {
                        let mut v = [0; 8];
                        for i in 0..8 {
                            let k = ((*y) & (1 << i)) != 0;
                            v[i] = k.into();
                        }
                        v.into_iter()
                    })
                    .flatten();
                let stat = ReportStat::from_msg(&data);
                REPORT_COUNTER.fetch_add(1, Ordering::SeqCst);
                self.hall.data_points.push_iter_overwrite(new);
                self.meta.reports.push_overwrite(stat);
                if data.balance.len() > 0 {
                    self.ba.pcoupler = data.balance.into_array().unwrap().into();
                }
                if data.balance_val > 0 {
                    self.ba.measurements.push_overwrite(data.balance_val);
                    if WEIGHING.load(Ordering::SeqCst) {
                        let _ = self.update(Msg::G4Cmd(G4Command::Weigh));
                    }
                }
            }
            Msg::G4Setting(set) => {
                if let Some(ref mut sx) = &mut self.g4_sx {
                    let g4 = unsafe { G4_CONF.as_mut().unwrap() };
                    match &set {
                        Setting::SetViewport(n, v) => {
                            self.hall.viewport = *n;
                            g4.sampling_window = g4.duration_to_sample_bytes(*v as u64);
                            self.hall.viewport_points = g4.sampling_window * 8;
                            self.hall.data_points =
                                HeapRb::new(self.hall.viewport_points.try_into().unwrap());
                        }
                        _ => g4.push(set.clone()),
                    };
                    let _ = sx.try_send(PushToG4::Notify);
                } else {
                    unreachable!()
                }
            }
            Msg::G4Cmd(cmd) => {
                match &cmd {
                    G4Command::SetDAC(dac) => {
                        self.dac_val = *dac;
                    }
                    _ => (),
                }
                if let Some(ref mut sx) = &mut self.g4_sx {
                    let _ = sx.try_send(PushToG4::CMD(cmd));
                } else {
                    unreachable!()
                }
            }
            Msg::Stats(stat) => self.stat = stat,
            Msg::Tick => {
                let v: Vec<_> = self
                    .hall
                    .data_points
                    .iter()
                    .rev()
                    .take(16384) // max size supported by analyzer
                    .map(|x| (*x as i16).into())
                    .collect();
                let mut hw = hann_window(&v[..]);
                let g4 = unsafe { G4_CONF.as_mut().unwrap() };
                let rate = Duration::from_secs(1).as_micros() / g4.sampling_interval as u128;
                fill_vec_p2(&mut hw);
                // println!("samples {}. rate {}hz", hw.len(), rate);
                let spec = samples_fft_to_spectrum(
                    &hw[..],
                    rate.try_into().unwrap(),
                    spectrum_analyzer::FrequencyLimit::Range(0.2, min(rate / 2, 1700) as f32),
                    None,
                );
                match spec {
                    Ok(spec) => {
                        for (f, v) in spec.data() {
                            let rounded = if f.val() < 1. { 0.5 } else { f.val().round() };
                            self.freq
                                .freqs
                                .insert(Frequency::from(rounded), Frequency::from(v.val().round()));
                        }
                        let mut vec: Vec<_> = self.freq.freqs.iter().collect();
                        vec.sort_by(|a, b| {
                            let k = a.1.cmp(b.1);
                            if k == cmp::Ordering::Equal {
                                b.0.cmp(a.0)
                            } else {
                                k
                            }
                        });
                        let rows = vec
                            .into_iter()
                            .map(|(freq, val)| freq::Row {
                                freq: *freq,
                                val: *val,
                            })
                            .rev();
                        self.freq.top = rows.collect();
                    }
                    Err(er) => {
                        println!("{:?}", er);
                    }
                }
            }
            Msg::Omit(o) => {
                self.ba.omit = o;
            }
            Msg::BaSelect(sel) => {
                self.ba.select = match sel {
                    BaSelect::Begin(p) => {
                        let p = Point {
                            x: p.x as i32,
                            y: p.y as i32,
                        };
                        self.ba.cur_moving = true;
                        Some((p, p))
                    }
                    BaSelect::Move(p) => {
                        let p = Point {
                            x: p.x as i32,
                            y: p.y as i32,
                        };
                        if self.ba.select.is_none() {
                            None
                        } else {
                            Some((self.ba.select.unwrap().0, p))
                        }
                    }
                    BaSelect::End(p) => {
                        let p = Point {
                            x: p.x as i32,
                            y: p.y as i32,
                        };
                        self.ba.cur_moving = false;
                        if self.ba.select.is_none() {
                            None
                        } else {
                            Some((self.ba.select.unwrap().0, p))
                        }
                    }
                    BaSelect::Clear => None,
                };
                if self.ba.select == None {
                    self.ba.cur_moving = false;
                }
                if let Some((a, b)) = self.ba.select {
                    let k = a - b;
                    if (k.x.abs() < 10 || k.y.abs() < 10) && !self.ba.cur_moving {
                        self.ba.select = None;
                    }
                }
            }
            Msg::Null => (),
        };
        Command::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let g4 = unsafe { G4_CONF.as_ref().unwrap() };
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
                let btn_state = button("get state")
                    .padding(10)
                    .on_press(Msg::G4Cmd(G4Command::CheckState));
                let btn_weigh = button("weigh")
                    .padding(10)
                    .on_press(Msg::G4Cmd(G4Command::Weigh));

                let tg = toggler(
                    Some("weighing".to_owned()),
                    WEIGHING.load(Ordering::SeqCst),
                    |val| Msg::ContinualWeighing(val),
                );
                let controls = row!(btn_state, btn_weigh)
                    .align_items(Alignment::Center)
                    .spacing(10);
                let sample_int = FREQ_PRESETS.iter().position(|k| *k == g4.sampling_interval);
                g4.sampling_window;
                widget::row([
                    column!(
                        self.hall.view(),
                        self.meta.view(),
                        self.freq.view(),
                        self.ba.view()
                    )
                    .into(),
                    column!(
                        text(format!("Sample per {}us", g4.sampling_interval)),
                        slider(0..=4u32, sample_int.unwrap_or(0) as u32, |t| {
                            let val = FREQ_PRESETS[t as usize];
                            Msg::G4Setting(Setting::SetSamplingIntv(val))
                        }),
                        text(format!("Refresh per {}us", g4.min_report_interval)),
                        slider(
                            6..=18u32,
                            log2(g4.min_report_interval).unwrap_or_default(),
                            |t| {
                                let val_us: u64 = 2usize.pow(t) as u64;
                                Msg::G4Setting(Setting::SetRefreshIntv(val_us))
                            }
                        ),
                        text(format!("Viewport {}ms", self.hall.viewport_millis())),
                        slider(7..=18u32, self.hall.viewport, |t| {
                            let time_in_millisecs: usize = 2usize.pow(t);
                            Msg::G4Setting(Setting::SetViewport(t, time_in_millisecs))
                        }),
                        text(format!("DAC-OUT {}", self.dac_val)),
                        slider(0..=10, self.dac_val, |t| {
                            Msg::G4Cmd(G4Command::SetDAC(t))
                        }),
                        text(format!("Probe-expand {}", g4.probe_expand)),
                        slider(0..=60, g4.probe_expand, |t| {
                            Msg::G4Setting(Setting::ProbeExpand(t as u8))
                        }),
                        controls,
                        text(format!(
                            "reports {}/s \nframe {}/s\nsettings {}/{}",
                            self.stat.reports_per_sec,
                            self.stat.frame_per_sec,
                            g4.id,
                            ACK.load(Ordering::SeqCst)
                        )),
                        tg
                    )
                    .width(200)
                    .spacing(10)
                    .padding(10)
                    .into(),
                ])
                .align_items(iced::Alignment::Center)
                .width(Length::Fill)
                .height(Length::Fill)
                .spacing(0)
                .padding(20)
                .into()
            }
        })
        .into()
    }

    fn subscription(&self) -> iced::Subscription<Self::Message> {
        struct Host;
        struct Stat;
        struct Ticker;
        iced::Subscription::batch([
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
            ),
            iced::subscription::channel(any::TypeId::of::<Stat>(), 10, |mut sx| async move {
                loop {
                    sx.send(Msg::Stats(Stats {
                        reports_per_sec: REPORT_COUNTER
                            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |_| Some(0))
                            .unwrap(),
                        frame_per_sec: DRAW_COUNTER
                            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |_| Some(0))
                            .unwrap(),
                    }))
                    .await
                    .unwrap();
                    sleep(Duration::from_secs(1)).await;
                }
            }),
            iced::subscription::channel(TypeId::of::<Ticker>(), 10, |mut sx| async move {
                loop {
                    sx.send(Msg::Tick).await.unwrap();
                    sleep(Duration::from_millis(500)).await;
                }
            }),
        ])
    }
}

static REPORT_COUNTER: AtomicU32 = AtomicU32::new(0);
static DRAW_COUNTER: AtomicU32 = AtomicU32::new(0);
static ACK: AtomicU64 = AtomicU64::new(0);

async fn handle_g4(portname: String, mut sx: Sender<Msg>) -> anyhow::Result<()> {
    println!("handle g4 {}", portname);
    let rx = unsafe { G4_RX.as_mut().unwrap() };
    let mut dev = tokio_serial::new(portname.clone(), 9600).open_native_async()?;

    // dev.set_exclusive(true)?;
    use ringbuf::*;
    let mut rbuf: LocalRb<storage::Heap<u8>> = LocalRb::new(BUF_SIZE);
    dev.clear(serialport::ClearBuffer::All)?;

    tokio::spawn(async move {
        let mut dev = tokio_serial::new(portname, 9600)
            .open_native_async()
            .unwrap();
        println!("serial writer active");
        while let (Some(rxd), _) = join!(rx.next(), sleep(Duration::from_millis(500))) {
            let pakt = match rxd {
                PushToG4::CMD(cmd) => cmd,
                PushToG4::Notify => G4Command::ConfigState({
                    let conf = unsafe { G4_CONF.as_mut().unwrap() };
                    conf.id += 1;
                    conf.clone()
                }),
            };
            println!("send {:?}", &pakt);
            let en: heapless::Vec<u8, MAX_PACKET_SIZE> = postcard::to_vec_cobs(&pakt).unwrap();
            AsyncWriteExt::write(&mut dev, &en).await.unwrap();
            dev.flush().await.unwrap();
        }
    });

    loop {
        let mut readbuf = Vec::with_capacity(BUF_SIZE);
        let mut packet = Vec::with_capacity(BUF_SIZE);
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
        let des: Result<G4Message, postcard::Error> = postcard::from_bytes_cobs(&mut packet);
        match des {
            Ok(decoded) => {
                ACK.store(decoded.ack, Ordering::SeqCst);
                let s = Msg::G4Data(decoded);
                sx.send(s).await?;
            }
            Err(er) => {
                println!("read err, {:?}", er);
            }
        }
    }

    Ok(())
}

struct HallChart {
    data_points: HeapRb<i32>,
    pub viewport: u32,
    pub viewport_points: u64,
}

impl HallChart {
    pub fn viewport_millis(&self) -> usize {
        2usize.pow(self.viewport)
    }
}

impl Default for HallChart {
    fn default() -> Self {
        let l = 256;
        let h = HeapRb::new(l);
        HallChart {
            data_points: h,
            viewport: 7,
            viewport_points: l as u64,
        }
    }
}

impl HallChart {
    fn view(&self) -> Element<Msg> {
        column!(ChartWidget::new(self), text("hall"))
            .height(150)
            .align_items(iced::Alignment::Center)
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
        DRAW_COUNTER.fetch_add(1, Ordering::SeqCst);
        use plotters::prelude::*;
        let mut c = c
            .x_label_area_size(10)
            .y_label_area_size(30)
            .margin(10)
            .build_cartesian_2d(0..self.viewport_points as usize, 0..1)
            .unwrap();
        c.configure_mesh().draw().unwrap();
        c.draw_series(
            Histogram::vertical(&c)
                .data(self.data_points.iter().enumerate().map(|(x, y)| (x, *y)))
                .margin(0)
                .style(style::colors::BLUE.mix(0.5).filled())
                .baseline(0),
        )
        .unwrap();
    }
}

#[test]
fn spect() -> anyhow::Result<()> {
    let samples = [
        0f32, 1., 1., 1., 1., 0., 0., 1., 0., 0., 0., 0., 1., 0., 1., 1.,
    ];
    let w = hann_window(&samples[..]);
    dbg!(&w);
    let spec =
        samples_fft_to_spectrum(&w, 256, spectrum_analyzer::FrequencyLimit::All, None).unwrap();
    for (fr, v) in spec.data().iter() {
        println!("{}hz -> {}", fr, v);
    }

    Ok(())
}
