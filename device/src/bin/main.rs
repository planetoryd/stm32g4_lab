#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]
#![feature(iter_next_chunk)]
#![feature(iter_array_chunks)]

use core::{
    future::{pending, Pending},
    ops::{Range, RangeFrom}, sync::atomic,
};

use ::atomic::Atomic;
use common::{
    cob::{
        self,
        COBErr::{Codec, NextRead},
        COBSeek, COB,
    }, G4Command, G4Message, G4Settings, HALL_BYTES, MAX_PACKET_SIZE
};
use defmt::*;
use embassy_futures::join;
use embassy_sync::{
    blocking_mutex::{raw::CriticalSectionRawMutex, CriticalSectionMutex},
    channel,
    pubsub::PubSubChannel,
    signal::Signal,
};
use embassy_usb::{
    class::cdc_acm::{self, CdcAcmClass},
    driver::EndpointError,
    UsbDevice,
};

use futures_util::Stream;
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
    dma::WritableRingBuffer,
    exti::ExtiInput,
    gpio::{self, Level, Output, Pull, Speed},
    opamp::{self, *},
    peripherals::{OPAMP1, PA1, PB10, TIM2, USB},
    time::{khz, mhz},
    timer::pwm_input::PwmInput,
    usb::Driver,
    Config,
};
use embassy_stm32::{peripherals, usb};
use embassy_time::{Delay, Duration, Timer};

bind_interrupts!(
    struct Irqs {
        USB_LP => usb::InterruptHandler<peripherals::USB>;
    }
);

type HallData = u8;
static HALL_SPEED: channel::Channel<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    HallData,
    128,
> = channel::Channel::new();

static HALL_INTERVAL: u64 = 500;

static CONF: Atomic<G4Settings> = Atomic::new(G4Settings::new());

// todo: hall effect sensor speed meter
// electromagnetic microbalance

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
    // let mut vrefint = adc.enable_vrefint();
    // let mut temp = adc.enable_temperature();
    unwrap!(spawner.spawn(led(Output::new(p.PC13, Level::High, Speed::Low))));

    let driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);
    let mut config = embassy_usb::Config::new(0xc0de, 0xcafe);
    config.manufacturer = Some("Plein");
    config.product = Some("Lab device G4");
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

    let class = embassy_usb::class::cdc_acm::CdcAcmClass::new(&mut builder, &mut state, 64);
    let (mut sx, mut rx) = class.split();
    info!("usb max packet size, {}", sx.max_packet_size());
    let mut usb = builder.build();
    let usb_fut = usb.run();

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
            let x = join::join(report(&mut sx), listen(&mut rx)).await;
            info!("Disconnected, {:?}", x);
            unsafe {
                USB_STATE = UsbState::Waiting;
                sig.publish(USB_STATE).await;
            }
        }
    };

    unwrap!(spawner.spawn(hall_digital(p.PA1)));
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
    let (sx, rx) = (HALL_SPEED.sender(), HALL_SPEED.receiver());
    loop {
        let va = adc.blocking_read(&mut opi);
        // let _ = sx.try_send(va);
        Timer::after_millis(HALL_INTERVAL).await;
    }
}

#[embassy_executor::task]
async fn hall_digital(pa1: PA1) {
    const MICROS: usize = 2000;
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
    info!("hall sensor, sample interval={}us", MICROS);
    loop {
        let mut byte = 0u8;
        for pos in 0..8 {
            Timer::after_micros(MICROS as u64).await;
            let le = p.is_high() as u8;
            byte |= le << pos;
        }
        let _ = HALL_SPEED.sender().try_send(byte);
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
        info!("read into {}", (&mut buf[rg.as_ref().unwrap().clone()]).len());
        let rd = rx.read_packet(&mut buf[rg.take().unwrap()]).await?;
        debug!("serial read len = {}", rd);
        if rd == 0 {
            Timer::after_secs(2).await;
            continue;
        }
        let copy = buf.clone();
        let cob: COB<G4Command> = COB::new(&mut buf[..], &copy);
        for msg in cob {
            if let Ok(msg) = msg {
                info!("{:?}", msg);
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

async fn report<'d, T: 'd + embassy_stm32::usb::Instance>(
    class: &mut cdc_acm::Sender<'d, Driver<'d, T>>,
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
    loop {
        let mut hall = Vec::new();
        for _ in 0..HALL_BYTES {
            unwrap!(hall.push(HALL_SPEED.receive().await))
        }
        let reply = G4Message { hall };
        let rx: Result<heapless::Vec<u8, 1024>, postcard::Error> = postcard::to_vec_cobs(&reply);
        if let Ok(coded) = rx {
            debug!("send upstream {}", coded.len());
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
    }
    Ok(())
}
