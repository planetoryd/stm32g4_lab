#![no_std]
#![no_main]

use core::future::{pending, Pending};

use lab_stm32g4::fmt;

#[cfg(not(feature = "defmt"))]
use panic_halt as _;
#[cfg(feature = "defmt")]
use {defmt_rtt as _, panic_probe as _};

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::{
    adc::{self, Adc},
    bind_interrupts,
    gpio::{Level, Output, Speed},
    opamp::{self, *},
    peripherals::PB10,
    time::mhz,
    Config,
};
use embassy_time::{Delay, Duration, Timer};
use fmt::{debug, info};

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
            divq: None,
            // Main system clock at 170 MHz
            divr: Some(PllRDiv::DIV2),
        });
        config.rcc.mux.adc12sel = mux::Adcsel::SYS;
        config.rcc.sys = Sysclk::PLL1_R;
    }
    let mut p = embassy_stm32::init(config);

    info!("init opmap for hall effect sensor");
    let mut op1 = OpAmp::new(p.OPAMP1, OpAmpSpeed::Normal);
    let mut adc = Adc::new(p.ADC1);
    let mut opi = op1.buffer_int(&mut p.PA1, OpAmpGain::Mul16);

    unwrap!(spawner.spawn(led(Output::new(p.PC13, Level::Low, Speed::Low))));

    pending::<()>().await;

    // loop {
    //     info!("PA1={}", adc.read(&mut opi));
    //     Timer::after_millis(500).await;
    // }
}

#[embassy_executor::task]
async fn led(mut out: Output<'static>) {
    loop {
        out.toggle();
        Timer::after_millis(500).await;
    }
}
