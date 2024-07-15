#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]
#![feature(iter_next_chunk)]
#![feature(iter_array_chunks)]
#![feature(async_closure)]
#![allow(unreachable_code)]
#![allow(unused_mut)]

use core::{
    cmp::{max, min},
    future::{pending, Pending},
    hash::{Hash, Hasher, SipHasher},
    ops::{Range, RangeFrom},
    sync::atomic::{self, AtomicBool, AtomicU16, AtomicU32, Ordering::SeqCst},
};

use ::atomic::Atomic;
use bbqueue::{BBBuffer, Consumer, Producer};
use bittle::{BitsMut, BitsOwned};
use common::{
    cob::{
        self,
        COBErr::{Codec, NextRead},
        COBSeek, COB,
    },
    G4Command, G4Message, G4Settings, BALANCE_BYTES, HALL_BYTES, MAX_PACKET_SIZE,
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

use futures_util::{Stream, StreamExt};
use heapless::Vec;
#[cfg(not(feature = "defmt"))]
use panic_halt as _;
#[cfg(feature = "defmt")]
use {defmt_rtt as _, panic_probe as _};

use defmt::*;
use embassy_executor::{SpawnToken, Spawner};
use embassy_stm32::{
    adc::{self, Adc, Temperature, VrefInt},
    bind_interrupts,
    dac::{Dac, DacCh1, Value},
    dma::WritableRingBuffer,
    exti::ExtiInput,
    flash::BANK1_REGION,
    gpio::{self, Level, Output, Pull, Speed},
    interrupt,
    opamp::{self, *},
    pac::spi::vals::Ismod,
    peripherals::{DAC1, DMA1, OPAMP1, PA1, PA4, PB0, PB10, RNG, TIM2, USB},
    rng::Rng,
    time::{khz, mhz},
    timer::pwm_input::PwmInput,
    usb::Driver,
    Config,
};
use embassy_stm32::{peripherals, rng, usb};
use embassy_time::{Delay, Duration, Instant, Ticker, Timer};

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
    config.product = Some("HALL_BUFSIZE device G4");
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

    let (prod, mut cons) = HALL_DATA.try_split().unwrap();

    let report_fut = async {
        let sig = unsafe { USB_SIGNAL.publisher() }.unwrap();
        loop {
            info!("waiting for usb");
            sx.wait_connection().await;
            unsafe {
                USB_STATE = UsbState::Connected;
                sig.publish(USB_STATE).await;
            }
            info!("Connected");
            let x = join::join(report(&mut sx, &mut cons), listen(&mut rx)).await;
            info!("Disconnected, {:?}", x);
            unsafe {
                USB_STATE = UsbState::Waiting;
                sig.publish(USB_STATE).await;
            }
        }
    };

    unwrap!(spawner.spawn(balance(p.DAC1, p.DMA1, p.PA4, p.PB0, rngen)));
    unwrap!(spawner.spawn(hall_digital(p.PA1, prod)));
    // spawner
    //     .spawn(hall_watcher(adc, p.OPAMP1, p.PA1, vrefint, temp))
    //     .unwrap();
    embassy_futures::join::join(usb_fut, report_fut).await;

    // pending::<()>().await;
    // Driver::new();

    // loop {
    //     info!("PA1={}", adc.read(&mut opi));
    //     Timer::after_millis(500).await;
    // }
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

enum PointerState {
    Below,
    Above,
    Mid,
}

#[embassy_executor::task]
async fn balance(dac: DAC1, dma: DMA1, pin: PA4, pc: PB0, rng: Rng<'static, peripherals::RNG>) {
    let mut dac = DacCh1::new(dac, dma, pin);
    let pc = gpio::Input::new(pc, Pull::Down);
    dac.enable();
    dac.set(Value::Bit12Left(0));
    let max_dac: u16 = 4095;
    loop {
        WEIGH_NOTIF.receive().await;
        let conf = CONF.load(SeqCst);
        dac.set(Value::Bit12Left(conf.bump as u16));
        Timer::after_millis(conf.bump_wait as u64).await;
        let mut tk = Ticker::every(Duration::from_millis(conf.balance_interval as u64));
        for step in conf.bump as u16..max_dac {
            dac.set(Value::Bit12Left(step));
            tk.next().await;
            if pc.is_low() {
                break;
            }
        }
        let mut mean = dac.read();
        info!("approx1={}", mean);
        let mut approx = async |num: u32, delta: u16, time_init: u64, time: u64| {
            let low = mean - delta;
            let up = mean + delta;
            dac.set(Value::Bit12Left(low));
            Timer::after_millis(time_init).await;
            for step in low..up {
                dac.set(Value::Bit12Left(step));
                Timer::after_millis(time).await;
                if pc.is_low() {
                    mean = dac.read();
                    info!("approx{}={}", num, mean);
                    break;
                }
            }
        };
        approx(2, 10, 300, 100).await;
        approx(3, 3, 500, 800).await;
        BALANCE_MEAN.store(mean, SeqCst);

        if conf.probe_expand > 0 {
            // this is used for verification
            let mut vec = [false; BALANCE_BYTES * 8];
            let half = vec.len() / 2;
            let mut tk = Ticker::every(Duration::from_millis(conf.balance_verify as u64));
            info!(
                "balance, mean {}, range {}",
                mean,
                conf.probe_expand as usize * half
            );

            for di in (0usize..half as usize).into_iter().rev() {
                let mut fill = async |val: &mut bool, dac_val: u16| {
                    dac.set(Value::Bit12Left(dac_val));
                    tk.next().await;
                    *val = pc.is_high();
                };
                let di2 = di as u16 * conf.probe_expand as u16;
                (fill)(&mut vec[half as usize - 1 - di], mean - di2).await;
                (fill)(&mut vec[half as usize + di], mean + di2).await;
            }
            for (ix, val) in vec.into_iter().enumerate() {
                unsafe {
                    if !val {
                        BALANCE.clear_bit(ix as u32)
                    } else {
                        BALANCE.set_bit(ix as u32)
                    }
                };
            }
            BALANCE_DATA.store(true, SeqCst);
        }
        info!("weighing finished, {}", dac.read());
    }
}

static mut DAC_VAL: u16 = 0;
static mut BALANCE: bittle::Set<[u8; BALANCE_BYTES]> = bittle::Set::ONES;
static BALANCE_DATA: AtomicBool = AtomicBool::new(false);
static BALANCE_MEAN: AtomicU16 = AtomicU16::new(0);

#[embassy_executor::task]
async fn hall_digital(pa1: PA1, mut prod: Producer<'static, HALL_BUFSIZE>) {
    let p = gpio::Input::new(pa1, Pull::Up);
    unsafe {
        let mut usb = USB_SIGNAL.subscriber().unwrap();
        loop {
            if USB_STATE == UsbState::Connected
                || usb.next_message_pure().await == UsbState::Connected
            {
                break;
            }
        }
    }
    let mut confn = CONF_NOTIF.subscriber().unwrap();
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
        let mut usb = USB_SIGNAL.subscriber().unwrap();
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
        let rd = rx.read_packet(&mut buf[rg.take().unwrap()]).await?;
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
                    }
                    G4Command::Weigh => {
                        WEIGH_NOTIF.send(Notif).await;
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

const HALL_BUFSIZE: usize = 1024 * 4;
static HALL_DATA: BBBuffer<HALL_BUFSIZE> = BBBuffer::new();

async fn report<'d, T: 'd + embassy_stm32::usb::Instance>(
    class: &mut cdc_acm::Sender<'d, Driver<'d, T>>,
    hall_con: &mut Consumer<'static, HALL_BUFSIZE>,
) -> Result<(), Disconnected> {
    info!("reporting enabled");
    unsafe {
        let mut usb = USB_SIGNAL.subscriber().unwrap();
        loop {
            if USB_STATE == UsbState::Connected
                || usb.next_message_pure().await == UsbState::Connected
            {
                break;
            }
        }
    }
    let mut tkreload = Ticker::every(Duration::from_secs(1));
    loop {
        let e = embassy_futures::select::select(tkreload.next(), async {
            // debug!("reloading conf");
            let conf: G4Settings = CONF.load(SeqCst);

            let mut tkrp = Ticker::every(Duration::from_micros(conf.min_report_interval));
            loop {
                let hall: Option<Vec<u8, HALL_BYTES>>;
                if let Ok(rd) = hall_con.read() {
                    let copy = &rd[..min(rd.len(), HALL_BYTES)];
                    hall = Some(heapless::Vec::from_slice(copy).unwrap());
                    rd.release(hall.as_ref().unwrap().len());
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
                    balance: if BALANCE_DATA.load(SeqCst) {
                        BALANCE_DATA.store(false, SeqCst);
                        Vec::from_slice(&unsafe { BALANCE.into_bits() }).unwrap()
                    } else {
                        Vec::new()
                    },
                    balance_val: bval,
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
