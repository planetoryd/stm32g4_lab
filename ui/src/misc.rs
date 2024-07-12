use std::iter::repeat;

pub fn fill_vec_p2<T: Default + Clone>(ve: &mut Vec<T>) {
    let base = ve.len();
    let mut sol = 1;
    for m in 0..32 {
        sol = 1 << m;
        if sol > base {
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
fn freqs() {
    // 1 RPM to 100K RPM
    dbg!(1f32 / 60f32);
    dbg!(100 * 1000 / 60);
}
