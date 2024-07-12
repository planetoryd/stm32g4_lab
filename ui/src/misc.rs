use std::iter::repeat;

use spectrum_analyzer::{samples_fft_to_spectrum, windows::hann_window, FrequencyLimit};

pub fn fill_vec_p2<T: Default + Clone>(ve: &mut Vec<T>) {
    let base = ve.len();
    let mut sol = 1;
    for m in 0..32 {
        sol = 1 << m;
        if sol >= base {
            break;
        }
    }
    let fill = sol - base;
    ve.extend(repeat(T::default()).take(fill));
}

#[test]
fn testvec2() {
    let mut v1: Vec<i32> = repeat(2).take(32760).collect();
    fill_vec_p2(&mut v1);
    dbg!(v1.len());
}

#[test]
fn testvec22() {
    let mut v1: Vec<i32> = repeat(2).take(16384).collect();
    fill_vec_p2(&mut v1);
    dbg!(v1.len());
}

#[test]
fn freqs() {
    // 1 RPM to 100K RPM
    dbg!(1f32 / 60f32);
    dbg!(100 * 1000 / 60);
}

#[test]
fn freqanalyze() {
    let waveform = [0, 0, 0, 0, 1, 1, 1, 0]
        .into_iter()
        .map(|x| x as f32);
    let track: Vec<_> = repeat(waveform).take(32).flatten().collect();
    let hw = hann_window(&track);
    let spec = samples_fft_to_spectrum(&hw[..], 8, FrequencyLimit::Min(0.2), None);
    let rx = spec.unwrap();

    dbg!(rx.max());    
}
