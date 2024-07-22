#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]
#![feature(iter_next_chunk)]
#![feature(iter_array_chunks)]
#![feature(async_closure)]
#![allow(unreachable_code)]
#![allow(unused_mut)]

use core::{
    borrow::BorrowMut,
    cmp::{max, min},
    future::{pending, Pending},
    hash::{Hash, Hasher, SipHasher},
    mem::MaybeUninit,
    ops::{Range, RangeFrom},
    pin::pin,
    sync::atomic::{self, AtomicBool, AtomicU16, AtomicU32, Ordering::SeqCst},
};

use ::atomic::Atomic;
use bbqueue::BBBuffer;
use bittle::{BitsMut, BitsOwned};
use common::{
    cob::{
        self,
        COBErr::{Codec, NextRead},
        COBSeek, COB,
    },
    BalanceReport, G4Command, G4Message, G4Settings, VMap, BALANCE_BYTES, HALL_BYTES,
    MAX_PACKET_SIZE,
};
use defmt::*;
use embassy_futures::{
    join,
    select::{self, Either},
};
use embassy_sync::{
    blocking_mutex::{raw::CriticalSectionRawMutex, CriticalSectionMutex},
    channel::{self, Channel},
    mutex::Mutex,
    pubsub::PubSubChannel,
    signal::Signal,
};
use embassy_usb::{
    class::cdc_acm::{self, CdcAcmClass},
    driver::EndpointError,
    UsbDevice,
};

use futures_util::{future::select, Stream, StreamExt};
use heapless::{Deque, LinearMap, Vec};
use lock_free_static::lock_free_static;
#[cfg(not(feature = "defmt"))]
use panic_halt as _;
use ringbuf::{
    storage::Owning,
    traits::{Producer, SplitRef},
    wrap::caching::Caching,
    SharedRb, StaticRb,
};
#[cfg(feature = "defmt")]
use {defmt_rtt as _, panic_probe as _};

use defmt::*;
use embassy_executor::{SpawnToken, Spawner};
use embassy_stm32::{
    adc::{self, Adc, Temperature, VrefInt},
    bind_interrupts,
    cordic::{self, utils, Cordic},
    dac::{Dac, DacCh1, DacCh2, DacChannel, TriggerSel, Value},
    dma::WritableRingBuffer,
    exti::ExtiInput,
    flash::BANK1_REGION,
    gpio::{self, Input, Level, Output, Pull, Speed},
    interrupt::{self, InterruptExt},
    opamp::{self, *},
    peripherals::{
        ADC1, ADC2, DAC1, DAC3, DMA1, EXTI0, EXTI1, EXTI6, OPAMP1, OPAMP2, OPAMP3, PA1, PA2, PA4,
        PA5, PA6, PA7, PB0, PB10, PC6, RNG, TIM1, TIM2, USB,
    },
    rng::Rng,
    time::{khz, mhz},
    timer::{low_level, pwm_input::PwmInput},
    usb::Driver,
    Config, Peripheral,
};
use embassy_stm32::{peripherals, rng, usb};
use embassy_time::{Delay, Duration, Instant, Ticker, Timer};
use micromath::*;

bind_interrupts!(
    struct Irqs {
        USB_LP => usb::InterruptHandler<peripherals::USB>;
        RNG => rng::InterruptHandler<peripherals::RNG>;
    }
);

type HallData = u8;

struct Notif;

static CONF: Atomic<G4Settings> = Atomic::new(G4Settings::new());
static CONF_NOTIF: PubSubChannel<CriticalSectionRawMutex, (), 4, 2, 1> = PubSubChannel::new();
static CHECK_STATE: AtomicBool = AtomicBool::new(false);
static WEIGH_NOTIF: Channel<CriticalSectionRawMutex, Notif, 1> = Channel::new();
static DAC_NOTIF: Channel<CriticalSectionRawMutex, Notif, 1> = Channel::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.pll = Some(Pll {
            source: PllSource::HSI,
            prediv: PllPreDiv::DIV4,
            mul: PllMul::MUL85,
            divp: None,
            divq: Some(PllQDiv::DIV6), // 48mhz for USB
            // Main system clock at 170 MHz
            divr: Some(PllRDiv::DIV2),
        });
        config.rcc.mux.adc12sel = mux::Adcsel::SYS;
        config.rcc.sys = Sysclk::PLL1_R;

        // config.enable_ucpd1_dead_battery = true;
    }
    debug!("init");
    let mut p = embassy_stm32::init(config);
    // info!("init opmap for hall effect sensor");
    // let mut adc: Adc<peripherals::ADC1> = Adc::new(p.ADC1);
    // adc.set_sample_time(adc::SampleTime::CYCLES24_5);
    // adc.blocking_read(&mut p.PB0);
    // let mut vrefint = adc.enable_vrefint();
    // let mut temp = adc.enable_temperature();
    unwrap!(spawner.spawn(led(Output::new(p.PC13, Level::High, Speed::Low))));

    let driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);
    let mut config = embassy_usb::Config::new(0xc0de, 0xcafe);
    config.manufacturer = Some("Plein");
    config.product = Some("device G4");
    config.serial_number = Some("1");
    config.device_class = 0xEF;
    config.device_sub_class = 0x02;
    config.device_protocol = 0x01;
    config.composite_with_iads = true;

    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut control_buf = [0; 64];

    let mut state = embassy_usb::class::cdc_acm::State::new();

    let mut builder = embassy_usb::Builder::new(
        driver,
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut [], // no msos descriptors
        &mut control_buf,
    );
    let rngen = Rng::new(p.RNG, Irqs);
    let class = embassy_usb::class::cdc_acm::CdcAcmClass::new(&mut builder, &mut state, 64);
    let (mut sx, mut rx) = class.split();
    info!("usb max packet size, {}", sx.max_packet_size());
    let mut usb = builder.build();
    let usb_fut = usb.run();
    let (hall_prod, cons_hall) = unwrap!(HALL_DATA.try_split().map_err(|x| "bbq split"));

    defmt::assert!(ADC_BUF.init());

    let (ba_prod, ba_cons) = unwrap!(ADC_BUF.get_mut()).split_ref();
    let mut bufrd = BufReaders {
        hall_con: cons_hall,
        balance: ba_cons,
    };

    let report_fut = async {
        let sig = unwrap!(unsafe { USB_SIGNAL.publisher() });
        loop {
            info!("waiting for usb");
            sx.wait_connection().await;
            unsafe {
                USB_STATE = UsbState::Connected;
                sig.publish(USB_STATE).await;
            }
            info!("Connected");
            let x = join::join(report(&mut sx, &mut bufrd), listen(&mut rx)).await;
            info!("Disconnected, {:?}", x);
            unsafe {
                USB_STATE = UsbState::Waiting;
                sig.publish(USB_STATE).await;
            }
        }
    };

    debug!("start balance");

    unwrap!(spawner.spawn(feedbacked_balance(
        p.DAC1, p.DMA1, p.PA5, p.PB0, p.ADC2, p.ADC1, p.OPAMP2, p.PA6, ba_prod, p.OPAMP1, p.PA7
    )));
    unwrap!(spawner.spawn(hall_digital(p.PA1, hall_prod)));

    embassy_futures::join::join(usb_fut, report_fut).await;
}

#[embassy_executor::task]
async fn led(mut out: Output<'static>) {
    loop {
        if unsafe { USB_STATE } == UsbState::Connected {
            out.set_low();
        } else {
            out.toggle();
        }
        Timer::after_millis(200).await;
    }
}

/// only for analog output hall sensors
/// what im using is 3144, which is digital
#[embassy_executor::task]
async fn hall_watcher(
    mut adc: Adc<'static, peripherals::ADC1>,
    op1: OPAMP1,
    mut pa1: PA1,
    mut vref: VrefInt,
    temp: Temperature,
) {
    let mut op1 = OpAmp::new(op1, OpAmpSpeed::Normal);
    let mut opi = op1.buffer_int(&mut pa1, OpAmpGain::Mul1);
    // let (sx, rx) = (HALL_SPEED.sender(), HALL_SPEED.receiver());
    loop {
        let va = adc.blocking_read(&mut opi);
        // let _ = sx.try_send(va);
        // Timer::after_millis(HALL_INTERVAL).await;
    }
}

use embassy_futures::select::select as slct;

#[derive(Clone, Copy, Format)]
struct Photocouplers {
    up: bool,
    down: bool,
}
use ringbuf::traits::*;

use bytemuck::*;

#[derive(Clone, Copy, NoUninit, Format)]
#[repr(u8)]
enum PointS {
    Up,
    Down,
    Mid,
}

#[derive(Clone, Copy, NoUninit, Format)]
#[repr(C)]
struct PointV {
    speed: i64,
}

static VELOCITY: Atomic<PointV> = Atomic::new(PointV { speed: 0 });
static PT_STATS: Atomic<PtStats> = Atomic::new(PtStats { sum: 0, counter: 0 });
static FEEDBACK_AVG: Channel<CriticalSectionRawMutex, u32, 2> = Channel::new();
const PT_PAST: u32 = 256;

#[derive(Clone, Copy, NoUninit, Format, Default)]
#[repr(C)]
struct PtStats {
    sum: u32,
    counter: u32,
}
struct StateSpan {
    t: Instant,
    mid: bool,
}

#[embassy_executor::task]
async fn feedbacked_balance(
    dac: DAC1,
    dma: DMA1,
    pin: PA5,
    fb: PB0,
    adc2: ADC2,
    adc1: ADC1,
    op2: OPAMP2,
    mut pc: PA6,
    mut prod: Caching<
        &'static SharedRb<Owning<[MaybeUninit<BalanceReport>; ADC_BUF_LEN]>>,
        true,
        false,
    >,
    op1: OPAMP1,
    mut hall: PA7,
) {
    let mut op2 = OpAmp::new(op2, OpAmpSpeed::Normal);
    let mut op2out = op2.buffer_int(fb, OpAmpGain::Mul16);

    let mut adc2 = Adc::new(adc2);
    let mut adc1 = Adc::new(adc1);
    let mut dac2 = DacCh2::new(dac, dma, pin);

    let mut op1 = OpAmp::new(op1, OpAmpSpeed::Normal);
    let mut op1out = op1.buffer_int(hall, OpAmpGain::Mul2);

    dac2.enable();
    static PTST: Atomic<PointS> = Atomic::new(PointS::Up);
    let pcin = Input::new(pc, Pull::Up);

    let mut sts = StaticRb::<StateSpan, 8>::default();
    let (sprod, scon) = sts.split_ref();
    embassy_futures::join::join(
        async {
            let read = || pcin.is_low();
            let mut last: Option<StateSpan> = Some(StateSpan {
                t: Instant::now(),
                mid: read(),
            });
            loop {
                Timer::after_millis(2).await;
                let mut rp = BalanceReport {
                    feedback: adc2.blocking_read(&mut op2out),
                    photoc: if read() { 3000 } else { 1000 },
                    vmap: None,
                    speed: None,
                    hall: adc1.blocking_read(&mut op1out),
                };
                let sp = StateSpan {
                    t: Instant::now(),
                    mid: rp.photoc < 3000,
                };
                if let Some(last) = &mut last {
                    if sp.mid != last.mid {
                        let leave = !sp.mid;
                        let speed = sp.t - last.t;
                        let mut speed = speed.as_micros() as i64;
                        if leave {
                            speed = -speed;
                        }
                        rp.speed = Some(speed as i32);
                        debug!("speed = {}", speed);
                        VELOCITY.store(PointV { speed }, SeqCst);
                        *last = sp;
                    }
                }
                let prev = PT_STATS.fetch_update(SeqCst, SeqCst, |v| {
                    Some(if v.counter >= PT_PAST {
                        PtStats {
                            counter: 0,
                            sum: rp.feedback as u32,
                        }
                    } else {
                        PtStats {
                            counter: v.counter + 1,
                            sum: v.sum + rp.feedback as u32,
                        }
                    })
                });
                if let Ok(prev) = prev {
                    if prev.counter >= PT_PAST {
                        let avg = prev.sum / (prev.counter + 1);
                        let _ = FEEDBACK_AVG.try_send(avg);
                        let stat = VMap {
                            dac: unsafe { DAC_VAL },
                            fbavg: avg as u16,
                        };
                        let _ = rp.vmap.insert(stat);
                    }
                }
                if prod.is_full() {
                    Timer::after_millis(500).await;
                    continue;
                }
                prod.push_iter([rp].into_iter());
            }
        },
        async {
            type DacT = DacCh2<'static, DAC1, DMA1>;
            let val = |da: &DacT| da.read();
            let mut set = |v, da: &mut DacT| {
                unsafe {
                    DAC_VAL = v;
                }
                da.set(Value::Bit12Left(v));
            };
            for x in [0, 40, 29].into_iter().cycle() {
                set(x * 100, &mut dac2);
                debug!("set dac to {}", x);
                Timer::after_millis(1000).await;
            }
            // loop {
            //     let n = PTST.load(atomic::Ordering::Relaxed);
            //     match n {
            //         PointS::Down => {
            //             let t = val(&dac2).saturating_add_signed(-10);
            //             set(t, &mut dac2);
            //         }
            //         PointS::Up => {
            //             let t = val(&dac2).saturating_add_signed(10);
            //             set(t, &mut dac2);
            //         }
            //         _ => (),
            //     }
            //     Timer::after_millis(10).await;
            // }
        },
    )
    .await;
    debug!("fb balance ended");
}

async fn measure_ratio(mut out: impl FnMut(u16), mut read: impl FnMut()) {
    out(0);
}

static mut DAC_VAL: u16 = 0;
static mut BALANCE: bittle::Set<[u8; BALANCE_BYTES]> = bittle::Set::ONES;
static BALANCE_DATA: AtomicBool = AtomicBool::new(false);
static BALANCE_MEAN: AtomicU16 = AtomicU16::new(0);

#[embassy_executor::task]
async fn hall_digital(pa1: PA1, mut prod: bbqueue::Producer<'static, HALL_BUFSIZE>) {
    let p = gpio::Input::new(pa1, Pull::Up);
    unsafe {
        let mut usb = unwrap!(USB_SIGNAL.subscriber());
        loop {
            if USB_STATE == UsbState::Connected
                || usb.next_message_pure().await == UsbState::Connected
            {
                break;
            }
        }
    }
    let mut confn = unwrap!(CONF_NOTIF.subscriber());
    loop {
        let conf = CONF.load(SeqCst);
        let mut tker = Ticker::every(Duration::from_micros(conf.sampling_interval));
        if let Ok(mut buf) = prod.grant_exact(1) {
            let mut i = 0;
            select::select(
                async {
                    for k in 0..buf.len() {
                        let mut byte = 0u8;
                        for pos in 0..8 {
                            let le = p.is_high() as u8;
                            byte |= le << pos;
                            tker.next().await;
                        }
                        buf[i] = byte;
                        i = k + 1;
                    }
                },
                confn.next(),
            )
            .await;
            buf.commit(i);
        } else {
            break;
        }
    }
}

static mut USB_SIGNAL: PubSubChannel<CriticalSectionRawMutex, UsbState, 4, 2, 1> =
    PubSubChannel::new();
static mut USB_STATE: UsbState = UsbState::Waiting;

#[derive(PartialEq, Eq, Clone, Copy)]
enum UsbState {
    Waiting,
    Connected,
}

#[derive(Format)]
struct Disconnected {}

impl From<EndpointError> for Disconnected {
    fn from(val: EndpointError) -> Self {
        match val {
            EndpointError::BufferOverflow => crate::panic!("buffer overflow"),
            EndpointError::Disabled => Disconnected {},
        }
    }
}

async fn listen<'d, T: 'd + embassy_stm32::usb::Instance>(
    rx: &mut cdc_acm::Receiver<'d, Driver<'d, T>>,
) -> Result<(), Disconnected> {
    info!("listening for commands");
    unsafe {
        let mut usb = unwrap!(USB_SIGNAL.subscriber());
        loop {
            if USB_STATE == UsbState::Connected
                || usb.next_message_pure().await == UsbState::Connected
            {
                break;
            }
        }
    }
    const BUFLEN: usize = MAX_PACKET_SIZE;
    let mut buf = [0; BUFLEN];
    let mut rg: Option<RangeFrom<usize>> = None;

    loop {
        rg.get_or_insert(0..);
        let rd = rx.read_packet(&mut buf[unwrap!(rg.take())]).await?;
        if rd == 0 {
            Timer::after_secs(2).await;
            continue;
        }
        let copy = buf.clone();
        let cob: COB<G4Command> = COB::new(&mut buf[..], &copy);
        for msg in cob {
            if let Ok(msg) = msg {
                debug!("{:?}", &msg);
                match msg {
                    G4Command::CheckState => {
                        CHECK_STATE.store(true, SeqCst);
                    }
                    G4Command::ConfigState(state) => {
                        CONF.store(state, SeqCst);
                        info!("config updated");
                    }
                    G4Command::SetDAC(dac) => {
                        debug!("set dac to {}", dac);
                        unsafe {
                            DAC_VAL = dac as u16;
                        }
                        let _ = DAC_NOTIF.try_send(Notif);
                    }
                    G4Command::Weigh => {
                        let _ = WEIGH_NOTIF.try_send(Notif);
                    }
                    _ => (),
                }
            } else {
                match msg.unwrap_err() {
                    NextRead(rg1) => rg = Some(rg1),
                    Codec(er) => {
                        warn!("malformed packet {:?}", er);
                    }
                }
            }
        }
    }
    Ok(())
}

const HALL_BUFSIZE: usize = 1024 * 2;
static HALL_DATA: BBBuffer<HALL_BUFSIZE> = BBBuffer::new();

lock_free_static! {
    static mut ADC_BUF : StaticRb<BalanceReport, ADC_BUF_LEN>  = StaticRb::default();
}
const ADC_BUF_LEN: usize = 4;

struct BufReaders {
    hall_con: bbqueue::Consumer<'static, HALL_BUFSIZE>,
    balance:
        Caching<&'static SharedRb<Owning<[MaybeUninit<BalanceReport>; ADC_BUF_LEN]>>, false, true>,
}

async fn report<'d, T: 'd + embassy_stm32::usb::Instance>(
    class: &mut cdc_acm::Sender<'d, Driver<'d, T>>,
    BufReaders {
        hall_con,
        balance: ba,
    }: &mut BufReaders,
) -> Result<(), Disconnected> {
    info!("reporting enabled");
    unsafe {
        let mut usb = unwrap!(USB_SIGNAL.subscriber());
        loop {
            if USB_STATE == UsbState::Connected
                || usb.next_message_pure().await == UsbState::Connected
            {
                break;
            }
        }
    }
    let mut tkreload = Ticker::every(Duration::from_secs(3));
    loop {
        let e = embassy_futures::select::select(tkreload.next(), async {
            let conf: G4Settings = CONF.load(SeqCst);

            let mut tkrp = Ticker::every(Duration::from_micros(conf.min_report_interval));
            loop {
                let hall: Option<Vec<u8, HALL_BYTES>>;
                if let Ok(rd) = hall_con.read() {
                    let copy = &rd[..min(rd.len(), HALL_BYTES)];
                    hall = Some(unwrap!(heapless::Vec::from_slice(copy)));
                    rd.release(unwrap!(hall.as_ref()).len());
                } else {
                    hall = None;
                };
                let send_state = CHECK_STATE.fetch_update(SeqCst, SeqCst, |_| Some(false));
                let send_state = match send_state {
                    Ok(k) => k,
                    Err(k) => k,
                };
                let bval = BALANCE_MEAN.load(SeqCst);
                if bval > 0 {
                    BALANCE_MEAN.store(0, SeqCst);
                }

                let reply = G4Message {
                    hall: hall.unwrap_or_default(),
                    state: if send_state { Some(conf) } else { None },
                    ack: conf.id,
                    balance_val: bval,
                    balance: ba.pop_iter().collect(),
                    ..Default::default()
                };
                let rx: Result<heapless::Vec<u8, 1024>, postcard::Error> =
                    postcard::to_vec_cobs(&reply);
                if let Ok(coded) = rx {
                    let mut chunks = coded.into_iter().array_chunks::<64>();
                    while let Some(p) = chunks.next() {
                        class.write_packet(&p).await?;
                    }
                    if let Some(p) = chunks.into_remainder() {
                        class.write_packet(p.as_slice()).await?;
                    }
                } else {
                    error!("{:?}", rx.unwrap_err())
                }
                tkrp.next().await;
            }

            Result::<_, Disconnected>::Ok(())
        })
        .await;
        match e {
            Either::Second(s) => s?,
            _ => (),
        }
    }

    Ok(())
}
